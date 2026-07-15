//! The Legion knowledge wiki: corpus loader/cache and the `wiki_index` /
//! `wiki_read` / `wiki_search` tools.
//!
//! The corpus ships INSIDE the binary: `build.rs` embeds every page under the
//! repo's `wiki/` directory into [`WIKI_EMBEDDED`], so the wiki tools work in
//! every AI build — including prebuilt release binaries — with nothing to
//! locate at run time. An EMPTY `wiki_root` selects the embedded corpus; a
//! non-empty root reads that directory from disk instead (the `--wiki`
//! override, for corpus development without a rebuild).

use super::source::SKIP_DIRS;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

// (rel_path, content) for every `wiki/**/*.md`, sorted by path. Generated.
include!(concat!(env!("OUT_DIR"), "/wiki_embedded.rs"));

/// Default per-read character budget for `wiki_read`. Chosen from the corpus size
/// distribution (median ~5.7 KB, p90 ~7.2 KB, max ~40 KB): `12_000` chars (~3K
/// tokens) returns the vast majority of pages whole and caps only the handful of
/// outliers (the application pages + the auto-generated meta lint report).
const WIKI_READ_DEFAULT_MAX_CHARS: usize = 12_000;

/// One wiki page's metadata, parsed once and cached. `path` is relative to the
/// wiki root, forward-slashed (e.g. `concepts/mapper.md`).
#[derive(Clone)]
struct WikiPage {
    /// Relative, forward-slashed path under the wiki root.
    path: String,
    /// Top-level section directory (concepts|pitfalls|workflows|glossary|meta|applications|…).
    section: String,
    /// Frontmatter `title` (falls back to a title-cased filename).
    title: String,
    /// Frontmatter `summary` — the universal one-line TL;DR (falls back to `title`).
    summary: String,
    /// Frontmatter `tags` (inline `[a, b]` array; empty when absent).
    tags: Vec<String>,
    /// The `## TL;DR` block body, lower-cased, for search matching only (empty when
    /// the page has none — pitfalls/workflows/glossary/meta).
    tldr_lc: String,
    /// Coarse importance tier: `core` for concepts/pitfalls/workflows/applications,
    /// `optional` for glossary/meta.
    tier: &'static str,
}

/// Per-root corpus cache: the metadata walk runs once per wiki root, then every
/// `wiki_*` call reuses it. Page CONTENT is never cached (read fresh on demand).
static WIKI_CORPUS_CACHE: OnceLock<Mutex<HashMap<String, Arc<Vec<WikiPage>>>>> = OnceLock::new();

/// Coarse tier heuristic: glossary/meta are reference/bookkeeping (`optional`);
/// everything else is `core`.
fn wiki_tier(section: &str) -> &'static str {
    match section {
        "glossary" | "meta" => "optional",
        _ => "core",
    }
}

