//! Tool implementations for Legion Prof AI analysis.
//!
//! Plain Rust functions called directly by the built-in agent (zero overhead).
//! No MCP protocol layer — external client support can be added later as a
//! thin wrapper around these same functions.
//!
//! The `run_query` and `gather_overview` tools require the `duckdb` feature.
//! The `read_code` tool requires only the `ai` feature.

use std::path::Path;

// ── File discovery constants ─────────────────────────────────────────────────

/// Source extensions included in file listings and tree views.
const SOURCE_EXTS: &[&str] = &[
    "cc", "cpp", "c", "h", "hpp", "cu", "cuh", "py", "rs", "rg",
    "mk", "cmake", "toml", "json", "yaml", "yml", "txt", "md",
];

/// Directories to skip when walking the source tree.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "__pycache__",
    ".cache",
    ".vscode",
    ".idea",
];

// ── File tree helpers ────────────────────────────────────────────────────────

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

/// Recursively walk a directory tree, appending an indented listing to `output`.
///
/// Caps at `max_depth` levels and `max_files` total entries to prevent
/// runaway scanning on large repositories.
fn walk_dir_tree(
    dir: &Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    output: &mut String,
    file_count: &mut usize,
    max_files: usize,
) {
    if depth > max_depth || *file_count >= max_files {
        return;
    }

    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(_) => return,
    };

    // Sort: directories first, then files, both alphabetical
    entries.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.file_name().cmp(&b.file_name()),
        }
    });

    let indent = "  ".repeat(depth);

    for entry in entries {
        if *file_count >= max_files {
            output.push_str(&format!("{indent}  ... (truncated at {max_files} entries)\n"));
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            output.push_str(&format!("{indent}{prefix}{name}/\n"));
            walk_dir_tree(
                &path,
                "",
                depth + 1,
                max_depth,
                output,
                file_count,
                max_files,
            );
        } else {
            // Only list files with recognised source extensions
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !SOURCE_EXTS.contains(&ext) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            output.push_str(&format!(
                "{indent}{prefix}{name} ({})\n",
                format_size(size)
            ));
            *file_count += 1;
        }
    }
}

/// Build a recursive file tree listing for the given code root directory.
///
/// Returns a formatted string showing directories and source files with sizes,
/// capped at 6 levels deep and 500 files. Used both in scan messages and as
/// the `list_files` tool implementation.
pub fn recursive_file_tree(code_root: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err("Code path not configured. Set it in the Settings panel.".into());
    }

    let root = Path::new(code_root);
    if !root.is_dir() {
        return Err(format!("'{}' is not a directory.", code_root));
    }

    let mut output = format!("Files in `{}`:\n", code_root);
    let mut file_count = 0usize;
    walk_dir_tree(root, "", 0, 6, &mut output, &mut file_count, 500);

    if file_count == 0 {
        output.push_str("  (no source files found)\n");
    }

    Ok(output)
}

/// Execute the `list_files` tool: list source files in a subdirectory of the code root.
///
/// If `path` is empty or `"."`, lists from the code root itself.
pub fn execute_list_files(code_root: &str, path: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err("Code path not configured. Set it in the Settings panel.".into());
    }

    let target = if path.is_empty() || path == "." {
        code_root.to_owned()
    } else {
        if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
            return Err("Invalid path: must be relative with no '..' or absolute prefix.".into());
        }
        Path::new(code_root)
            .join(path)
            .to_string_lossy()
            .to_string()
    };

    recursive_file_tree(&target)
}

// ── Query tools ──────────────────────────────────────────────────────────────

/// Execute a read-only SQL query against the Legion DuckDB database.
///
/// Wraps the user's SQL with DuckDB's `json_group_array(to_json(t))` to serialize
/// all column types (including STRUCTs like Interval and ItemLink) as JSON.
/// Execute a query and return the result as a markdown table (for LLM consumption).
/// Falls back to raw JSON if table formatting fails.
#[cfg(feature = "duckdb")]
pub fn execute_run_query(duckdb_path: &str, sql: &str) -> Result<String, String> {
    let json_result = execute_run_query_raw(duckdb_path, sql)?;
    match json_array_to_markdown_table(&json_result) {
        Some(table) => Ok(table),
        None => Ok(json_result),
    }
}

/// Make the 50-row cap VISIBLE. `execute_run_query_raw` probes one past the cap
/// (`LIMIT 51`); if the wrapped `json_group_array` result parses to an array of
/// MORE than 50 elements, keep the first 50 and append ONE marker object so the
/// agent can tell a truncated result from a full 50-row one. In EVERY other case
/// — parse fails, a scalar/non-array, `len <= 50`, or empty `[]` — the original
/// string is returned UNCHANGED. The `json_group_array(...)`-in-one-row aggregates
/// (`slug_exists`, `gather_overview` sections) are a single row → len 1 → never marked.
#[cfg(feature = "duckdb")]
fn mark_truncation_if_over(result: String) -> String {
    match serde_json::from_str::<Vec<serde_json::Value>>(&result) {
        Ok(mut arr) if arr.len() > 50 => {
            arr.truncate(50);
            arr.push(serde_json::json!({
                "_truncated": true,
                "_shown": 50,
                "_hint": "result capped at 50 rows; refine with aggregation or a narrower filter"
            }));
            serde_json::to_string(&arr).unwrap_or(result)
        }
        _ => result,
    }
}

/// Execute a query and return raw JSON array string.
/// Used internally by gather_overview() which parses the JSON itself.
#[cfg(feature = "duckdb")]
pub fn execute_run_query_raw(duckdb_path: &str, sql: &str) -> Result<String, String> {
    use duckdb::{AccessMode, Config, Connection};

    let sql_trimmed = sql.trim().trim_end_matches(';');

    // Safety: only allow SELECT / WITH queries
    let upper = sql_trimmed.to_ascii_uppercase();
    if !upper.starts_with("SELECT") && !upper.starts_with("WITH") {
        return Err("Only SELECT/WITH queries are allowed.".into());
    }

    // Open read-only with external file access disabled. The SELECT/WITH prefix
    // guard above does NOT block table functions such as read_text()/read_csv()/
    // glob() used in a FROM clause, so an untrusted query could otherwise read
    // arbitrary host files. AccessMode::ReadOnly rejects writes/DDL (defense in
    // depth + clearer errors); enable_external_access(false) is the actual gate
    // that blocks external-file reads.
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|e| format!("config access_mode: {e}"))?
        .enable_external_access(false)
        .map_err(|e| format!("config external_access: {e}"))?;
    let conn = Connection::open_with_flags(duckdb_path, config)
        .map_err(|e| format!("Failed to open DuckDB '{}': {}", duckdb_path, e))?;

    // Strip trailing LIMIT clause to avoid LIMIT-inside-LIMIT syntax errors.
    // The agent's LIMIT is respected up to our hard cap of 50 rows.
    let sql_for_wrap = {
        let upper = sql_trimmed.to_ascii_uppercase();
        if let Some(pos) = upper.rfind("LIMIT") {
            let after_limit = sql_trimmed[pos + 5..].trim();
            if !after_limit.is_empty() && after_limit.chars().all(|c| c.is_ascii_digit()) {
                sql_trimmed[..pos].trim()
            } else {
                sql_trimmed
            }
        } else {
            sql_trimmed
        }
    };

    // Probe one PAST the 50-row cap (LIMIT 51) so a truncated result can be
    // distinguished from a full one — see mark_truncation_if_over.
    let wrapped = format!(
        "SELECT COALESCE(CAST(json_group_array(to_json(t)) AS VARCHAR), '[]') \
         FROM ({sql_for_wrap} LIMIT 51) AS t",
    );

    match conn.query_row(&wrapped, [], |row| row.get::<_, String>(0)) {
        Ok(result) if result == "null" || result.is_empty() => Ok("[]".into()),
        Ok(result) => Ok(mark_truncation_if_over(result)),
        Err(e) => {
            let err_str = e.to_string();
            let mut msg = format!("Query failed: {}\n", err_str);

            // Add contextual hints based on common error patterns
            if err_str.contains("not found") || err_str.contains("Referenced column") {
                msg.push_str(
                    "\nHINT: The `items` table columns are: entry_slug, item_uid, title, \
                     size, plus STRUCT columns (lifetime, running, waiting, deferred, delayed, \
                     ready, scheduling_overhead, triggering_latency, operation, creator, \
                     critical_path, previous_executing, mapper). \
                     Access STRUCT fields with dot notation: running.start, running.duration, \
                     critical_path.item_uid. \
                     The `entries` table columns are: entry_slug, short_name, long_name, \
                     parent_slug, type."
                );
            } else if err_str.contains("Conversion Error") || err_str.contains("Could not convert") {
                msg.push_str(
                    "\nHINT: All timestamp fields are BIGINT nanoseconds. \
                     Use arithmetic: running.duration / 1e6 for milliseconds. \
                     Use CAST() for explicit type conversions."
                );
            } else if err_str.contains("Binder Error") || err_str.contains("No function matches") {
                msg.push_str(
                    "\nHINT: Type mismatch. Column types: entry_slug is TEXT, \
                     item_uid is UBIGINT, title is TEXT, size is UBIGINT (may be NULL). \
                     STRUCT fields (running.duration, etc.) are BIGINT. \
                     You cannot SUM/AVG text columns. Use COUNT(*) for text, \
                     SUM()/AVG() only on numeric columns. \
                     For size: use COALESCE(size, 0) since it may be NULL for non-copy items."
                );
            }

            Err(msg)
        }
    }
}

/// Canonical per-`item_uid` dedup SELECT — the single source of truth for
/// corrected task durations, composed by eval/MCP layers.
///
/// The tile-exported DB stores one row per lifecycle slice, so naive
/// `COUNT(*)`/`SUM` over `items` over-counts massively (e.g.
/// `SUM(lifetime.duration)` inflates a task's wall-clock ~523x on bg4N2). This
/// collapses to one row per `item_uid`:
///   - inner `DISTINCT (item_uid, entry_slug, title, lifetime, running, waiting)`
///     removes exact-dup slice rows and cross-slug copy dups (some copy uids
///     span 2 slugs);
///   - `any_value(lifetime…)` is the true wall-clock span (lifetime is constant
///     per uid); `sum(running/waiting)` are true totals over real slices (never
///     `SUM(DISTINCT …)`, which would collapse equal-duration slices);
///     `max(running)` is the longest single slice.
///
/// Returned WITHOUT a trailing `;` so it composes as a subquery / `CREATE VIEW`
/// body. On the live read-only connection it must be inlined as a CTE
/// (`CREATE VIEW` is rejected read-only); the `CREATE VIEW` form is used only in
/// a separate writable connection (e.g. the regression test).
#[cfg(feature = "duckdb")]
pub fn dedup_select_sql() -> &'static str {
    "WITH slices AS (
    SELECT DISTINCT item_uid, entry_slug, title, lifetime, running, waiting
    FROM items
)
SELECT
    item_uid,
    any_value(title)                              AS title,
    min(entry_slug)                               AS entry_slug,
    count(DISTINCT entry_slug)                    AS n_slugs,
    any_value(lifetime.start)                     AS lifetime_start_ns,
    any_value(lifetime.stop)                      AS lifetime_stop_ns,
    any_value(lifetime.duration)                  AS lifetime_dur_ns,
    round(any_value(lifetime.duration) / 1e6, 4)  AS lifetime_ms,
    round(sum(running.duration) / 1e6, 4)         AS running_ms,
    round(sum(waiting.duration) / 1e6, 4)         AS waiting_ms,
    round(max(running.duration) / 1e6, 4)         AS longest_running_slice_ms
FROM slices
GROUP BY item_uid"
}

