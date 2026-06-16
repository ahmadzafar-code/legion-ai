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

    let wrapped = format!(
        "SELECT COALESCE(CAST(json_group_array(to_json(t)) AS VARCHAR), '[]') \
         FROM ({sql_for_wrap} LIMIT 50) AS t",
    );

    match conn.query_row(&wrapped, [], |row| row.get::<_, String>(0)) {
        Ok(result) if result == "null" || result.is_empty() => Ok("[]".into()),
        Ok(result) => Ok(result),
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
    let dist = execute_run_query_raw(
        duckdb_path,
        "SELECT title, COUNT(*) AS cnt, \
         ROUND(AVG(running.duration)/1e6, 2) AS avg_run_ms, \
         ROUND(MAX(running.duration)/1e6, 2) AS max_run_ms \
         FROM items WHERE running IS NOT NULL \
         GROUP BY title ORDER BY cnt DESC LIMIT 15",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Top Task Types (by count)\n{dist}\n\n"));

    // ── Slot counts by kind ───────────────────────────────────────────────────
    let slots = execute_run_query_raw(
        duckdb_path,
        "SELECT parent_slug, COUNT(*) AS slot_cnt FROM entries WHERE type = 'slot' \
         GROUP BY parent_slug ORDER BY parent_slug",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Slots by Kind\n{slots}\n\n"));

    // ── Sample item ───────────────────────────────────────────────────────────
    let sample = execute_run_query_raw(duckdb_path, "SELECT * FROM items LIMIT 1")
        .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Sample Item Row\n{sample}\n\n"));

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
    let copies = execute_run_query_raw(
        duckdb_path,
        "SELECT COUNT(*) AS copy_count, \
         ROUND(COALESCE(SUM(running.duration), 0) / 1e6, 1) AS total_copy_ms, \
         ROUND(COALESCE(SUM(CAST(size AS BIGINT)), 0) / 1e6, 1) AS total_copy_mb \
         FROM items \
         WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL",
    );
    out.push_str("## Channel Copy Patterns\n");
    match &copies {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let count = row.get("copy_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let ms = row.get("total_copy_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let mb = row.get("total_copy_mb").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {} copies | total time: {:.1}ms | total volume: {:.1}MB\n",
                        count, ms, mb
                    ));
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
    let chan_dir = execute_run_query_raw(
        duckdb_path,
        "SELECT entry_slug, COUNT(*) AS copy_count, \
         ROUND(SUM(running.duration) / 1e6, 1) AS total_ms, \
         ROUND(COALESCE(SUM(TRY_CAST(size AS BIGINT)), 0) / 1e6, 1) AS total_mb \
         FROM items \
         WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL \
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
                        let mb = row.get("total_mb").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        // Classify channel direction from slug
                        let direction = classify_channel_slug(slug);
                        if direction.contains("PCIe") {
                            has_pcie = true;
                        }
                        if direction.contains("inter-node") {
                            has_inter_node = true;
                        }

                        out.push_str(&format!(
                            "- {} [{}]: {} copies, {:.1}ms, {:.1}MB\n",
                            slug, direction, copies, ms, mb
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
    let copy_ratio = execute_run_query_raw(
        duckdb_path,
        "WITH copy_time AS ( \
           SELECT COALESCE(SUM(running.duration), 0) AS copy_ns \
           FROM items WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL \
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

/// Return Claude API tool definitions for the agent.
///
/// - `has_duckdb`: include `run_query` tool (only if duckdb feature AND path is set)
/// - `has_code`: include `read_code` tool (only if code path is configured)
///
/// `screenshot` and `zoom_to` are included as stubs (Phase 3b implementation).
pub fn tool_definitions(has_duckdb: bool, has_code: bool) -> Vec<serde_json::Value> {
    let mut tools = Vec::new();

    if has_duckdb {
        tools.push(serde_json::json!({
            "name": "run_query",
            "description":
                "Execute a read-only SQL query against the Legion profiling DuckDB database. \
                 Returns up to 50 rows as JSON. Do NOT include a trailing semicolon.\n\n\
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
                 7. Channel copy analysis:\n\
                    SELECT entry_slug, COUNT(*) AS copy_count,\n\
                      ROUND(SUM(running.duration) / 1e6, 1) AS total_ms,\n\
                      ROUND(SUM(TRY_CAST(size AS BIGINT)) / 1e6, 1) AS total_mb\n\
                    FROM items\n\
                    WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL\n\
                    GROUP BY entry_slug ORDER BY total_ms DESC\n\n\
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
}