/// Title-case a `kebab-or_snake.md` filename into a readable fallback title.
fn filename_to_title(name: &str) -> String {
    let stem = name.strip_suffix(".md").unwrap_or(name);
    stem.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extract `(title, summary, tags)` from a page's leading YAML frontmatter (the
/// block between the first `---` and the next `---`). Only the flat single-line
/// fields we care about are parsed; pages without frontmatter (the meta pages)
/// return all-empty and the callers fall back to a filename-derived title.
fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>, Vec<String>) {
    let after = match content.strip_prefix("---") {
        Some(rest) => rest,
        None => return (None, None, Vec::new()),
    };
    let end = match after.find("\n---") {
        Some(i) => i,
        None => return (None, None, Vec::new()),
    };
    let (mut title, mut summary, mut tags) = (None, None, Vec::new());
    for line in after[..end].lines() {
        let line = line.trim_end();
        if let Some(v) = line.strip_prefix("title:") {
            title = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("summary:") {
            summary = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("tags:") {
            let inner = v.trim().trim_start_matches('[').trim_end_matches(']');
            tags = inner
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    (title, summary, tags)
}

/// A markdown fenced-code-block delimiter (``` or ~~~, optionally indented, with
/// an optional info string). Used so a `# shell-comment` line inside a code fence
/// is not mistaken for a heading.
fn is_code_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Extract a single `## <header>` block (header + body up to the next level-1/2
/// heading or EOF). Case-insensitive on the header text. `None` if not found.
///
/// Fenced code blocks are tracked so a `# ...` / `## ...` comment line INSIDE a
/// ```` ```bash ```` block is not mistaken for a heading (which would silently
/// truncate the section). `###`+ subheadings stay inside the block.
fn extract_section(content: &str, header: &str) -> Option<String> {
    let want = header.trim().trim_start_matches('#').trim().to_lowercase();
    let lines: Vec<&str> = content.lines().collect();

    // Locate the start header (outside any code fence).
    let mut in_fence = false;
    let mut start = None;
    for (i, l) in lines.iter().enumerate() {
        if is_code_fence(l) {
            in_fence = !in_fence;
        } else if !in_fence
            && l.strip_prefix("## ")
                .is_some_and(|rest| rest.trim().to_lowercase() == want)
        {
            start = Some(i);
            break;
        }
    }
    let start = start?;

    // Scan to the next level-1/2 heading, ignoring heading-like lines inside fences.
    let mut in_fence = false;
    let mut end = lines.len();
    for (j, l) in lines.iter().enumerate().skip(start + 1) {
        if is_code_fence(l) {
            in_fence = !in_fence;
        } else if !in_fence && (l.starts_with("## ") || l.starts_with("# ")) {
            end = j;
            break;
        }
    }
    Some(lines[start..end].join("\n"))
}

/// Cap `text` to `max_chars` characters, appending a machine-readable truncation
/// marker (mirrors the `run_query` 50-row marker) with a `next_offset` so a caller
/// can tell a clipped read from a whole one and knows how to get the rest.
fn wiki_cap_with_marker(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_owned();
    }
    let shown: String = text.chars().take(max_chars).collect();
    format!(
        "{shown}\n\n[TRUNCATED] shown first {max_chars} of {total} chars \
         (next_offset={max_chars}). To see more: re-call wiki_read with a larger \
         max_chars, or pass `section` to read just one `## Header` block.",
    )
}

/// Parse one page's metadata from its relative path + content (shared by the
/// filesystem walk and the embedded corpus).
fn page_from_content(rel: &str, content: &str) -> WikiPage {
    let name = rel.rsplit('/').next().unwrap_or(rel).to_owned();
    let section = rel.split('/').next().unwrap_or("").to_owned();
    let (title, summary, tags) = parse_frontmatter(content);
    let title = title
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| filename_to_title(&name));
    let summary = summary
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| title.clone());
    let tldr_lc = extract_section(content, "TL;DR")
        .unwrap_or_default()
        .to_lowercase();
    let tier = wiki_tier(&section);
    WikiPage {
        path: rel.to_owned(),
        section,
        title,
        summary,
        tags,
        tldr_lc,
        tier,
    }
}

/// Walk `dir` recursively, collecting every `.md` page's metadata into `out`.
fn collect_wiki_pages(root: &Path, dir: &Path, out: &mut Vec<WikiPage>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                collect_wiki_pages(root, &path, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            out.push(page_from_content(&rel, &content));
        }
    }
}

/// Build (and memoize) the page-metadata corpus for `wiki_root`. An EMPTY root
/// selects the embedded corpus (the shipped default); a non-empty root walks
/// that directory instead. Err if a directory root is missing or has no pages.
fn wiki_corpus(wiki_root: &str) -> Result<Arc<Vec<WikiPage>>, String> {
    let cache = WIKI_CORPUS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(c) = cache.lock().unwrap().get(wiki_root) {
        return Ok(c.clone());
    }
    let mut pages = Vec::new();
    if wiki_root.is_empty() {
        for (rel, content) in WIKI_EMBEDDED {
            pages.push(page_from_content(rel, content));
        }
        if pages.is_empty() {
            // Only possible if the crate was built without a `wiki/` directory.
            return Err("Embedded wiki corpus is empty (crate built without wiki/).".into());
        }
    } else {
        let root = Path::new(wiki_root);
        if !root.is_dir() {
            return Err(format!("Wiki root '{wiki_root}' is not a directory."));
        }
        collect_wiki_pages(root, root, &mut pages);
        if pages.is_empty() {
            return Err(format!("No .md pages found under wiki root '{wiki_root}'."));
        }
    }
    pages.sort_by(|a, b| a.path.cmp(&b.path));
    let arc = Arc::new(pages);
    cache
        .lock()
        .unwrap()
        .insert(wiki_root.to_owned(), arc.clone());
    Ok(arc)
}

/// `wiki_index`: a grouped, one-line-per-page table of the whole corpus (or one
/// section). Each row is `path [tier] — summary`. Scan it, then `wiki_read` the
/// pages you need.
/// # Errors
/// Returns `Err` if the wiki corpus under `wiki_root` cannot be loaded.
pub fn wiki_index(wiki_root: &str, section: Option<&str>) -> Result<String, String> {
    let corpus = wiki_corpus(wiki_root)?;

    let mut sections: Vec<&str> = corpus.iter().map(|p| p.section.as_str()).collect();
    sections.sort_unstable();
    sections.dedup();

    if let Some(sec) = section {
        if !sections.contains(&sec) {
            return Err(format!(
                "Unknown wiki section '{sec}'. Sections: {}.",
                sections.join(", ")
            ));
        }
    }

    let mut out = String::with_capacity(40 * 1024);
    out.push_str("# Legion Wiki Index\n");
    out.push_str(
        "Consumption loop: scan these one-line summaries → `wiki_read` the relevant page(s) → \
         follow their `Related` links. Use `wiki_search` if unsure which page.\n\n",
    );
    for sec in &sections {
        if section.is_some_and(|want| want != *sec) {
            continue;
        }
        let pages: Vec<&WikiPage> = corpus.iter().filter(|p| &p.section == sec).collect();
        out.push_str(&format!("## {} ({} pages)\n", sec, pages.len()));
        for p in pages {
            out.push_str(&format!("- `{}` [{}] — {}\n", p.path, p.tier, p.summary));
        }
        out.push('\n');
    }
    Ok(out)
}

/// `wiki_read`: read a page verbatim (Related links intact), or just one `## Header`
/// block when `section` is given. Capped at `max_chars` (default
/// [`WIKI_READ_DEFAULT_MAX_CHARS`]) with a truncation marker when cut. Path-safe:
/// rejects traversal/absolute paths AND requires the path to be an enumerated page.
/// # Errors
/// Returns `Err` if the path escapes the wiki root, names a page that does
/// not exist, or the file cannot be read.
pub fn wiki_read(
    wiki_root: &str,
    path: &str,
    section: Option<&str>,
    max_chars: Option<usize>,
) -> Result<String, String> {
    // Path safety (mirrors execute_read_code): no traversal, no absolute prefix.
    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        return Err("Invalid wiki path: must be relative with no '..' or absolute prefix.".into());
    }
    let corpus = wiki_corpus(wiki_root)?;
    let norm = path.replace('\\', "/");
    // Second guard: the path must be one of the enumerated pages under the root.
    if !corpus.iter().any(|p| p.path == norm) {
        return Err(format!(
            "Unknown wiki page '{path}'. Use wiki_index or wiki_search to find valid paths."
        ));
    }
    let content = if wiki_root.is_empty() {
        // Embedded corpus: the table is sorted by path (build.rs), so look up
        // by binary search.
        WIKI_EMBEDDED
            .binary_search_by(|(p, _)| p.cmp(&norm.as_str()))
            .map(|i| WIKI_EMBEDDED[i].1.to_owned())
            .map_err(|_| format!("Embedded wiki page '{norm}' missing from the table."))?
    } else {
        let full = Path::new(wiki_root).join(&norm);
        std::fs::read_to_string(&full)
            .map_err(|e| format!("Cannot read '{}': {}", full.display(), e))?
    };

    let body = match section {
        Some(header) => extract_section(&content, header).ok_or_else(|| {
            format!("Section '## {header}' not found in '{norm}'. Read the page with no `section` to see its headers.")
        })?,
        None => content,
    };

    let max = max_chars.unwrap_or(WIKI_READ_DEFAULT_MAX_CHARS).max(1);
    Ok(wiki_cap_with_marker(&body, max))
}