/// Cycle-guarded critical-path walk from `start_uid`, enriched with corrected
/// (deduped) durations — the single source of truth for the `find_blockers` tool.
///
/// Walks `critical_path.item_uid` edges from `start_uid` toward the root blocker,
/// carrying a visited-uid `path` array. The `list_contains` guard plus the
/// `cycle` flag are MANDATORY: there is no self-loop, but real 2-cycles exist
/// (e.g. 2220↔1481), and DuckDB's recursive UNION cannot dedup them because the
/// growing `path`/`depth` keeps rows distinct — a depth cap alone would walk
/// 100k+ rows. The depth cap (64) is a secondary backstop.
///
/// The enrichment join reuses the SAME dedup grain as [`dedup_select_sql`]
/// (inner `DISTINCT (item_uid, entry_slug, lifetime, running, waiting)`, outer
/// `GROUP BY item_uid`) so the slice-row inflation cannot re-enter.
///
/// `start_uid` is a `u64` (never model text), so formatting it directly into the
/// two `CAST(... AS UBIGINT)` literals carries no injection surface. Returned
/// WITHOUT a trailing `;` so it composes as a subquery and passes the
/// `SELECT/WITH` prefix guard in [`execute_run_query_raw`].
///
/// Note: routed through `execute_run_query_raw`, results inherit its 50-row cap.
/// Real chains are ≤7 rows (uid 48 → 7; uid 2220 → 2), so this is acceptable; a
/// pathological 51–64-row chain would silently drop its DEEPEST rows.
#[cfg(feature = "duckdb")]
pub fn find_blockers_sql(start_uid: u64) -> String {
    format!(
        "WITH RECURSIVE edges AS (
    SELECT DISTINCT item_uid AS src, critical_path.item_uid AS dst
    FROM items WHERE critical_path.item_uid IS NOT NULL
),
walk AS (
    SELECT CAST({start_uid} AS UBIGINT) AS uid, 0 AS depth,
           [CAST({start_uid} AS UBIGINT)] AS path, FALSE AS cycle
    UNION ALL
    SELECT e.dst, w.depth + 1,
           list_append(w.path, e.dst),
           list_contains(w.path, e.dst)        AS cycle
    FROM walk w
    JOIN edges e ON e.src = w.uid
    WHERE w.depth < 64
      AND NOT w.cycle
      AND NOT list_contains(w.path, e.dst)
)
SELECT w.depth, w.uid, t.title, t.lifetime_ms, t.running_ms, t.waiting_ms
FROM walk w
LEFT JOIN (
    SELECT item_uid, any_value(title) AS title,
           round(any_value(lifetime.duration)/1e6,4) AS lifetime_ms,
           round(sum(running.duration)/1e6,4)        AS running_ms,
           round(sum(waiting.duration)/1e6,4)        AS waiting_ms
    FROM (SELECT DISTINCT item_uid, entry_slug, title, lifetime, running, waiting FROM items) s
    GROUP BY item_uid
) t ON t.item_uid = w.uid
ORDER BY w.depth"
    )
}

/// Read a source file from the code root directory.
///
/// The path must be relative and within `code_root` — path traversal (`..`) is rejected.
pub fn execute_read_code(code_root: &str, path: &str) -> Result<String, String> {
    if code_root.is_empty() {
        return Err("Code path not configured. Set it in the Settings panel.".into());
    }

    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        return Err("Invalid path: must be relative with no '..' or absolute prefix.".into());
    }

    let full_path = Path::new(code_root).join(path);
    std::fs::read_to_string(&full_path).map_err(|e| {
        let mut msg = format!("Cannot read '{}': {}", full_path.display(), e);
        // On file-not-found, show the recursive file tree so the agent can self-correct.
        if e.kind() == std::io::ErrorKind::NotFound {
            if let Ok(tree) = recursive_file_tree(code_root) {
                msg.push_str("\n\nAvailable files:\n");
                msg.push_str(&tree);
            }
        }
        msg
    })
}

// ── Wiki tools (JIT knowledge retrieval) ─────────────────────────────────────
//
// Three named tools give the agent on-demand access to a structured Legion wiki
// (replacing the removed 126K knowledge injection with retrieval): `wiki_index`
// (browse the corpus), `wiki_read` (read a page or one section), `wiki_search`
// (find pages by keyword). All are pure functions that serve ONLY .md files under
// the configured wiki root — never `raw/` and never outside the root. Path safety
// mirrors `execute_read_code` AND validates against the enumerated page set.

/// Default per-read character budget for `wiki_read`. Chosen from the corpus size
/// distribution (median ~5.7 KB, p90 ~7.2 KB, max ~40 KB): 12_000 chars (~3K
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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
            let section = rel.split('/').next().unwrap_or("").to_owned();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let (title, summary, tags) = parse_frontmatter(&content);
            let title = title
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| filename_to_title(&name));
            let summary = summary.filter(|s| !s.is_empty()).unwrap_or_else(|| title.clone());
            let tldr_lc = extract_section(&content, "TL;DR")
                .unwrap_or_default()
                .to_lowercase();
            let tier = wiki_tier(&section);
            out.push(WikiPage {
                path: rel,
                section,
                title,
                summary,
                tags,
                tldr_lc,
                tier,
            });
        }
    }
}

/// Build (and memoize) the page-metadata corpus for `wiki_root`. Err if the root
/// is missing/not-a-directory or contains no pages.
fn wiki_corpus(wiki_root: &str) -> Result<Arc<Vec<WikiPage>>, String> {
    if wiki_root.is_empty() {
        return Err("Wiki path not configured.".into());
    }
    let cache = WIKI_CORPUS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(c) = cache.lock().unwrap().get(wiki_root) {
        return Ok(c.clone());
    }
    let root = Path::new(wiki_root);
    if !root.is_dir() {
        return Err(format!("Wiki root '{}' is not a directory.", wiki_root));
    }
    let mut pages = Vec::new();
    collect_wiki_pages(root, root, &mut pages);
    if pages.is_empty() {
        return Err(format!("No .md pages found under wiki root '{}'.", wiki_root));
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
    let full = Path::new(wiki_root).join(&norm);
    let content = std::fs::read_to_string(&full)
        .map_err(|e| format!("Cannot read '{}': {}", full.display(), e))?;

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
    let tag_lc = tag.map(|t| t.to_lowercase());

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
        let scope = section.map(|s| format!(" in section '{s}'")).unwrap_or_default();
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

/// Gather a pre-computed overview of the profiling database.
///
/// Runs several SQL queries and combines results into a structured text summary
/// (~4–8 KB) suitable for the agent's initial context message.
#[cfg(feature = "duckdb")]
pub fn gather_overview(duckdb_path: &str) -> Result<String, String> {
    let mut out = String::with_capacity(8192);

    // ── Schema ────────────────────────────────────────────────────────────────
    out.push_str("## Schema\n");
    out.push_str(
        "Table `entries`: entry_slug TEXT PK, short_name TEXT, long_name TEXT, \
         parent_slug TEXT, type TEXT ('panel'|'slot')\n",
    );
    out.push_str(
        "Table `items`: entry_slug TEXT (FK→entries), item_uid UBIGINT, title TEXT,\n\
         lifetime/running/waiting/deferred/delayed/ready/scheduling_overhead/triggering_latency: \
         STRUCT(start BIGINT, stop BIGINT, duration BIGINT),\n\
         operation/creator/critical_path/previous_executing/mapper: \
         STRUCT(item_uid UBIGINT, title TEXT, interval STRUCT(start,stop,duration), entry_slug TEXT),\n\
         size: UBIGINT.\n\
         All timestamps are NANOSECONDS. Access STRUCTs with dot notation: \
         running.start, critical_path.item_uid.\n\n",
    );

    // ── Row counts ────────────────────────────────────────────────────────────
    let entry_count = execute_run_query_raw(duckdb_path, "SELECT COUNT(*) AS cnt FROM entries")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    let item_count = execute_run_query_raw(duckdb_path, "SELECT COUNT(*) AS cnt FROM items")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    out.push_str(&format!(
        "## Row Counts\nentries: {entry_count}  items: {item_count}\n\n"
    ));

    // ── Processor hierarchy ───────────────────────────────────────────────────
    let hier = execute_run_query_raw(
        duckdb_path,
        "SELECT parent_slug, type, COUNT(*) AS cnt, \
         STRING_AGG(entry_slug, ', ' ORDER BY entry_slug) AS slugs \
         FROM entries GROUP BY parent_slug, type ORDER BY parent_slug, type",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Processor Hierarchy\n{hier}\n\n"));

    // ── Timeline bounds ───────────────────────────────────────────────────────
    let bounds = execute_run_query_raw(
        duckdb_path,
        "SELECT MIN(lifetime.start) AS earliest_ns, MAX(lifetime.stop) AS latest_ns, \
         ROUND((MAX(lifetime.stop) - MIN(lifetime.start)) / 1e6, 1) AS span_ms FROM items",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Timeline Bounds\n{bounds}\n\n"));

    // ── Task distribution ─────────────────────────────────────────────────────
    // Top-10 headline (was 15) — orientation, not an exhaustive distribution; the
    // agent uses run_query for the full GROUP BY when it needs it. The LIMIT is
    // wrapped in a subquery because execute_run_query_raw strips a TRAILING
    // `LIMIT n` and re-applies its own 50-row cap — so an un-wrapped `LIMIT 10`
    // would still return up to 50 task types (the pre-compaction behavior).
    let dist = execute_run_query_raw(
        duckdb_path,
        "SELECT * FROM (\
           SELECT title, COUNT(*) AS cnt, \
           ROUND(AVG(running.duration)/1e6, 2) AS avg_run_ms, \
           ROUND(MAX(running.duration)/1e6, 2) AS max_run_ms \
           FROM items WHERE running IS NOT NULL \
           GROUP BY title ORDER BY cnt DESC LIMIT 10\
         ) s",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Top Task Types (by count, top 10)\n{dist}\n\n"));

    // ── Slot counts by kind ───────────────────────────────────────────────────
    let slots = execute_run_query_raw(
        duckdb_path,
        "SELECT parent_slug, COUNT(*) AS slot_cnt FROM entries WHERE type = 'slot' \
         GROUP BY parent_slug ORDER BY parent_slug",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Slots by Kind\n{slots}\n\n"));

    // ── Sample item (compact) ─────────────────────────────────────────────────
    // `SELECT *` dumped every lifecycle + cross-ref STRUCT for one row — ~63 KB on
    // bg4N2 (85% of the old overview, and overflowed the MCP tool-result budget).
    // The Schema section already lists the columns; a 4-column projection still
    // shows the populated STRUCT SHAPE (a lifecycle struct + a cross-ref struct)
    // without the dump. Full rows are one `run_query` away.
    // The inner LIMIT 1 is wrapped in a subquery: execute_run_query_raw strips a
    // TRAILING `LIMIT n` and re-applies its 50-row cap, so a bare `... LIMIT 1`
    // returned 50 FULL rows (the old 63 KB / 85%-of-output dump).
    let sample = execute_run_query_raw(
        duckdb_path,
        "SELECT item_uid, title, running, critical_path FROM (\
           SELECT * FROM items WHERE running IS NOT NULL LIMIT 1\
         ) s",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Sample Item Row (shape; SELECT * via run_query)\n{sample}\n\n"));

    // ── Profile classification (human-readable) ──────────────────────────────
    let classification = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpudev%' AND type = 'slot') AS gpu_device_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpuhost%' AND type = 'slot') AS gpu_host_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%cpu%' AND type = 'slot') AS cpu_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%util%' AND type = 'slot') AS util_count, \
         (SELECT COUNT(*) FROM entries WHERE type = 'panel' AND (parent_slug IS NULL OR parent_slug = '') AND entry_slug <> 'all') AS node_count",
    );
    out.push_str("## Profile Classification\n");
    match &classification {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let gpu = row.get("gpu_device_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cpu = row.get("cpu_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let util = row.get("util_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let nodes = row.get("node_count").and_then(|v| v.as_u64()).unwrap_or(1);
                    let profile_type = if gpu > 0 { "GPU-present" } else { "CPU-only" };
                    let node_str = if nodes <= 1 { "single-node".to_string() } else { format!("{}-node", nodes) };
                    out.push_str(&format!(
                        "- Type: {} {}\n- GPUs: {} | CPUs: {} | Utility procs: {}\n",
                        profile_type, node_str, gpu, cpu, util
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Tracing detection (human-readable) ────────────────────────────────────
    let tracing = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         COUNT(*) FILTER (WHERE title LIKE '%Replay Physical Trace%') AS replay_trace_count, \
         COUNT(*) FILTER (WHERE title LIKE '%map_task%' OR title LIKE '%select_task_options%') AS mapper_call_count, \
         COUNT(*) FILTER (WHERE entry_slug LIKE '%util%') AS total_util_items \
         FROM items",
    );
    out.push_str("## Tracing Status\n");
    match &tracing {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let rpt = row.get("replay_trace_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let mapper = row.get("mapper_call_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!(
                        "- Replay Physical Trace tasks: {}\n\
                         - Mapper calls: {}\n",
                        rpt, mapper
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Per-kind utilization (human-readable) ─────────────────────────────────
    let utilization = execute_run_query_raw(
        duckdb_path,
        "WITH bounds AS ( \
           SELECT MIN(lifetime.start) AS t_start, MAX(lifetime.stop) AS t_stop \
           FROM items \
         ), \
         kind_busy AS ( \
           SELECT \
             CASE \
               WHEN entry_slug LIKE '%gpu%' THEN 'GPU' \
               WHEN entry_slug LIKE '%cpu%' AND entry_slug NOT LIKE '%util%' THEN 'CPU' \
               WHEN entry_slug LIKE '%util%' THEN 'Utility' \
               WHEN entry_slug LIKE '%channel%' THEN 'Channel' \
               ELSE 'Other' \
             END AS kind, \
             entry_slug, \
             SUM(running.duration) AS busy_ns \
           FROM items \
           WHERE running IS NOT NULL \
           GROUP BY kind, entry_slug \
         ) \
         SELECT kb.kind, \
                COUNT(DISTINCT kb.entry_slug) AS proc_count, \
                ROUND(AVG(kb.busy_ns * 100.0 / (b.t_stop - b.t_start)), 1) AS avg_util_pct, \
                ROUND(MAX(kb.busy_ns * 100.0 / (b.t_stop - b.t_start)), 1) AS max_util_pct \
         FROM kind_busy kb CROSS JOIN bounds b \
         GROUP BY kb.kind \
         ORDER BY kb.kind",
    );
    out.push_str("## Per-Kind Utilization\n");
    match &utilization {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                for row in &parsed {
                    let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                    let count = row.get("proc_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let avg = row.get("avg_util_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max = row.get("max_util_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {}: {} proc(s), avg {:.1}% util, max {:.1}%\n",
                        kind, count, avg, max
                    ));
                }
                if parsed.is_empty() {
                    out.push_str("(no utilization data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Deferred health (human-readable) ──────────────────────────────────────
    let deferred = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         ROUND(AVG(deferred.duration) / 1e6, 2) AS avg_deferred_ms, \
         ROUND(PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY deferred.duration) / 1e6, 2) AS p10_deferred_ms, \
         ROUND(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY deferred.duration) / 1e6, 2) AS p50_deferred_ms, \
         COUNT(*) FILTER (WHERE deferred.duration < 100000) AS items_under_100us \
         FROM items WHERE deferred IS NOT NULL AND deferred.duration IS NOT NULL",
    );
    out.push_str("## Deferred Health (runtime run-ahead)\n");
    match &deferred {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let avg = row.get("avg_deferred_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let p10 = row.get("p10_deferred_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let p50 = row.get("p50_deferred_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let under_100us = row.get("items_under_100us").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!(
                        "- Avg: {:.2}ms | P10: {:.2}ms | P50: {:.2}ms | Items <100us: {}\n",
                        avg, p10, p50, under_100us
                    ));
                } else {
                    out.push_str("(no deferred data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Utility meta-task breakdown ────────────────────────────────────────
    let util_breakdown = execute_run_query_raw(
        duckdb_path,
        "WITH util_breakdown AS ( \
           SELECT \
             CASE \
               WHEN title LIKE '%Logical Dependence%' OR title LIKE '%Disjointness%' THEN 'analysis' \
               WHEN title LIKE 'Mapper Call%' OR title LIKE '%MapperRuntime%' THEN 'mapper' \
               WHEN title LIKE '%Replay Physical Trace%' THEN 'trace_replay' \
               WHEN title LIKE '%Scheduler%' OR title LIKE '%Prepipeline%' THEN 'scheduling' \
               ELSE 'other' \
             END AS category, \
             SUM(running.duration) AS total_ns \
           FROM items \
           WHERE entry_slug LIKE '%util%' AND running IS NOT NULL \
           GROUP BY category \
         ) \
         SELECT category, \
                ROUND(total_ns / 1e6, 1) AS total_ms, \
                ROUND(total_ns * 100.0 / NULLIF((SELECT SUM(total_ns) FROM util_breakdown), 0), 1) AS pct \
         FROM util_breakdown \
         ORDER BY total_ns DESC",
    );
    out.push_str("## Utility Meta-Task Breakdown\n");
    match &util_breakdown {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                let mut has_trace_replay = false;
                let mut mapper_pct = 0.0_f64;
                for row in &parsed {
                    let cat = row.get("category").and_then(|v| v.as_str()).unwrap_or("?");
                    let ms = row.get("total_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let pct = row.get("pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let label = match cat {
                        "analysis" => "Analysis (dependence/disjointness)",
                        "mapper" => "Mapper calls",
                        "trace_replay" => "Trace replay",
                        "scheduling" => "Scheduling (scheduler/prepipeline)",
                        _ => "Other meta-tasks",
                    };
                    out.push_str(&format!("- {}: {:.1}% ({:.1}ms)\n", label, pct, ms));
                    if cat == "trace_replay" && pct > 0.0 {
                        has_trace_replay = true;
                    }
                    if cat == "mapper" {
                        mapper_pct = pct;
                    }
                }
                let _ = (has_trace_replay, mapper_pct);
                if parsed.is_empty() {
                    out.push_str("(no utility items)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Mapper call analysis ───────────────────────────────────────────────
    let mapper_calls = execute_run_query_raw(
        duckdb_path,
        "SELECT COUNT(*) AS call_count, \
         ROUND(AVG(running.duration) / 1e6, 2) AS avg_ms, \
         ROUND(PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY running.duration) / 1e6, 2) AS p95_ms, \
         ROUND(MAX(running.duration) / 1e6, 2) AS max_ms \
         FROM items \
         WHERE title LIKE 'Mapper Call%' AND running IS NOT NULL",
    );
    out.push_str("## Mapper Call Analysis\n");
    match &mapper_calls {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let count = row.get("call_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let avg = row.get("avg_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let p95 = row.get("p95_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max = row.get("max_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {} mapper calls | avg: {:.2}ms | P95: {:.2}ms | max: {:.2}ms\n",
                        count, avg, p95, max
                    ));
                    if max > 10.0 {
                        out.push_str(
                            "- ANOMALOUS — individual mapper calls >10ms, \
                             possible OS descheduling or expensive mapper logic\n",
                        );
                    }
                    if count == 0 {
                        out.push_str("- No mapper calls found (tracing may be handling all mapping)\n");
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Task granularity ───────────────────────────────────────────────────
    let granularity = execute_run_query_raw(
        duckdb_path,
        "SELECT COUNT(*) AS app_task_count, \
         ROUND(AVG(running.duration) / 1e6, 3) AS avg_run_ms, \
         ROUND(MIN(running.duration) / 1e6, 3) AS min_run_ms, \
         ROUND(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY running.duration) / 1e6, 3) AS median_run_ms \
         FROM items \
         WHERE running IS NOT NULL \
           AND entry_slug NOT LIKE '%util%' \
           AND entry_slug NOT LIKE '%chan%' \
           AND title NOT LIKE '%ProfTask%' \
           AND title NOT LIKE 'top_level%'",
    );
    out.push_str("## Task Granularity (application tasks only)\n");
    match &granularity {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let count = row.get("app_task_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let avg = row.get("avg_run_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let min = row.get("min_run_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let median = row.get("median_run_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {} app tasks | median: {:.3}ms | avg: {:.3}ms | min: {:.3}ms\n",
                        count, median, avg, min
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Channel copy patterns ──────────────────────────────────────────────
    // Copies live on %chan% slots and carry their cost in `lifetime` + `size`,
    // NEVER `running` (title = "Copy"). The old `running IS NOT NULL` filter +
    // `SUM(running.duration)` reported 0 copies / 0ms while the truth on bg4N2 is
    // 207 distinct copies / 53.3ms. Dedup by item_uid (235 raw chan rows -> 207
    // distinct copies). Volume is intentionally omitted (see TODO below).
    let copies = execute_run_query_raw(
        duckdb_path,
        "SELECT COUNT(*) AS copy_count, \
         ROUND(SUM(ld) / 1e6, 1) AS total_copy_ms \
         FROM (SELECT item_uid, any_value(lifetime.duration) AS ld \
               FROM (SELECT DISTINCT item_uid, entry_slug, lifetime \
                     FROM items WHERE entry_slug LIKE '%chan%') s \
               GROUP BY item_uid)",
    );
    out.push_str("## Channel Copy Patterns\n");
    match &copies {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let count = row.get("copy_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let ms = row.get("total_copy_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {} copies | total comm time (lifetime): {:.1}ms\n",
                        count, ms
                    ));
                    // TODO(volume): total bytes copied needs unit-aware parsing of
                    // `size` (a unit-suffixed string: "76.000 KiB", "96 B", ...);
                    // units vary (B/KiB on bg4N2) so a naive CAST is wrong. Deferred
                    // rather than report an incorrect MB figure.
                    if count == 0 {
                        out.push_str("- No channel copies (CPU-only or no data movement)\n");
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Delayed distribution (Realm pickup latency) ────────────────────────
    let delayed = execute_run_query_raw(
        duckdb_path,
        "SELECT ROUND(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY delayed.duration) / 1e6, 3) AS p50_ms, \
         ROUND(PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY delayed.duration) / 1e6, 3) AS p90_ms, \
         ROUND(MAX(delayed.duration) / 1e6, 2) AS max_ms, \
         COUNT(*) FILTER (WHERE delayed.duration > 1000000) AS items_over_1ms \
         FROM items \
         WHERE delayed IS NOT NULL AND delayed.duration IS NOT NULL",
    );
    out.push_str("## Delayed Distribution (Realm pickup latency)\n");
    match &delayed {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let p50 = row.get("p50_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let p90 = row.get("p90_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max = row.get("max_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let over_1ms = row.get("items_over_1ms").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!(
                        "- P50: {:.3}ms | P90: {:.3}ms | max: {:.2}ms | items >1ms: {}\n",
                        p50, p90, max, over_1ms
                    ));
                } else {
                    out.push_str("(no delayed data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Triggering latency ─────────────────────────────────────────────────
    let trig_latency = execute_run_query_raw(
        duckdb_path,
        "SELECT ROUND(PERCENTILE_CONT(0.9) WITHIN GROUP \
         (ORDER BY triggering_latency.duration) / 1e6, 3) AS p90_ms, \
         ROUND(MAX(triggering_latency.duration) / 1e6, 2) AS max_ms, \
         COUNT(*) FILTER (WHERE triggering_latency.duration > 1000000) AS items_over_1ms \
         FROM items \
         WHERE triggering_latency IS NOT NULL AND triggering_latency.duration IS NOT NULL",
    );
    out.push_str("## Triggering Latency (event propagation)\n");
    match &trig_latency {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let p90 = row.get("p90_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max = row.get("max_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let over_1ms = row.get("items_over_1ms").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!(
                        "- P90: {:.3}ms | max: {:.2}ms | items >1ms: {}\n",
                        p90, max, over_1ms
                    ));
                } else {
                    out.push_str("(no triggering latency data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── Python/Legate detection ────────────────────────────────────────────
    let python = execute_run_query_raw(
        duckdb_path,
        "SELECT COUNT(*) AS py_proc_count \
         FROM entries \
         WHERE (entry_slug LIKE '%py%' OR short_name LIKE '%Python%') AND type = 'slot'",
    );
    out.push_str("## Python/Legate Detection\n");
    match &python {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let count = row.get("py_proc_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    if count > 0 {
                        out.push_str(&format!(
                            "- Python processors: {} (Legate/cuNumeric)\n",
                            count
                        ));
                    } else {
                        out.push_str("- Python processors: 0\n");
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => out.push_str(&format!("(error: {})\n", e)),
    }
    out.push('\n');

    // ── GC and instance activity ───────────────────────────────────────────
    let gc = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         COUNT(*) FILTER (WHERE title LIKE '%Garbage Collection%' \
                            OR title LIKE '%Free Instance%' \
                            OR title LIKE '%Malloc Instance%') AS gc_count, \
         ROUND(COALESCE(SUM(running.duration) FILTER (WHERE title LIKE '%Garbage Collection%' \
                                                        OR title LIKE '%Free Instance%' \
                                                        OR title LIKE '%Malloc Instance%'), 0) / 1e6, 1) AS gc_total_ms, \
         COUNT(*) FILTER (WHERE entry_slug LIKE '%system%' \
                            OR entry_slug LIKE '%fbmem%' \
                            OR entry_slug LIKE '%zcmem%') AS instance_items \
         FROM items WHERE running IS NOT NULL",
    );
    out.push_str("## GC and Instance Activity\n");
    match &gc {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let gc_count = row.get("gc_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let gc_ms = row.get("gc_total_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let inst = row.get("instance_items").and_then(|v| v.as_u64()).unwrap_or(0);
                    if gc_count > 0 {
                        out.push_str(&format!(
                            "- GC activity detected: {} events, {:.1}ms — check for memory pressure\n",
                            gc_count, gc_ms
                        ));
                    } else {
                        out.push_str("- No GC activity detected\n");
                    }
                    out.push_str(&format!("- Instance-related items: {}\n", inst));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Per-node utility balance ───────────────────────────────────────────
    let node_util = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         SPLIT_PART(entry_slug, '_', 1) AS node, \
         COUNT(DISTINCT entry_slug) AS util_procs, \
         ROUND(SUM(running.duration) / 1e6, 1) AS total_busy_ms \
         FROM items \
         WHERE entry_slug LIKE '%util%' AND running IS NOT NULL \
         GROUP BY node ORDER BY node",
    );
    out.push_str("## Per-Node Utility Balance\n");
    match &node_util {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if parsed.len() <= 1 {
                    out.push_str("- Single-node profile — balanced by definition\n");
                    for row in &parsed {
                        let node = row.get("node").and_then(|v| v.as_str()).unwrap_or("?");
                        let ms = row.get("total_busy_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        out.push_str(&format!("- {}: {:.1}ms utility busy\n", node, ms));
                    }
                } else {
                    let mut min_ms = f64::MAX;
                    let mut max_ms = 0.0_f64;
                    let mut min_node = "?".to_string();
                    let mut max_node = "?".to_string();
                    for row in &parsed {
                        let node = row.get("node").and_then(|v| v.as_str()).unwrap_or("?");
                        let ms = row.get("total_busy_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        out.push_str(&format!("- {}: {:.1}ms utility busy\n", node, ms));
                        if ms < min_ms {
                            min_ms = ms;
                            min_node = node.to_string();
                        }
                        if ms > max_ms {
                            max_ms = ms;
                            max_node = node.to_string();
                        }
                    }
                    let ratio = max_ms / min_ms.max(0.1);
                    out.push_str(&format!(
                        "- Utility-work spread: {:.1}x (busiest {}, lightest {})\n",
                        ratio, max_node, min_node
                    ));
                }
                if parsed.is_empty() {
                    out.push_str("(no utility data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Channel direction analysis ─────────────────────────────────────────
    // Per-channel comm activity. Copies use `lifetime` (not `running`); dedup by
    // item_uid, assigning each copy to its min(entry_slug) so the per-channel
    // total matches the 53.3ms whole-run figure. Volume omitted (see TODO above).
    let chan_dir = execute_run_query_raw(
        duckdb_path,
        "SELECT entry_slug, COUNT(*) AS copy_count, \
         ROUND(SUM(ld) / 1e6, 1) AS total_ms \
         FROM (SELECT item_uid, min(entry_slug) AS entry_slug, \
                 any_value(lifetime.duration) AS ld \
               FROM (SELECT DISTINCT item_uid, entry_slug, lifetime \
                     FROM items WHERE entry_slug LIKE '%chan%') s \
               GROUP BY item_uid) \
         GROUP BY entry_slug ORDER BY total_ms DESC",
    );
    out.push_str("## Channel Direction Analysis\n");
    match &chan_dir {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                let mut has_pcie = false;
                let mut has_inter_node = false;
                let count = parsed.len().min(5);
                if parsed.is_empty() {
                    out.push_str("- No channel copy activity\n");
                } else {
                    for row in parsed.iter().take(5) {
                        let slug = row.get("entry_slug").and_then(|v| v.as_str()).unwrap_or("?");
                        let copies = row.get("copy_count").and_then(|v| v.as_u64()).unwrap_or(0);
                        let ms = row.get("total_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        // Classify channel direction from slug
                        let direction = classify_channel_slug(slug);
                        if direction.contains("PCIe") {
                            has_pcie = true;
                        }
                        if direction.contains("inter-node") {
                            has_inter_node = true;
                        }

                        out.push_str(&format!(
                            "- {} [{}]: {} copies, {:.1}ms\n",
                            slug, direction, copies, ms
                        ));
                    }
                    if parsed.len() > 5 {
                        out.push_str(&format!("  ... and {} more channels\n", parsed.len() - 5));
                    }
                    if has_pcie {
                        out.push_str("- PCIe (SYS↔FB) copies present\n");
                    }
                    if has_inter_node {
                        out.push_str("- Inter-node copies present\n");
                    }
                }
                let _ = count; // suppress unused warning
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Copy-to-compute ratio ──────────────────────────────────────────────
    // Copy time = SUM(lifetime.duration) on channels, dedup'd by item_uid: copies
    // have no `running` (old `SUM(running)` on chan was always 0), and a copy's
    // rows are the SAME transfer on 2 channel slugs sharing ONE lifetime — true
    // duplication, so dedup is required. Compute time keeps the NAIVE
    // `SUM(running.duration)` on cpu/gpu non-util (2263.1ms on bg4N2): compute
    // items repeat as genuine RE-EXECUTIONS (distinct running slices), so the raw
    // sum is correct and collapsing per item_uid would wrongly drop them.
    let copy_ratio = execute_run_query_raw(
        duckdb_path,
        "WITH copy_time AS ( \
           SELECT COALESCE(SUM(ld), 0) AS copy_ns \
           FROM (SELECT item_uid, any_value(lifetime.duration) AS ld \
                 FROM (SELECT DISTINCT item_uid, entry_slug, lifetime \
                       FROM items WHERE entry_slug LIKE '%chan%') s \
                 GROUP BY item_uid) \
         ), \
         compute_time AS ( \
           SELECT COALESCE(SUM(running.duration), 0) AS compute_ns \
           FROM items \
           WHERE (entry_slug LIKE '%cpu%' OR entry_slug LIKE '%gpu%') \
             AND entry_slug NOT LIKE '%util%' AND running IS NOT NULL \
         ) \
         SELECT \
           ROUND(ct.copy_ns / 1e6, 1) AS copy_total_ms, \
           ROUND(cm.compute_ns / 1e6, 1) AS compute_total_ms, \
           ROUND(ct.copy_ns * 100.0 / GREATEST(ct.copy_ns + cm.compute_ns, 1), 1) AS copy_pct \
         FROM copy_time ct, compute_time cm",
    );
    out.push_str("## Copy-to-Compute Ratio\n");
    match &copy_ratio {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let copy_ms = row.get("copy_total_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let compute_ms = row.get("compute_total_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let pct = row.get("copy_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- Copy: {:.1}ms | Compute: {:.1}ms | Copy fraction: {:.1}%\n",
                        copy_ms, compute_ms, pct
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Scheduling overhead ────────────────────────────────────────────────
    let sched_overhead = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         ROUND(PERCENTILE_CONT(0.9) WITHIN GROUP \
           (ORDER BY scheduling_overhead.duration) / 1e6, 2) AS p90_overhead_ms, \
         ROUND(AVG(scheduling_overhead.duration) / 1e6, 2) AS avg_overhead_ms, \
         COUNT(*) AS items_with_overhead \
         FROM items \
         WHERE scheduling_overhead IS NOT NULL \
           AND scheduling_overhead.duration IS NOT NULL",
    );
    out.push_str("## Scheduling Overhead\n");
    match &sched_overhead {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let p90 = row.get("p90_overhead_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let avg = row.get("avg_overhead_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let count = row.get("items_with_overhead").and_then(|v| v.as_u64()).unwrap_or(0);
                    out.push_str(&format!(
                        "- P90: {:.2}ms | Avg: {:.2}ms ({} items)\n",
                        p90, avg, count
                    ));
                } else {
                    out.push_str("(no scheduling overhead data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Application processor balance ──────────────────────────────────────
    let proc_balance = execute_run_query_raw(
        duckdb_path,
        "WITH bounds AS ( \
           SELECT MIN(lifetime.start) AS t_start, MAX(lifetime.stop) AS t_stop FROM items \
         ), \
         per_proc AS ( \
           SELECT entry_slug, \
             CASE \
               WHEN entry_slug LIKE '%gpu%' THEN 'GPU' \
               WHEN entry_slug LIKE '%cpu%' AND entry_slug NOT LIKE '%util%' THEN 'CPU' \
               ELSE 'Other' \
             END AS kind, \
             ROUND(SUM(running.duration) * 100.0 / (b.t_stop - b.t_start), 1) AS util_pct \
           FROM items CROSS JOIN bounds b \
           WHERE running IS NOT NULL \
             AND (entry_slug LIKE '%gpu%' OR \
                  (entry_slug LIKE '%cpu%' AND entry_slug NOT LIKE '%util%')) \
           GROUP BY entry_slug, kind, b.t_stop, b.t_start \
         ) \
         SELECT kind, COUNT(*) AS proc_count, \
           ROUND(MIN(util_pct), 1) AS min_util, \
           ROUND(MAX(util_pct), 1) AS max_util, \
           ROUND(AVG(util_pct), 1) AS avg_util \
         FROM per_proc GROUP BY kind ORDER BY kind",
    );
    out.push_str("## Application Processor Balance\n");
    match &proc_balance {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                for row in &parsed {
                    let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                    let count = row.get("proc_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let min_u = row.get("min_util").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let max_u = row.get("max_util").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let avg_u = row.get("avg_util").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {}: {} procs, min {:.1}%, max {:.1}%, avg {:.1}%\n",
                        kind, count, min_u, max_u, avg_u
                    ));
                    let ratio = max_u / min_u.max(0.1);
                    out.push_str(&format!("  spread: {:.1}x (max/min)\n", ratio));
                }
                if parsed.is_empty() {
                    out.push_str("(no application processor data)\n");
                }
            } else {
                out.push_str(&format!("{}\n", json_str));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {})\n", e));
            }
        }
    }
    out.push('\n');

    // ── Navigation anchors ─────────────────────────────────────────────────
    out.push_str("## Navigation Anchors\n");

    // Sub-query A: Steady-state midpoint (middle 20% of profile)
    let midpoint = execute_run_query_raw(
        duckdb_path,
        "SELECT \
         MIN(lifetime.start) + (MAX(lifetime.stop) - MIN(lifetime.start)) * 4 / 10 AS steady_start, \
         MIN(lifetime.start) + (MAX(lifetime.stop) - MIN(lifetime.start)) * 6 / 10 AS steady_end \
         FROM items",
    );
    match &midpoint {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let start = row.get("steady_start").and_then(|v| v.as_i64()).unwrap_or(0);
                    let end = row.get("steady_end").and_then(|v| v.as_i64()).unwrap_or(0);
                    if start > 0 && end > start {
                        out.push_str(&format!(
                            "- Steady-state zoom (middle 20%%): [{}, {}]\n",
                            start, end
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (midpoint error: {})\n", e));
            }
        }
    }

    // Sub-query B: Worst mapper call
    let worst_mapper = execute_run_query_raw(
        duckdb_path,
        "SELECT entry_slug, title, running.start AS start_ns, \
         running.stop AS stop_ns, ROUND(running.duration / 1e6, 2) AS duration_ms \
         FROM items \
         WHERE title LIKE 'Mapper Call%' AND running IS NOT NULL \
         ORDER BY running.duration DESC LIMIT 1",
    );
    match &worst_mapper {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let slug = row.get("entry_slug").and_then(|v| v.as_str()).unwrap_or("?");
                    let ms = row.get("duration_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let start = row.get("start_ns").and_then(|v| v.as_i64()).unwrap_or(0);
                    let stop = row.get("stop_ns").and_then(|v| v.as_i64()).unwrap_or(0);
                    if ms > 0.0 {
                        out.push_str(&format!(
                            "- Longest mapper call: {:.2}ms at [{}, {}] on {}\n",
                            ms, start, stop, slug
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (mapper anchor error: {})\n", e));
            }
        }
    }

    // Sub-query C: Largest application processor gap
    let worst_gap = execute_run_query_raw(
        duckdb_path,
        "WITH ordered AS ( \
           SELECT entry_slug, running.stop AS task_end, \
             LEAD(running.start) OVER (PARTITION BY entry_slug ORDER BY running.start) AS next_start \
           FROM items \
           WHERE running IS NOT NULL \
             AND (entry_slug LIKE '%gpu%' OR \
                  (entry_slug LIKE '%cpu%' AND entry_slug NOT LIKE '%util%')) \
         ) \
         SELECT entry_slug, task_end AS gap_start_ns, next_start AS gap_end_ns, \
           ROUND((next_start - task_end) / 1e6, 2) AS gap_ms \
         FROM ordered \
         WHERE next_start > task_end \
         ORDER BY gap_ms DESC LIMIT 1",
    );
    match &worst_gap {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let slug = row.get("entry_slug").and_then(|v| v.as_str()).unwrap_or("?");
                    let ms = row.get("gap_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let start = row.get("gap_start_ns").and_then(|v| v.as_i64()).unwrap_or(0);
                    let end = row.get("gap_end_ns").and_then(|v| v.as_i64()).unwrap_or(0);
                    if ms > 0.0 {
                        out.push_str(&format!(
                            "- Largest app processor gap: {:.2}ms at [{}, {}] on {}\n",
                            ms, start, end, slug
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (gap anchor error: {})\n", e));
            }
        }
    }

    out.push_str("Use zoom_to or set_view with these nanosecond ranges to navigate directly.\n");
    out.push('\n');

    Ok(out)
}

#[cfg(feature = "duckdb")]
/// Convert a JSON array of objects into a markdown table.
/// Returns None if the input is empty, not an array, or parse fails.
fn json_array_to_markdown_table(json_str: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let arr = parsed.as_array()?;
    if arr.is_empty() {
        return None;
    }

    // Extract column names from first object, sorted for determinism
    let first = arr[0].as_object()?;
    let mut columns: Vec<&String> = first.keys().collect();
    columns.sort();

    // Build header
    let mut out = String::new();
    out.push('|');
    for col in &columns {
        out.push_str(&format!(" {} |", col));
    }
    out.push('\n');

    // Separator
    out.push('|');
    for _ in &columns {
        out.push_str("------|");
    }
    out.push('\n');

    // Data rows
    for row in arr {
        out.push('|');
        for col in &columns {
            let val = row.get(col.as_str());
            let cell = match val {
                None | Some(serde_json::Value::Null) => String::new(),
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Number(n)) => n.to_string(),
                Some(serde_json::Value::Bool(b)) => b.to_string(),
                Some(v @ serde_json::Value::Object(_)) | Some(v @ serde_json::Value::Array(_)) => {
                    serde_json::to_string(v).unwrap_or_default()
                }
            };
            out.push_str(&format!(" {} |", cell));
        }
        out.push('\n');
    }

    Some(out)
}

#[cfg(feature = "duckdb")]
/// Classify a channel entry_slug into a direction label.
///
/// Best-effort parsing:
/// - Two different node prefixes (e.g. "n0" and "n1") → "inter-node"
/// - Contains both 's' and 'f' components (system mem and framebuffer) → "SYS↔FB (PCIe)"
/// - Otherwise → "local"
fn classify_channel_slug(slug: &str) -> &'static str {
    // Extract the part after "chan_" (e.g. "n0s0_n1s0" or "fn0s0")
    let chan_part = slug
        .find("chan_")
        .map(|i| &slug[i + 4..])
        .unwrap_or(slug);

    // Check for inter-node: look for different node numbers
    let node_numbers: Vec<&str> = chan_part
        .split(|c: char| !c.is_ascii_digit() && c != 'n')
        .filter(|s| s.starts_with('n') && s.len() > 1)
        .collect();
    if node_numbers.len() >= 2 {
        let first = node_numbers[0];
        if node_numbers.iter().any(|n| *n != first) {
            return "inter-node";
        }
    }

    // Check for SYS↔FB: presence of both 's' (system) and 'f' (framebuffer) components
    let has_sys = chan_part.contains('s') && !chan_part.starts_with("sys");
    let has_fb = chan_part.contains('f');
    if has_sys && has_fb {
        return "SYS↔FB (PCIe)";
    }

    "local"
}

/// True iff `slug` is a known `entries.entry_slug` in the profile DB. Used to
/// reject a `highlight` on an unknown row (both the embedded agent and the
/// in-viewer MCP server). Injection-safe: the slug is NEVER interpolated into SQL
/// — we fetch the full slug set and check membership in Rust.
#[cfg(feature = "duckdb")]
pub fn slug_exists(duckdb_path: &str, slug: &str) -> bool {
    let json = match execute_run_query_raw(
        duckdb_path,
        "SELECT json_group_array(entry_slug) AS all_slugs FROM entries",
    ) {
        Ok(j) => j,
        Err(_) => return false,
    };
    serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| {
            let cell = v.get(0)?.get("all_slugs")?;
            // Normally a nested JSON array; tolerate a JSON-encoded string too.
            let arr = match cell {
                serde_json::Value::Array(a) => a.clone(),
                serde_json::Value::String(s) => serde_json::from_str::<Vec<serde_json::Value>>(s).ok()?,
                _ => return None,
            };
            Some(arr.iter().any(|x| x.as_str() == Some(slug)))
        })
        .unwrap_or(false)
}

/// Return Claude API tool definitions for the agent.
///
/// - `has_duckdb`: include `run_query` tool (only if duckdb feature AND path is set)
/// - `has_code`: include `read_code` tool (only if code path is configured)
/// - `has_wiki`: include `wiki_index`/`wiki_read`/`wiki_search` (only if a wiki root is configured)
///
/// `screenshot` and `zoom_to` are included as stubs (Phase 3b implementation).
pub fn tool_definitions(has_duckdb: bool, has_code: bool, has_wiki: bool) -> Vec<serde_json::Value> {
    let mut tools = Vec::new();

    if has_duckdb {
        tools.push(serde_json::json!({
            "name": "run_query",
            "description":
                "Execute a read-only SQL query against the Legion profiling DuckDB database. \
                 Returns up to 50 rows as JSON. Do NOT include a trailing semicolon.\n\n\
                 RANGE QUERIES — READ FIRST: when the question says \"in the range\", \
                 \"in the highlighted region\", \"in the selection\", or \"longest / most time \
                 within a window\", you MUST CLIP each item to the window with \
                 SUM(LEAST(running.stop, {hi_ns}) - GREATEST(running.start, {lo_ns})) (see example #10) — \
                 do NOT use the plain time-range FILTER (example #2), and do NOT compare full \
                 durations: an item mostly OUTSIDE the window must not win.\n\n\
                 TRUNCATION: results are capped at 50 rows. If MORE rows matched, the JSON array \
                 holds the first 50 plus a final marker object {\"_truncated\": true, \"_shown\": 50, ...} — \
                 when you see it, the result is INCOMPLETE; refine with aggregation (GROUP BY/COUNT/SUM) \
                 or a narrower filter rather than trusting the 50 shown.\n\n\
                 SCHEMA REMINDER: Two tables — `entries` (entry_slug, short_name, long_name, parent_slug, type) \
                 and `items` (entry_slug, item_uid, title, plus STRUCT columns). \
                 All STRUCT columns use dot notation: running.start, running.duration, critical_path.item_uid, etc.\n\n\
                 EXAMPLE QUERIES:\n\
                 1. Per-processor utilization:\n\
                    SELECT entry_slug, COUNT(*) AS task_count,\n\
                      ROUND(SUM(running.duration) / 1e6, 1) AS busy_ms\n\
                    FROM items WHERE running IS NOT NULL\n\
                    GROUP BY entry_slug ORDER BY busy_ms DESC\n\n\
                 2. Tasks in a time range on a specific processor:\n\
                    SELECT item_uid, title, running.start, running.stop,\n\
                      running.duration / 1e6 AS run_ms\n\
                    FROM items\n\
                    WHERE entry_slug = 'n0_gpu_g0' AND running.start < 500000000\n\
                      AND running.stop > 400000000\n\
                    ORDER BY running.start\n\n\
                 3. GPU idle gaps (find gaps between consecutive tasks):\n\
                    WITH ordered AS (\n\
                      SELECT running.stop AS task_end,\n\
                        LEAD(running.start) OVER (PARTITION BY entry_slug ORDER BY running.start) AS next_start\n\
                      FROM items\n\
                      WHERE entry_slug LIKE '%gpu%' AND running IS NOT NULL\n\
                    )\n\
                    SELECT (next_start - task_end) / 1e6 AS gap_ms,\n\
                      task_end AS gap_start_ns, next_start AS gap_end_ns\n\
                    FROM ordered WHERE next_start > task_end\n\
                    ORDER BY gap_ms DESC LIMIT 10\n\n\
                 4. Walk critical path from a task:\n\
                    WITH RECURSIVE chain AS (\n\
                      SELECT item_uid, title, entry_slug,\n\
                        running.duration / 1e6 AS run_ms,\n\
                        critical_path.item_uid AS cp_uid, 1 AS depth\n\
                      FROM items WHERE item_uid = <START_UID>\n\
                      UNION ALL\n\
                      SELECT i.item_uid, i.title, i.entry_slug,\n\
                        i.running.duration / 1e6, i.critical_path.item_uid, c.depth + 1\n\
                      FROM items i JOIN chain c ON i.item_uid = c.cp_uid\n\
                      WHERE c.cp_uid IS NOT NULL AND c.depth < 10\n\
                    )\n\
                    SELECT * FROM chain ORDER BY depth\n\n\
                 5. Identify processor kinds by entry_slug pattern:\n\
                    SELECT entry_slug FROM entries\n\
                    WHERE type = 'slot' AND entry_slug LIKE '%gpu%'\n\
                    ORDER BY entry_slug\n\n\
                 6. Task lifecycle breakdown:\n\
                    SELECT title,\n\
                      ROUND(AVG(waiting.duration) / 1e6, 2) AS avg_wait_ms,\n\
                      ROUND(AVG(deferred.duration) / 1e6, 2) AS avg_defer_ms,\n\
                      ROUND(AVG(running.duration) / 1e6, 2) AS avg_run_ms\n\
                    FROM items WHERE running IS NOT NULL\n\
                    GROUP BY title ORDER BY avg_wait_ms DESC LIMIT 10\n\n\
                 7. Channel copy analysis (copies use lifetime + size, NEVER running; dedup by item_uid):\n\
                    SELECT entry_slug, COUNT(*) AS copy_count,\n\
                      ROUND(SUM(ld) / 1e6, 1) AS total_ms\n\
                    FROM (SELECT item_uid, min(entry_slug) AS entry_slug,\n\
                            any_value(lifetime.duration) AS ld\n\
                          FROM (SELECT DISTINCT item_uid, entry_slug, lifetime\n\
                                FROM items WHERE entry_slug LIKE '%chan%') s\n\
                          GROUP BY item_uid)\n\
                    GROUP BY entry_slug ORDER BY total_ms DESC\n\
                    (size is a unit-suffixed string like '76.000 KiB'/'96 B' \u{2014} parse units for bytes)\n\n\
                 8. Utility activity during a time window:\n\
                    SELECT title, COUNT(*) AS cnt,\n\
                      ROUND(SUM(running.duration) / 1e6, 1) AS total_ms\n\
                    FROM items\n\
                    WHERE entry_slug LIKE '%util%' AND running IS NOT NULL\n\
                      AND running.start < {end_ns} AND running.stop > {start_ns}\n\
                    GROUP BY title ORDER BY total_ms DESC\n\n\
                 9. Tasks with thin pipeline (deferred near zero):\n\
                    SELECT entry_slug, title, item_uid,\n\
                      ROUND(deferred.duration / 1e6, 3) AS deferred_ms,\n\
                      ROUND(running.duration / 1e6, 2) AS run_ms\n\
                    FROM items\n\
                    WHERE deferred IS NOT NULL AND deferred.duration < 100000\n\
                      AND running IS NOT NULL\n\
                    ORDER BY running.start\n\n\
                 10. Most running time WITHIN a range (CLIP to the range — for \"longest / most\n\
                     time in the selected range\"; count only the portion inside [{lo_ns}, {hi_ns}]):\n\
                    SELECT item_uid,\n\
                      ROUND(SUM(LEAST(e, {hi_ns}) - GREATEST(s, {lo_ns})) / 1e6, 2) AS ms_in_range\n\
                    FROM (SELECT DISTINCT item_uid, entry_slug,\n\
                            running.start AS s, running.stop AS e\n\
                          FROM items WHERE running.start IS NOT NULL\n\
                            AND (entry_slug LIKE '%_cpu_%' OR entry_slug LIKE '%_gpudev_%')\n\
                            AND running.start < {hi_ns} AND running.stop > {lo_ns})\n\
                    GROUP BY item_uid ORDER BY ms_in_range DESC, item_uid ASC\n\
                    (CLIP each task's running to the range with LEAST/GREATEST; do NOT compare\n\
                     full task durations — a task mostly outside the range should not win)\n\n\
                 IMPORTANT: You can call this tool multiple times per response to batch independent queries. \
                 Do NOT include LIMIT in your query — a hard cap of 50 rows is applied automatically. \
                 Before writing a query, check the overview's Schema section for exact column names. \
                 If a column shows 'Not available in this profile' in the overview, do NOT attempt to query it.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "The SELECT query to execute. No semicolon needed."
                    },
                    "purpose": {
                        "type": "string",
                        "description": "Brief description of why you are running this query."
                    }
                },
                "required": ["sql", "purpose"]
            }
        }));

        tools.push(serde_json::json!({
            "name": "overview",
            "description":
                "Return a precomputed structured overview of the profiling database: \
                 schema, row counts, processor hierarchy, per-kind utilization, timeline \
                 bounds, top task types, and other orientation signals. Takes no arguments. \
                 Call this once at the start when you need to get oriented before writing queries.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }

    if has_code {
        tools.push(serde_json::json!({
            "name": "list_files",
            "description":
                "List source files in the code directory tree. Shows files recursively with \
                 sizes, organized by directory. Use BEFORE read_code to discover what files \
                 exist. Pass a subdirectory path to narrow the listing.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Subdirectory to list (relative to code root). Empty or '.' for the root."
                    }
                },
                "required": []
            }
        }));

        tools.push(serde_json::json!({
            "name": "read_code",
            "description":
                "Read an application source file (path relative to the configured code root). \
                 Use to understand task logic, mapper policies, and application structure. \
                 Use list_files first to discover available files. \
                 Do NOT call read_code for a file that was already pre-loaded in the scan \
                 message — check the 'Application Source Code (pre-loaded)' section above first.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative file path, e.g. 'circuit.cc' or 'src/main.cc'."
                    }
                },
                "required": ["path"]
            }
        }));
    }

    if has_wiki {
        tools.push(serde_json::json!({
            "name": "wiki_index",
            "description":
                "Browse the structured Legion knowledge wiki: a grouped, one-line-per-page \
                 listing (path, tier, summary) of every page. CONSUMPTION LOOP: call wiki_index \
                 (or wiki_search) to find the right page → wiki_read it → follow its `Related` \
                 links to neighbours. Consult the wiki for Legion concepts, pitfalls, and \
                 diagnostic workflows instead of guessing. Pass `section` to list just one \
                 section (concepts, pitfalls, workflows, glossary, meta, applications). \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "description": "Optional top-level section to scope the listing (e.g. 'pitfalls')."
                    }
                },
                "required": []
            }
        }));

        tools.push(serde_json::json!({
            "name": "wiki_read",
            "description":
                "Read one wiki page (path as shown by wiki_index/wiki_search, e.g. \
                 'concepts/mapper.md'). Returns the page verbatim with its `Related` links \
                 intact — follow those to neighbouring pages. Pass `section` to return ONLY one \
                 `## Header` block (e.g. 'Debug signals', 'Failure modes', 'TL;DR'). Long pages \
                 are capped at max_chars (default 12000) with a truncation marker; raise \
                 max_chars or read a specific section to see more. \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative page path under the wiki root, e.g. 'concepts/mapper.md'."
                    },
                    "section": {
                        "type": "string",
                        "description": "Optional `## Header` to return just that block (e.g. 'Debug signals')."
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Optional character cap (default 12000)."
                    }
                },
                "required": ["path"]
            }
        }));

        tools.push(serde_json::json!({
            "name": "wiki_search",
            "description":
                "Search the wiki by keyword/substring over page titles, summaries, TL;DRs, and \
                 tags. Returns up to `limit` ranked {path, section, tldr, tags, score} matches — \
                 these are PAGES TO READ, not a synthesized answer; wiki_read the top hits. Use \
                 this when you don't know which page covers a topic. Optional `section` and `tag` \
                 narrow the search. \
                 USE THIS WHEN the question asks for a performance CLASSIFICATION \
                 (compute-/communication-/runtime-bound), a LIFECYCLE phase meaning (waiting vs \
                 deferred vs ready), a flag/concept/pitfall definition, or a ROOT-CAUSE verdict — \
                 consult the wiki BEFORE asserting any such claim from prior knowledge. DON'T use \
                 it for raw per-task numbers (use run_query) or to re-read a page you already read. \
                 If the question contains words like 'bound', 'stall', 'overhead', \
                 'waiting/deferred/ready/lifecycle', 'mapper', or 'critical path' — wiki_search \
                 that term and wiki_read the top hits before answering.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword(s) to match (case-insensitive substring)."
                    },
                    "section": {
                        "type": "string",
                        "description": "Optional top-level section to scope the search."
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional tag to require (exact, case-insensitive)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (default 5)."
                    }
                },
                "required": ["query"]
            }
        }));
    }

    tools.push(serde_json::json!({
        "name": "screenshot",
        "description":
            "Capture the current profiler timeline view as a PNG image. \
             Returns an image along with metadata showing the visible time \
             range (in nanoseconds) and the list of entry_slugs for each \
             processor row. Use this to visually inspect the timeline layout, \
             verify idle gaps, and understand spatial patterns across processors. \
             Do NOT take another screenshot if you already have a recent screenshot \
             of the same region — use the data you have. Prefer set_view over \
             screenshot when you need to change both zoom and scroll position.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "zoom_to",
        "description":
            "Zoom the profiler timeline to a specific nanosecond range and capture \
             a screenshot. Returns a screenshot with metadata showing the exact \
             visible range and entry_slugs. Use after identifying a region of \
             interest via queries to see fine-grained task scheduling, verify \
             gaps, and inspect processor utilization within the zoomed range. \
             Prefer set_view over zoom_to + scroll_to when you need both zoom and \
             vertical navigation — it's one round-trip instead of two.",
        "input_schema": {
            "type": "object",
            "properties": {
                "start_ns": {
                    "type": "integer",
                    "description": "Start of the zoom range in nanoseconds"
                },
                "stop_ns": {
                    "type": "integer",
                    "description": "End of the zoom range in nanoseconds"
                }
            },
            "required": ["start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "pan",
        "description":
            "Pan the timeline left or right by a percentage of the visible range. \
             Returns a screenshot with metadata after panning. Use to explore \
             adjacent time regions without changing zoom level — e.g. to see \
             what comes after a gap or to scan across the timeline incrementally.",
        "input_schema": {
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "enum": ["left", "right"],
                    "description": "Direction to pan: \"left\" moves earlier in time, \"right\" moves later."
                },
                "percent": {
                    "type": "number",
                    "description": "Percentage of the visible range to pan by (default 25). E.g. 50 pans half a screen width."
                }
            },
            "required": ["direction"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "scroll_to",
        "description":
            "Scroll the timeline vertically to bring a specific processor row \
             into view. Identifies the processor by entry_slug (e.g. \"n0_gpu_g0\", \
             \"n0_util_u0\"). Auto-expands the processor's parent panel if collapsed. \
             Returns a screenshot with metadata. Use to navigate to a processor of \
             interest — e.g. scroll to utility rows or channel rows.",
        "input_schema": {
            "type": "object",
            "properties": {
                "entry_slug": {
                    "type": "string",
                    "description": "The entry_slug of the processor to scroll to (e.g. \"n0_gpu_g0\")."
                }
            },
            "required": ["entry_slug"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "set_view",
        "description":
            "Combined view control in one call: zoom to a nanosecond range and \
             optionally scroll to a processor row, restrict the view to specific \
             processor kinds, expand/collapse kinds, and change row height. More \
             efficient than separate calls. Returns a screenshot with metadata. \
             Kind tokens come from the entry slugs / overview — typically \
             \"gpudev\", \"gpuhost\", \"cpu\", \"utility\", \"io\", \"system\", \
             \"framebuffer\", \"chan\", \"dp\". Matching is case-insensitive and \
             by substring, so \"gpu\" selects both \"gpudev\" and \"gpuhost\".",
        "input_schema": {
            "type": "object",
            "properties": {
                "start_ns": {
                    "type": "integer",
                    "description": "Start of the zoom range in nanoseconds"
                },
                "stop_ns": {
                    "type": "integer",
                    "description": "End of the zoom range in nanoseconds"
                },
                "entry_slug": {
                    "type": "string",
                    "description": "Optional entry_slug to scroll to after zooming."
                },
                "filter_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: show ONLY these processor kinds (others are hidden). Omit or pass [] to show all kinds."
                },
                "expand_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: expand these processor kinds to show their rows."
                },
                "collapse_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: collapse these processor kinds to hide their rows."
                },
                "vertical_scale": {
                    "type": "number",
                    "description": "Optional row-height multiplier in [0.25, 4.0]. >1 makes crowded rows taller/readable."
                }
            },
            "required": ["start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "search",
        "description":
            "Set the timeline's search box to a string. Every task whose title \
             matches is highlighted in place across the visible rows, and the \
             returned screenshot metadata reports how many matched. Use this to \
             LOCATE tasks visually by name; use run_query when you need an exact \
             list or per-task numbers. Searching clears when you call reset_view.",
        "input_schema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to search task titles for, e.g. \"calculate_new_currents\"."
                }
            },
            "required": ["query"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "reset_view",
        "description":
            "Reset the view to a clean slate: zoom out to the whole profile, clear \
             any kind filter, clear the search, and reset row height to default. \
             Use this before answering about the overall structure of the profile, \
             or to undo a previous set_view/search.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "highlight",
        "description":
            "Mark a region of the timeline to point the user at it (e.g. the task \
             you identified as the blocker). Highlights appear as clickable chips in \
             the chat and as overlays on the timeline. Use an entry_slug + the \
             nanosecond range from your queries or screenshots.",
        "input_schema": {
            "type": "object",
            "properties": {
                "entry_slug": { "type": "string", "description": "Processor row, e.g. \"n0_gpu_g0\"." },
                "start_ns": { "type": "integer", "description": "Start of the region (ns)." },
                "stop_ns": { "type": "integer", "description": "End of the region (ns)." },
                "severity": { "type": "string", "description": "\"critical\", \"high\", or \"medium\" (default medium)." },
                "label": { "type": "string", "description": "Short description shown on the chip." }
            },
            "required": ["entry_slug", "start_ns", "stop_ns"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "ask_user",
        "description":
            "Ask the user a clarifying question and wait for their answer. Use this \
             when the request is ambiguous, when you must choose between materially \
             different interpretations, or when you are unsure what the user wants — \
             prefer asking over guessing. Provide 2–4 concrete options when you can.",
        "input_schema": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional 2–4 suggested answers, shown to the user as buttons."
                }
            },
            "required": ["question"]
        }
    }));

    tools.push(serde_json::json!({
        "name": "clear_highlights",
        "description":
            "Remove ALL highlight overlays from the timeline. Use this when the user \
             asks to remove, clear, undo, or hide the highlights you added.",
        "input_schema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(serde_json::json!({
        "name": "update_findings",
        "description":
            "Record durable conclusions about THIS profile (its structure, the main \
             bottleneck, things you've ruled out) as short notes. They persist across \
             the user's questions and are shown back to you at the start of each new \
             question, so you don't re-derive them. Appends one bullet by default; pass \
             replace=true with a consolidated multi-line note to rewrite the whole list \
             (use this to correct or prune outdated notes). Keep each note to one terse \
             sentence.",
        "input_schema": {
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "The finding(s). One bullet, or multiple lines when replace=true."
                },
                "replace": {
                    "type": "boolean",
                    "description": "If true, replace ALL existing findings with this note. Default false (append)."
                }
            },
            "required": ["note"]
        }
    }));

    tools
}

#[cfg(all(test, feature = "duckdb"))]
mod tests {
    use super::*;

    /// Path to the shared bg4N2 test profile. It is an untracked fixture living in
    /// the repo root (one level above the crate dir), so resolve it relative to
    /// `CARGO_MANIFEST_DIR`. Tests soft-skip if it is absent on this machine.
    fn test_db_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb")
    }

    /// Task 2 unit (no DB): the 50-row cap is marked only when MORE than 50 rows
    /// were present; everything else is returned UNCHANGED.
    #[test]
    fn test_mark_truncation_if_over() {
        // <= 50, empty, scalar, non-JSON -> unchanged.
        let small = r#"[{"a":1},{"a":2}]"#.to_string();
        assert_eq!(mark_truncation_if_over(small.clone()), small);
        assert_eq!(mark_truncation_if_over("[]".into()), "[]");
        assert_eq!(mark_truncation_if_over("42".into()), "42"); // scalar (e.g. json_group_array NULL guard)
        assert_eq!(mark_truncation_if_over("not json".into()), "not json");

        // Exactly 50 -> unchanged (boundary).
        let fifty =
            serde_json::to_string(&(0..50).map(|i| serde_json::json!({ "a": i })).collect::<Vec<_>>()).unwrap();
        assert_eq!(mark_truncation_if_over(fifty.clone()), fifty);

        // 51 -> first 50 kept + ONE marker appended (51 elements total).
        let over =
            serde_json::to_string(&(0..51).map(|i| serde_json::json!({ "a": i })).collect::<Vec<_>>()).unwrap();
        let marked: Vec<serde_json::Value> =
            serde_json::from_str(&mark_truncation_if_over(over)).unwrap();
        assert_eq!(marked.len(), 51, "50 data + 1 marker");
        assert_eq!(marked[49]["a"], serde_json::json!(49), "last real row preserved");
        assert_eq!(marked[50]["_truncated"], serde_json::json!(true));
        assert_eq!(marked[50]["_shown"], serde_json::json!(50));
    }

    /// Task 2 integration: a >50-row query is marked; a small one is not.
    /// `json_group_array`-in-one-row aggregates are len 1 → never marked.
    #[test]
    fn test_run_query_truncation_marker_live() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("skipping test_run_query_truncation_marker_live: test DB absent");
            return;
        }
        let db = db.to_str().unwrap();

        // bg4N2 has 68 entries (> 50) -> 50 data rows + a marker.
        let out = execute_run_query_raw(db, "SELECT entry_slug FROM entries").expect("query");
        let arr: Vec<serde_json::Value> = serde_json::from_str(&out).expect("JSON array");
        assert_eq!(arr.len(), 51, "50 rows + 1 marker, got {}", arr.len());
        assert!(arr[..50].iter().all(|r| r.get("entry_slug").is_some() && r.get("_truncated").is_none()));
        assert_eq!(arr[50]["_truncated"], serde_json::json!(true));
        assert_eq!(arr[50]["_shown"], serde_json::json!(50));

        // A small aggregate is returned UNCHANGED (no marker).
        let small = execute_run_query_raw(db, "SELECT COUNT(*) AS n FROM items").expect("query");
        assert!(!small.contains("_truncated"), "small result must NOT be marked: {small}");
        let sa: Vec<serde_json::Value> = serde_json::from_str(&small).unwrap();
        assert_eq!(sa.len(), 1);
    }

    /// SIZE GUARD (overview-compact regression): `gather_overview` must stay
    /// INLINE-consumable for the MCP tool-result budget. Before compaction it was
    /// ~73,558 chars (~18.4K tokens) and overflowed Claude Code's ~25K-token
    /// MCP_OUTPUT budget → spilled to a file and went unread. The dominant bloat was
    /// `SELECT * FROM items LIMIT 1`, which (because execute_run_query_raw strips a
    /// trailing LIMIT and re-caps at 50) returned 50 FULL rows ≈ 63 KB (85%). After
    /// compaction it is ~7.3 KB (~1.8K tokens). This 16 KB cap leaves room for new
    /// signals yet still catches a re-bloat (the old dump alone was 4× this).
    #[test]
    fn test_overview_fits_inline_budget() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("skipping test_overview_fits_inline_budget: test DB absent");
            return;
        }
        let out = gather_overview(db.to_str().unwrap()).expect("gather_overview");
        assert!(
            out.len() < 16_000,
            "overview must stay inline-consumable; was {} chars (~{} tokens) — re-bloat?",
            out.len(),
            out.len() / 4
        );
    }

    /// SIGNALS PRESERVED: compaction trimmed only verbosity (the per-item sample and
    /// the task-type distribution), never an orientation SIGNAL. Every high-value
    /// section the agent re-derived by hand must still be present, and the trimmed
    /// sections must be in their compact form (not the old dumps). Exact numeric
    /// fidelity of the copy signal is pinned separately by
    /// `test_channel_copy_lifetime_fix` (it asserts the Copy-to-Compute numbers via
    /// gather_overview) — that SQL was not touched here.
    #[test]
    fn test_overview_preserves_signals() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("skipping test_overview_preserves_signals: test DB absent");
            return;
        }
        let out = gather_overview(db.to_str().unwrap()).expect("gather_overview");
        for header in [
            "## Profile Classification",
            "## Per-Kind Utilization",
            "## Copy-to-Compute Ratio",
            "## Application Processor Balance",
            "## Navigation Anchors",
            "## Schema",
            "## Row Counts",
            "## Timeline Bounds",
            "## Tracing Status",
            "## Deferred Health",
        ] {
            assert!(out.contains(header), "kept signal section missing: {header}");
        }
        // The trimmed sample is compact (the old 50-row `SELECT *` dump was ~63 KB).
        let sample = section_body(&out, "## Sample Item Row");
        assert!(
            sample.len() < 1_000,
            "Sample Item Row must be a compact shape sample, was {} chars",
            sample.len()
        );
        // It is exactly ONE row, not the old 50-row dump. (Count the array length —
        // not `"item_uid"` occurrences, since the critical_path struct nests its own.)
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(sample.trim()).expect("sample is a JSON array");
        assert_eq!(rows.len(), 1, "sample must be exactly one row, got {}", rows.len());
    }

    /// Return the text of `## <name>` up to the next `## ` header (test helper).
    fn section_body<'a>(overview: &'a str, header: &str) -> &'a str {
        let start = overview.find(header).expect("section present");
        let rest = &overview[start..];
        // Skip this header line, then cut at the next "\n## ".
        let after_hdr = rest.find('\n').map(|i| start + i).unwrap_or(overview.len());
        let body = &overview[after_hdr..];
        match body.find("\n## ") {
            Some(i) => &body[..i],
            None => body,
        }
    }

    /// P0(a): the read-only + `enable_external_access(false)` hardening must block
    /// table-function file reads (e.g. `read_text`) in a FROM clause, while benign
    /// SELECTs still work. The probe MUST use the FROM form: scalar `SELECT
    /// read_text(...)` raises a Binder Error regardless of hardening (false positive).
    #[test]
    fn test_run_query_blocks_external_file_read() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("skipping {}: test DB absent at {}", "exfil", db.display());
            return;
        }
        let db = db.to_str().unwrap();

        // Benign query still works through the hardened connection.
        let ok = execute_run_query_raw(db, "SELECT COUNT(*) AS cnt FROM items")
            .expect("benign SELECT should succeed through hardened connection");
        assert!(ok.starts_with('['), "benign query should return a JSON array, got: {ok}");
        assert!(ok.contains("cnt"), "benign query JSON missing alias, got: {ok}");

        // Exfil probe (table function in a FROM clause) must be blocked at the engine,
        // exercised through the real wrapped path the agent uses.
        let err = execute_run_query_raw(db, "SELECT content FROM read_text('/etc/hosts')")
            .expect_err("external file read must be blocked by the hardened connection");
        assert!(
            err.contains("Permission") || err.contains("disabled") || err.contains("external"),
            "blocked error should signal external-access denial, got: {err}"
        );
        assert!(
            !err.contains("Binder Error"),
            "must be an access denial, not a binder error (the scalar-form false positive): {err}"
        );

        // Positive control: the SAME probe SUCCEEDS against an unhardened (default,
        // external-access-enabled) connection — proving the test gates the hardening
        // rather than some unrelated failure. This is the in-process analogue of the
        // CLI positive control that read /etc/hosts before the fix.
        let unhardened = duckdb::Connection::open_in_memory().expect("open in-memory");
        let leaked: Result<String, _> =
            unhardened.query_row("SELECT content FROM read_text('/etc/hosts')", [], |r| r.get(0));
        assert!(
            leaked.is_ok(),
            "positive control: an unhardened connection should read the file, got {leaked:?}"
        );
    }

    /// P0(c): the canonical per-`item_uid` dedup (`dedup_select_sql`) must yield
    /// the TRUE durations for uid 48, not the inflated naive `SUM(lifetime…)`.
    /// Pins the NUMBERS (never the title). Owns a WRITABLE temp copy of the DB
    /// because the live connection is read-only (CREATE VIEW is rejected there).
    #[test]
    fn test_dedup_durations_uid48() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping dedup: test DB absent at {}", src.display());
            return;
        }
        // Writable temp copy — the canonical fixture must not be mutated and the
        // read-only live connection cannot CREATE VIEW.
        let tmp = std::env::temp_dir().join("legion_p0c_dedup_uid48.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");

        let conn = duckdb::Connection::open(&tmp).expect("open writable temp DB");
        conn.execute_batch(&format!(
            "CREATE OR REPLACE VIEW tasks_dedup AS {}",
            dedup_select_sql()
        ))
        .expect("create dedup view on the writable connection");

        let (lifetime_ms, running_ms, longest_ms): (f64, f64, f64) = conn
            .query_row(
                "SELECT lifetime_ms, running_ms, longest_running_slice_ms \
                 FROM tasks_dedup WHERE item_uid = 48",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("query the dedup view for uid 48");

        // The naive (un-deduped) wall-clock estimate — what the dedup corrects.
        let naive_ms: f64 = conn
            .query_row(
                "SELECT round(SUM(lifetime.duration) / 1e6, 4) FROM items WHERE item_uid = 48",
                [],
                |r| r.get(0),
            )
            .expect("query the naive inflation");

        let close = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(close(lifetime_ms, 1546.0172), "lifetime_ms = {lifetime_ms}, want 1546.0172");
        assert!(close(running_ms, 432.7334), "running_ms = {running_ms}, want 432.7334");
        assert!(
            close(longest_ms, 13.1611),
            "longest_running_slice_ms = {longest_ms}, want 13.1611"
        );
        assert!(close(naive_ms, 808566.9726), "naive_ms = {naive_ms}, want 808566.9726");
        // The dedup removes a ~523x inflation in the naive wall-clock estimate.
        assert!(
            (naive_ms / lifetime_ms - 523.0).abs() < 1.0,
            "inflation ratio = {}, want ~523",
            naive_ms / lifetime_ms
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// P1.0: the find_blockers critical-path walk must be cycle-guarded. Pins ROW
    /// COUNTS and the final uid (never the word "depth-N"). Uses a DIRECT
    /// connection — the unguarded variant's 100001 rows would be impossible
    /// through execute_run_query_raw's 50-row cap; mirrors P0(c)'s writable
    /// temp-copy style.
    #[test]
    fn test_find_blockers_cycle_guard() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping find_blockers: test DB absent at {}", src.display());
            return;
        }
        let tmp = std::env::temp_dir().join("legion_p1_find_blockers.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");
        let conn = duckdb::Connection::open(&tmp).expect("open writable temp DB");

        // Guarded walk from uid 48: an acyclic chain of exactly 7 rows ending at
        // the root blocker uid 1 (External Thread).
        let (rows, max_depth, deepest_uid, deepest_title): (i64, i32, u64, String) = conn
            .query_row(
                &format!(
                    "SELECT count(*), max(depth), arg_max(uid, depth), arg_max(title, depth) \
                     FROM ({}) s",
                    find_blockers_sql(48)
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("find_blockers(48)");
        assert_eq!(rows, 7, "uid 48 chain should be exactly 7 rows");
        assert_eq!(max_depth, 6, "uid 48 max depth should be 6");
        assert_eq!(deepest_uid, 1, "uid 48 root blocker should be uid 1");
        assert!(
            deepest_title.contains("External Thread"),
            "deepest title should be the External Thread, got: {deepest_title}"
        );

        // Guarded walk from uid 2220 stops at the 2220<->1481 2-cycle: 2 rows.
        let (rows2, max_depth2): (i64, i32) = conn
            .query_row(
                &format!("SELECT count(*), max(depth) FROM ({}) s", find_blockers_sql(2220)),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("find_blockers(2220)");
        assert_eq!(rows2, 2, "uid 2220 guarded chain should stop at 2 rows");
        assert_eq!(max_depth2, 1, "uid 2220 guarded max depth should be 1");

        let _ = std::fs::remove_file(&tmp);
    }

    /// Documents WHY the cycle guard exists: the UNguarded walk (no visited-set
    /// guard) from the cycle uid 2220 runs away to the depth cap — 100001 rows /
    /// max_depth 100000 — instead of stopping at the real 2220<->1481 2-cycle.
    ///
    /// `#[ignore]`d because the 100k-iteration recursive CTE takes ~40s through
    /// the bundled duckdb, which is too slow for the default `cargo test` loop.
    /// Run explicitly to verify the failure mode is pinned:
    ///   cargo test --features ai,duckdb test_find_blockers_unguarded_runaway -- --ignored
    #[test]
    #[ignore = "slow (~40s): 100k-row recursive runaway; run with --ignored"]
    fn test_find_blockers_unguarded_runaway() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping unguarded runaway: test DB absent at {}", src.display());
            return;
        }
        let tmp = std::env::temp_dir().join("legion_p1_find_blockers_unguarded.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");
        let conn = duckdb::Connection::open(&tmp).expect("open writable temp DB");

        let (urows, umax): (i64, i32) = conn
            .query_row(
                "WITH RECURSIVE edges AS (
                     SELECT DISTINCT item_uid AS src, critical_path.item_uid AS dst
                     FROM items WHERE critical_path.item_uid IS NOT NULL
                 ),
                 walk AS (
                     SELECT CAST(2220 AS UBIGINT) AS uid, 0 AS depth
                     UNION ALL
                     SELECT e.dst, w.depth + 1 FROM walk w JOIN edges e ON e.src = w.uid
                     WHERE w.depth < 100000
                 )
                 SELECT count(*), max(depth) FROM walk",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("unguarded runaway");
        assert_eq!(urows, 100001, "unguarded walk should run away to 100001 rows");
        assert_eq!(umax, 100000, "unguarded walk should hit the 100000 depth cap");

        let _ = std::fs::remove_file(&tmp);
    }

    /// P1.A(1): channel copies use `lifetime`, not `running`. The pre-fix overview
    /// query (`running IS NOT NULL` on `%chan%`) reported 0 copies / 0ms — a lie;
    /// the truth on bg4N2 is 207 distinct copies / 53.3ms via lifetime. Red→green
    /// on a DIRECT oracle connection, then asserts the real `gather_overview`
    /// (tool path) now surfaces the corrected numbers.
    #[test]
    fn test_channel_copy_lifetime_fix() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping channel-copy fix: test DB absent at {}", src.display());
            return;
        }
        let tmp = std::env::temp_dir().join("legion_p1a_channel_copy.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");
        let db = tmp.to_str().unwrap();

        // Direct oracle (independent answer key). Scoped so the connection is
        // dropped before gather_overview opens the same file read-only.
        {
            let conn = duckdb::Connection::open(&tmp).expect("open writable temp DB");

            // RED: the OLD buggy query (running on chan) finds 0 copies.
            let old_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM items \
                     WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .expect("old buggy copy count");
            assert_eq!(old_count, 0, "documents the bug: running-on-chan finds 0 copies");

            // GREEN: corrected copies via lifetime, dedup'd by item_uid.
            let (count, comm_ms): (i64, f64) = conn
                .query_row(
                    "SELECT COUNT(*), ROUND(SUM(ld) / 1e6, 1) \
                     FROM (SELECT item_uid, any_value(lifetime.duration) AS ld \
                           FROM (SELECT DISTINCT item_uid, entry_slug, lifetime \
                                 FROM items WHERE entry_slug LIKE '%chan%') s \
                           GROUP BY item_uid)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .expect("corrected copy count + comm time");
            assert_eq!(count, 207, "corrected distinct copy count");
            assert!((comm_ms - 53.3).abs() < 0.1, "corrected comm time, got {comm_ms}");
        }

        // Tool path: the real gather_overview output must now surface the copies.
        let overview = gather_overview(db).expect("gather_overview");
        assert!(overview.contains("207 copies"), "overview should report 207 copies");
        assert!(overview.contains("53.3"), "overview should report 53.3ms comm time");
        assert!(
            !overview.contains("No channel copies"),
            "overview must no longer claim there are no copies"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// Parse an execute_run_query_raw JSON array and return the `item_uid` of the
    /// row with the greatest `dur_ms` (argmax computed in Rust — robust to row
    /// order through the json_group_array wrap).
    fn argmax_uid_by_dur(json: &str) -> Option<u64> {
        let rows: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
        rows.iter()
            .filter_map(|r| Some((r.get("item_uid")?.as_u64()?, r.get("dur_ms")?.as_f64()?)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(uid, _)| uid)
    }

    /// First row's `key` as f64 from an execute_run_query_raw JSON array.
    fn first_f64(json: &str, key: &str) -> Option<f64> {
        let rows: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
        rows.first()?.get(key)?.as_f64()
    }

    /// P1.A(2) L1: "which task in [1.0s,1.5s] ran longest?" — scope matters. Uses a
    /// DETERMINISTIC per-slice argmax oracle (NOT `any_value` over multi-slice
    /// items, which is non-deterministic: uid 48 has ~33 running slices, uid 1 is a
    /// long-lived io thread). app-procs scope (cpu/gpudev) -> uid 48; all-items
    /// scope (no proc filter) -> uid 1. Asserts oracle == tool-path on both, and
    /// that the two scopes disagree. (The spec's 221/1461 were `any_value`
    /// artifacts — see executor log P1.A(2).)
    #[test]
    fn test_l1_longest_in_range_scope_matters() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping L1: test DB absent at {}", src.display());
            return;
        }
        let tmp = std::env::temp_dir().join("legion_p1a_l1.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");
        let db = tmp.to_str().unwrap();

        // Per-slice argmax of running.duration over slices overlapping [1.0s,1.5s].
        let app_sql = "WITH d AS (SELECT DISTINCT item_uid, entry_slug, running.start AS s, \
             running.stop AS e, running.duration AS dur FROM items \
             WHERE running IS NOT NULL AND (entry_slug LIKE '%cpu%' OR entry_slug LIKE '%gpudev%')) \
             SELECT item_uid, ROUND(dur / 1e6, 4) AS dur_ms FROM d \
             WHERE s < 1500000000 AND e > 1000000000 ORDER BY dur_ms DESC";
        let all_sql = "WITH d AS (SELECT DISTINCT item_uid, entry_slug, running.start AS s, \
             running.stop AS e, running.duration AS dur FROM items WHERE running IS NOT NULL) \
             SELECT item_uid, ROUND(dur / 1e6, 4) AS dur_ms FROM d \
             WHERE s < 1500000000 AND e > 1000000000 ORDER BY dur_ms DESC";

        // Oracle: direct connection, LIMIT 1.
        let (oracle_app, oracle_all): (u64, u64) = {
            let conn = duckdb::Connection::open(&tmp).expect("open temp");
            let app = conn
                .query_row(&format!("{app_sql} LIMIT 1"), [], |r| r.get(0))
                .expect("oracle app");
            let all = conn
                .query_row(&format!("{all_sql} LIMIT 1"), [], |r| r.get(0))
                .expect("oracle all");
            (app, all)
        };
        assert_eq!(oracle_app, 48, "app-scope longest in [1.0s,1.5s]");
        assert_eq!(oracle_all, 1, "all-items longest in [1.0s,1.5s]");
        assert_ne!(oracle_app, oracle_all, "scope matters: app-scope != all-items");

        // Tool path: same questions through execute_run_query_raw must agree.
        let tool_app = argmax_uid_by_dur(&execute_run_query_raw(db, app_sql).expect("tool app"));
        let tool_all = argmax_uid_by_dur(&execute_run_query_raw(db, all_sql).expect("tool all"));
        assert_eq!(tool_app, Some(oracle_app), "tool-path app == oracle");
        assert_eq!(tool_all, Some(oracle_all), "tool-path all == oracle");

        let _ = std::fs::remove_file(&tmp);
    }

    /// P1.A(2) L3: compute- vs communication-bound in [1.8s,2.3s]. compute =
    /// SUM(running) on cpu/gpudev (dedup'd) ~= 478.7ms; comm = SUM(lifetime) on
    /// chan (dedup'd) ~= 49.8ms -> computation-bound. The comm side is the key
    /// parity check: the tool-path (lifetime-based) comm query must equal the
    /// direct oracle, proving the agent's copy path is correct.
    #[test]
    fn test_l3_compute_vs_comm_bound() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!("skipping L3: test DB absent at {}", src.display());
            return;
        }
        let tmp = std::env::temp_dir().join("legion_p1a_l3.duckdb");
        let _ = std::fs::remove_file(&tmp);
        std::fs::copy(&src, &tmp).expect("copy test DB to a writable temp file");
        let db = tmp.to_str().unwrap();

        // DETERMINISTIC per-slice sum (NOT any_value over the dedup grain, which
        // can decorrelate start/stop/dur across an item's slices). Compute items
        // repeat as genuine re-executions (distinct running slices), so each slice
        // whose own interval overlaps the window is summed once. uids 48/221 have
        // NO single slice overlapping [1.8s,2.3s], so they are correctly excluded.
        let compute_sql = "WITH d AS (SELECT DISTINCT item_uid, entry_slug, running.start AS s, \
             running.stop AS e, running.duration AS dur FROM items \
             WHERE running IS NOT NULL AND (entry_slug LIKE '%cpu%' OR entry_slug LIKE '%gpudev%')) \
             SELECT ROUND(SUM(dur) / 1e6, 1) AS ms FROM d WHERE s < 2300000000 AND e > 1800000000";
        // comm dedups by item_uid (any_value over single-lifetime chan copies is
        // deterministic; a per-slice SUM would double-count the 28 cross-slug copies).
        let comm_sql = "WITH d AS (SELECT DISTINCT item_uid, entry_slug, lifetime FROM items \
             WHERE entry_slug LIKE '%chan%'), \
             g AS (SELECT item_uid, any_value(lifetime.start) s, any_value(lifetime.stop) e, \
                   any_value(lifetime.duration) dur FROM d GROUP BY item_uid) \
             SELECT ROUND(SUM(dur) / 1e6, 1) AS ms FROM g WHERE s < 2300000000 AND e > 1800000000";

        // Oracle: direct connection.
        let (oracle_compute, oracle_comm): (f64, f64) = {
            let conn = duckdb::Connection::open(&tmp).expect("open temp");
            let c = conn.query_row(compute_sql, [], |r| r.get(0)).expect("oracle compute");
            let m = conn.query_row(comm_sql, [], |r| r.get(0)).expect("oracle comm");
            (c, m)
        };
        assert!((oracle_compute - 478.7).abs() < 0.1, "oracle compute, got {oracle_compute}");
        assert!((oracle_comm - 49.8).abs() < 0.1, "oracle comm, got {oracle_comm}");
        assert!(
            oracle_compute > oracle_comm,
            "verdict computation-bound: {oracle_compute} > {oracle_comm}"
        );

        // Tool path: the comm query (lifetime-based) through execute_run_query_raw
        // must equal the oracle — proving the agent's copy path handles copies.
        let tool_comm = first_f64(&execute_run_query_raw(db, comm_sql).expect("tool comm"), "ms")
            .expect("parse tool comm");
        assert!(
            (tool_comm - oracle_comm).abs() < 0.1,
            "tool-path comm parity: {tool_comm} vs oracle {oracle_comm}"
        );
        assert!((tool_comm - 49.8).abs() < 0.1, "tool-path comm == 49.8, got {tool_comm}");

        let _ = std::fs::remove_file(&tmp);
    }
}

/// Wiki-tool tests. NOT gated on `duckdb` — the `wiki_*` tools are pure file/string
/// helpers and must work under `{ai}` alone. Soft-skip when the wiki tree is absent
/// (it is an untracked fixture one level above the crate dir).
#[cfg(test)]
mod wiki_tests {
    use super::*;

    /// The Legion wiki root (`wiki-legion/wiki`, one level above the crate dir).
    fn wiki_root() -> Option<String> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wiki-legion/wiki");
        p.is_dir().then(|| p.to_string_lossy().into_owned())
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
            assert!(idx.contains(&format!("## {sec} (")), "index missing section {sec}");
        }
        // Known page paths appear as rows.
        assert!(idx.contains("concepts/"), "no concept rows");
        assert!(idx.contains("pitfalls/"), "no pitfall rows");
        // Every row carries a tier + a non-empty one-liner after the em dash.
        let rows: Vec<&str> = idx.lines().filter(|l| l.starts_with("- `")).collect();
        assert!(rows.len() >= 100, "expected >=100 pages, got {}", rows.len());
        for r in &rows {
            assert!(r.contains("[core]") || r.contains("[optional]"), "row lacks tier: {r}");
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
        assert!(!idx.contains("## concepts ("), "concepts leaked into pitfalls scope");
        // Every listed row is under pitfalls/.
        for r in idx.lines().filter(|l| l.starts_with("- `")) {
            assert!(r.contains("`pitfalls/"), "non-pitfall row in scoped index: {r}");
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
        assert!(content.contains("## TL;DR"), "page read lacks TL;DR: {page}");
        // Related links are preserved verbatim (no synthesis).
        assert!(content.contains("## Related"), "Related section stripped: {page}");
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
        assert!(block.starts_with("## Debug signals"), "block not headed correctly: {block:.40}");
        // Only that block — other level-2 headers must not bleed in.
        assert!(!block.contains("## TL;DR"), "TL;DR leaked into Debug-signals block");
        assert!(!block.contains("## Related"), "Related leaked into Debug-signals block");
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
        assert!(block.contains("# In a separate shell:"), "fenced comment truncated the section");
        assert!(block.contains("gdb -p <PID>"), "second fenced comment lost");
        assert!(block.contains("Trailing prose after the fence."), "post-fence prose lost");
        // The genuine next heading still bounds the block.
        assert!(!block.contains("## Invariants"), "next section leaked in");
        assert!(!block.contains("next section, must NOT appear"), "next section body leaked in");

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
        assert!(tiny.contains("[TRUNCATED]"), "tiny read missing truncation marker");
        assert!(tiny.contains("next_offset="), "marker missing next_offset");
        // A generous cap leaves the page whole (no marker).
        let whole = wiki_read(&root, &page, None, Some(usize::MAX)).unwrap();
        assert!(!whole.contains("[TRUNCATED]"), "whole read wrongly marked truncated");
    }

    #[test]
    fn test_wiki_search_ranks_matching_page() {
        let Some(root) = wiki_root() else {
            eprintln!("skipping: wiki root absent");
            return;
        };
        // "mapper" appears in titles/summaries of multiple mapper pages.
        let res = wiki_search(&root, "mapper", None, None, 5).unwrap();
        assert!(res.contains("mapper"), "search for 'mapper' returned nothing relevant");
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
        if let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&scoped) {
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

/// Lock-in tests for the when-to-use trigger text in tool descriptions (tool-descriptions
/// task). `tool_definitions` is a pure JSON builder, so these run under {ai} alone — they
/// guard against a future edit silently dropping the triggers that drive tool selection.
#[cfg(test)]
mod tool_description_tests {
    use super::*;

    /// Description string for a tool, with run_query + wiki tools forced on.
    fn desc(name: &str) -> String {
        tool_definitions(true, false, true)
            .into_iter()
            .find(|t| t["name"] == name)
            .and_then(|t| t["description"].as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    #[test]
    fn test_run_query_range_trigger_precedes_filter_example() {
        let d = desc("run_query");
        assert!(d.contains("in the range"), "run_query missing the range-trigger phrase");
        assert!(d.contains("LEAST") && d.contains("GREATEST"), "run_query missing the CLIP formula");
        // The trigger must come BEFORE the plain-filter example #2, or the agent reads
        // the wrong pattern first.
        let trig = d.find("in the range").expect("range trigger present");
        let ex2 = d.find("Tasks in a time range").expect("example #2 present");
        assert!(trig < ex2, "range trigger must precede example #2 (trig={trig}, ex2={ex2})");
    }

    #[test]
    fn test_wiki_descriptions_have_when_to_use() {
        for name in ["wiki_index", "wiki_read", "wiki_search"] {
            assert!(
                desc(name).contains("USE THIS WHEN"),
                "{name} description missing the WHEN-to-use trigger"
            );
        }
    }

    #[test]
    fn test_wiki_search_has_keyword_cues() {
        let d = desc("wiki_search");
        assert!(d.contains("bound"), "wiki_search missing the 'bound' keyword cue");
        assert!(d.contains("mapper"), "wiki_search missing the 'mapper' keyword cue");
    }
}