/// Weighted case-insensitive substring score for a page against a lower-cased
/// query. Title > tags > summary > path; per-token hits in title/summary add a
/// little more. Substring-only (v1); BM25/embeddings can replace this later.
fn wiki_score(p: &WikiPage, q: &str) -> i64 {
    let title = p.title.to_lowercase();
    let summary = p.summary.to_lowercase();
    let path = p.path.to_lowercase();
    let mut score = 0i64;
    if title.contains(q) {
        score += 10;
    }
    if p.tags.iter().any(|t| t.to_lowercase().contains(q)) {
        score += 5;
    }
    if summary.contains(q) {
        score += 3;
    }
    if p.tldr_lc.contains(q) {
        score += 2;
    }
    if path.contains(q) {
        score += 2;
    }
    for tok in q.split_whitespace().filter(|t| t.len() >= 2) {
        if title.contains(tok) {
            score += 2;
        }
        if summary.contains(tok) || p.tldr_lc.contains(tok) {
            score += 1;
        }
    }
    score
}

/// `wiki_search`: rank pages by keyword/substring over titles, summaries, TL;DRs,
/// and tags (optionally scoped to a section and/or a tag). Returns up to `limit`
/// `{path, section, tldr, tags, score}` rows as JSON — PATHS to read, not prose.
/// # Errors
/// Returns `Err` if the wiki corpus cannot be loaded.
pub fn wiki_search(
    wiki_root: &str,
    query: &str,
    section: Option<&str>,
    tag: Option<&str>,
    limit: usize,
) -> Result<String, String> {
    let corpus = wiki_corpus(wiki_root)?;
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Err("wiki_search requires a non-empty query.".into());
    }
    let tag_lc = tag.map(str::to_lowercase);

    let mut scored: Vec<(i64, &WikiPage)> = corpus
        .iter()
        .filter(|p| section.is_none_or(|s| p.section == s))
        .filter(|p| {
            tag_lc
                .as_deref()
                .is_none_or(|t| p.tags.iter().any(|pt| pt.to_lowercase() == t))
        })
        .filter_map(|p| {
            let s = wiki_score(p, &q);
            (s > 0).then_some((s, p))
        })
        .collect();
    // Rank: score desc, then path asc for a deterministic order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
    scored.truncate(limit.max(1));

    if scored.is_empty() {
        let scope = section
            .map(|s| format!(" in section '{s}'"))
            .unwrap_or_default();
        return Ok(format!(
            "No wiki pages matched query '{query}'{scope}. Try wiki_index to browse, or broader terms."
        ));
    }
    let arr: Vec<serde_json::Value> = scored
        .iter()
        .map(|(s, p)| {
            serde_json::json!({
                "path": p.path,
                "section": p.section,
                "tldr": p.summary,
                "tags": p.tags,
                "score": s,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_owned()))
}

/// Wiki-tool tests. NOT gated on `duckdb` — the `wiki_*` tools are pure file/string
/// helpers and must work under `{ai}` alone. The corpus is the in-repo `wiki/`
/// directory (committed, always present), so nothing soft-skips; the `Option`
/// shape of `wiki_root()` is kept only to leave the guard sites untouched.
#[cfg(test)]
mod wiki_tests {
    use super::*;

    /// The Legion wiki root: the crate's own committed `wiki/` corpus.
    fn wiki_root() -> Option<String> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wiki");
        assert!(p.is_dir(), "in-repo wiki/ corpus missing");
        Some(p.to_string_lossy().into_owned())
    }

    /// The embedded corpus (empty root) must load and match the on-disk corpus
    /// it was built from: same page set, same index rows.
    #[test]
    fn test_wiki_embedded_matches_fs_corpus() {
        let fs_root = wiki_root().unwrap();
        let embedded_idx = wiki_index("", None).unwrap();
        let fs_idx = wiki_index(&fs_root, None).unwrap();
        assert_eq!(
            embedded_idx, fs_idx,
            "embedded corpus diverges from the committed wiki/ tree"
        );
    }

    /// An embedded read (empty root) returns the same bytes as the fs read.
    #[test]
    fn test_wiki_embedded_read_matches_fs_read() {
        let fs_root = wiki_root().unwrap();
        let page = concept_with_debug_signals(&fs_root).expect("a concept page exists");
        let emb = wiki_read("", &page, None, Some(usize::MAX)).unwrap();
        let fs = wiki_read(&fs_root, &page, None, Some(usize::MAX)).unwrap();
        assert_eq!(emb, fs, "embedded page content diverges: {page}");
    }

    /// Path safety holds for the embedded corpus too.
    #[test]
    fn test_wiki_embedded_read_path_safety() {
        assert!(wiki_read("", "../Cargo.toml", None, None).is_err());
        assert!(wiki_read("", "/etc/hosts", None, None).is_err());
        assert!(wiki_read("", "concepts/does-not-exist.md", None, None).is_err());
    }

    /// First listed concept page that has BOTH a `## TL;DR` and a `## Debug signals`
    /// section — picked from the live index so the tests don't hard-code a filename.
    fn concept_with_debug_signals(root: &str) -> Option<String> {
        let idx = wiki_index(root, Some("concepts")).ok()?;
        for line in idx.lines() {
            if let Some(rest) = line.strip_prefix("- `") {
                if let Some(end) = rest.find('`') {
                    let path = &rest[..end];
                    if let Ok(content) = wiki_read(root, path, None, Some(usize::MAX)) {
                        if content.contains("## TL;DR") && content.contains("## Debug signals") {
                            return Some(path.to_owned());
                        }
                    }
                }
            }
        }
        None
    }

    #[test]
    fn test_wiki_index_full_lists_sections_and_pages() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        let idx = wiki_index(&root, None).unwrap();

        // Sections present as grouped headers.
        for sec in ["concepts", "pitfalls", "workflows", "glossary"] {
            assert!(
                idx.contains(&format!("## {sec} (")),
                "index missing section {sec}"
            );
        }
        // Known page paths appear as rows.
        assert!(idx.contains("concepts/"), "no concept rows");
        assert!(idx.contains("pitfalls/"), "no pitfall rows");
        // Every row carries a tier + a non-empty one-liner after the em dash.
        let rows: Vec<&str> = idx.lines().filter(|l| l.starts_with("- `")).collect();
        assert!(
            rows.len() >= 100,
            "expected >=100 pages, got {}",
            rows.len()
        );
        for r in &rows {
            assert!(
                r.contains("[core]") || r.contains("[optional]"),
                "row lacks tier: {r}"
            );
            let oneliner = r.split(" — ").nth(1).unwrap_or("").trim();
            assert!(!oneliner.is_empty(), "row lacks a one-liner: {r}");
        }

        // MEASURE + REPORT the full no-arg index size (visible with --nocapture).
        let chars = idx.chars().count();
        eprintln!(
            "[wiki_index] full index: {} pages, {} chars (~{} tokens)",
            rows.len(),
            chars,
            chars / 4
        );
        // It must fit comfortably within one context window (≪ 200K tokens), so the
        // full form is kept (no compact fallback needed).
        assert!(chars < 200_000, "index unexpectedly huge: {chars} chars");
    }

    #[test]
    fn test_wiki_index_section_scopes() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        let idx = wiki_index(&root, Some("pitfalls")).unwrap();
        assert!(idx.contains("## pitfalls ("), "pitfalls header missing");
        // No other section headers leak in.
        assert!(
            !idx.contains("## concepts ("),
            "concepts leaked into pitfalls scope"
        );
        // Every listed row is under pitfalls/.
        for r in idx.lines().filter(|l| l.starts_with("- `")) {
            assert!(
                r.contains("`pitfalls/"),
                "non-pitfall row in scoped index: {r}"
            );
        }
        // An unknown section is a clear error.
        assert!(wiki_index(&root, Some("nope")).is_err());
    }

    #[test]
    fn test_wiki_read_returns_tldr() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        let Some(page) = concept_with_debug_signals(&root) else {
            eprintln!("skipping: no concept page with TL;DR + Debug signals");
            return;
        };
        let content = wiki_read(&root, &page, None, None).unwrap();
        assert!(
            content.contains("## TL;DR"),
            "page read lacks TL;DR: {page}"
        );
        // Related links are preserved verbatim (no synthesis).
        assert!(
            content.contains("## Related"),
            "Related section stripped: {page}"
        );
    }

    #[test]
    fn test_wiki_read_section_extracts_only_that_block() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        let Some(page) = concept_with_debug_signals(&root) else {
            eprintln!("skipping: no concept page with TL;DR + Debug signals");
            return;
        };
        let block = wiki_read(&root, &page, Some("Debug signals"), None).unwrap();
        assert!(
            block.starts_with("## Debug signals"),
            "block not headed correctly: {block:.40}"
        );
        // Only that block — other level-2 headers must not bleed in.
        assert!(
            !block.contains("## TL;DR"),
            "TL;DR leaked into Debug-signals block"
        );
        assert!(
            !block.contains("## Related"),
            "Related leaked into Debug-signals block"
        );
        // A missing section is an explicit error.
        assert!(wiki_read(&root, &page, Some("No Such Header"), None).is_err());
    }

    /// Regression: a `# shell-comment` line inside a ```` ```bash ```` fence must
    /// NOT prematurely terminate the section (it is not a heading). Pure unit test
    /// on extract_section — no wiki root needed.
    #[test]
    fn test_extract_section_ignores_comments_in_code_fence() {
        let page = "\
## Typical combinations
Prose before the fence.

```bash
./app -ll:cpu 4
# In a separate shell:
# gdb -p <PID>
./app -lg:inorder 1
```

Trailing prose after the fence.

## Invariants
- next section, must NOT appear in the block.
";
        let block = extract_section(page, "Typical combinations").unwrap();
        // The fenced comment lines and everything up to the NEXT real heading are kept.
        assert!(
            block.contains("# In a separate shell:"),
            "fenced comment truncated the section"
        );
        assert!(block.contains("gdb -p <PID>"), "second fenced comment lost");
        assert!(
            block.contains("Trailing prose after the fence."),
            "post-fence prose lost"
        );
        // The genuine next heading still bounds the block.
        assert!(!block.contains("## Invariants"), "next section leaked in");
        assert!(
            !block.contains("next section, must NOT appear"),
            "next section body leaked in"
        );

        // Case-insensitive header match still works.
        assert!(extract_section(page, "typical COMBINATIONS").is_some());
        // A `## ` that only appears inside a fence is not matched as a start header.
        let fenced_only = "## Real\nbody\n```\n## not a heading\n```\n";
        assert!(extract_section(fenced_only, "not a heading").is_none());
    }

    #[test]
    fn test_wiki_read_truncation_marker() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        let Some(page) = concept_with_debug_signals(&root) else {
            eprintln!("skipping: no concept page");
            return;
        };
        let tiny = wiki_read(&root, &page, None, Some(100)).unwrap();
        assert!(
            tiny.contains("[TRUNCATED]"),
            "tiny read missing truncation marker"
        );
        assert!(tiny.contains("next_offset="), "marker missing next_offset");
        // A generous cap leaves the page whole (no marker).
        let whole = wiki_read(&root, &page, None, Some(usize::MAX)).unwrap();
        assert!(
            !whole.contains("[TRUNCATED]"),
            "whole read wrongly marked truncated"
        );
    }

    #[test]
    fn test_wiki_search_ranks_matching_page() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        // "mapper" appears in titles/summaries of multiple mapper pages.
        let res = wiki_search(&root, "mapper", None, None, 5).unwrap();
        assert!(
            res.contains("mapper"),
            "search for 'mapper' returned nothing relevant"
        );
        // Result is a JSON array of {path, tldr, section, score} — paths, not prose.
        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        let arr = parsed.as_array().expect("search result is a JSON array");
        assert!(!arr.is_empty(), "no hits for 'mapper'");
        assert!(arr.len() <= 5, "limit not honored");
        let first = &arr[0];
        for k in ["path", "tldr", "section", "score"] {
            assert!(first.get(k).is_some(), "hit missing field {k}: {first}");
        }
        // Section scoping works.
        let scoped = wiki_search(&root, "mapper", Some("pitfalls"), None, 5).unwrap();
        if let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&scoped)
        {
            for hit in a {
                assert_eq!(hit["section"], "pitfalls", "section scope leaked");
            }
        }
    }

    #[test]
    fn test_wiki_read_path_safety_rejects_escape() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        // Traversal / absolute paths are rejected before any read.
        assert!(wiki_read(&root, "../CLAUDE.md", None, None).is_err());
        assert!(wiki_read(&root, "../../etc/hosts", None, None).is_err());
        assert!(wiki_read(&root, "/etc/hosts", None, None).is_err());
        // A path that escapes via a non-".." but is simply not an enumerated page.
        assert!(wiki_read(&root, "concepts/does-not-exist.md", None, None).is_err());
    }
}
