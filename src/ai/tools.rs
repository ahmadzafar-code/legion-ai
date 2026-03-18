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
/// Returns up to 50 rows as a JSON array string.
#[cfg(feature = "duckdb")]
pub fn execute_run_query(duckdb_path: &str, sql: &str) -> Result<String, String> {
    use duckdb::Connection;

    let sql_trimmed = sql.trim().trim_end_matches(';');

    // Safety: only allow SELECT / WITH queries
    let upper = sql_trimmed.to_ascii_uppercase();
    if !upper.starts_with("SELECT") && !upper.starts_with("WITH") {
        return Err("Only SELECT/WITH queries are allowed.".into());
    }

    let conn = Connection::open(duckdb_path)
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
    let entry_count = execute_run_query(duckdb_path, "SELECT COUNT(*) AS cnt FROM entries")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    let item_count = execute_run_query(duckdb_path, "SELECT COUNT(*) AS cnt FROM items")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    out.push_str(&format!(
        "## Row Counts\nentries: {entry_count}  items: {item_count}\n\n"
    ));

    // ── Processor hierarchy ───────────────────────────────────────────────────
    let hier = execute_run_query(
        duckdb_path,
        "SELECT parent_slug, type, COUNT(*) AS cnt, \
         STRING_AGG(entry_slug, ', ' ORDER BY entry_slug) AS slugs \
         FROM entries GROUP BY parent_slug, type ORDER BY parent_slug, type",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Processor Hierarchy\n{hier}\n\n"));

    // ── Timeline bounds ───────────────────────────────────────────────────────
    let bounds = execute_run_query(
        duckdb_path,
        "SELECT MIN(lifetime.start) AS earliest_ns, MAX(lifetime.stop) AS latest_ns, \
         ROUND((MAX(lifetime.stop) - MIN(lifetime.start)) / 1e6, 1) AS span_ms FROM items",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Timeline Bounds\n{bounds}\n\n"));

    // ── Task distribution ─────────────────────────────────────────────────────
    let dist = execute_run_query(
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
    let slots = execute_run_query(
        duckdb_path,
        "SELECT parent_slug, COUNT(*) AS slot_cnt FROM entries WHERE type = 'slot' \
         GROUP BY parent_slug ORDER BY parent_slug",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Slots by Kind\n{slots}\n\n"));

    // ── Sample item ───────────────────────────────────────────────────────────
    let sample = execute_run_query(duckdb_path, "SELECT * FROM items LIMIT 1")
        .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Sample Item Row\n{sample}\n\n"));

    // ── Profile classification (human-readable) ──────────────────────────────
    let classification = execute_run_query(
        duckdb_path,
        "SELECT \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpudev%' AND type = 'slot') AS gpu_device_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpuhost%' AND type = 'slot') AS gpu_host_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%cpu%' AND type = 'slot') AS cpu_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%util%' AND type = 'slot') AS util_count, \
         (SELECT COUNT(DISTINCT SPLIT_PART(entry_slug, '/', 1)) FROM entries WHERE type = 'panel' AND parent_slug IS NOT NULL) AS node_count",
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
    let tracing = execute_run_query(
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
                    if rpt > 0 {
                        out.push_str(&format!(
                            "- TRACING IS ACTIVE: {} Replay Physical Trace tasks found\n\
                             - {} mapper calls also present (expected: first-iteration capture + init/shutdown)\n\
                             - Do NOT recommend -dm:memoize — tracing is already working\n",
                            rpt, mapper
                        ));
                    } else if mapper > 0 {
                        out.push_str(&format!(
                            "- TRACING NOT DETECTED: 0 Replay Physical Trace tasks\n\
                             - {} mapper calls found — per-task analysis overhead likely\n\
                             - Check source code for trace annotations before recommending -dm:memoize\n",
                            mapper
                        ));
                    } else {
                        out.push_str("- No tracing tasks and no mapper calls found (unusual)\n");
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

    // ── Per-kind utilization (human-readable) ─────────────────────────────────
    let utilization = execute_run_query(
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
                    let tier = if avg > 80.0 { "well-optimized" }
                        else if avg > 50.0 { "room for improvement" }
                        else { "significant issues" };
                    out.push_str(&format!(
                        "- {}: {} proc(s), avg {:.1}% util, max {:.1}% ({})\n",
                        kind, count, avg, max, tier
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
    let deferred = execute_run_query(
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
                    let health = if p10 > 1.0 { "HEALTHY — runtime is running well ahead of execution" }
                        else if avg > 1.0 { "MIXED — some tasks have thin run-ahead" }
                        else { "UNHEALTHY — execution is catching up with analysis" };
                    out.push_str(&format!(
                        "- Avg: {:.2}ms | P10: {:.2}ms | P50: {:.2}ms | Items <100us: {}\n\
                         - Assessment: {}\n\
                         - Remember: LARGE deferred = GOOD (runtime ahead), SMALL deferred = BAD (pipeline stall)\n",
                        avg, p10, p50, under_100us, health
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
    let util_breakdown = execute_run_query(
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
                if !has_trace_replay {
                    out.push_str("- NOTE: NO TRACE REPLAY activity on utility — consistent with missing tracing\n");
                }
                if mapper_pct > 30.0 {
                    out.push_str("- NOTE: MAPPER-DOMINATED — investigate individual mapper call durations\n");
                }
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
    let mapper_calls = execute_run_query(
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
    let granularity = execute_run_query(
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
                    if median < 1.0 {
                        out.push_str(
                            "- BELOW METG — tasks may be too fine-grained, \
                             runtime overhead per-task becomes significant\n",
                        );
                    } else if median > 10.0 {
                        out.push_str(
                            "- Tasks are coarse-grained — per-task overhead should be negligible\n",
                        );
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

    // ── Channel copy patterns ──────────────────────────────────────────────
    let copies = execute_run_query(
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
    let delayed = execute_run_query(
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
                    if p90 > 1.0 {
                        out.push_str(
                            "- HIGH — Realm overloaded, too many ready items competing\n",
                        );
                    } else if p90 > 0.1 {
                        out.push_str(
                            "- ELEVATED — Realm is slow to pick up ready work\n",
                        );
                    }
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
    let trig_latency = execute_run_query(
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
                    if p90 > 0.1 {
                        out.push_str(
                            "- ELEVATED — event propagation delays may bottleneck pipeline\n",
                        );
                    }
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
    let python = execute_run_query(
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
                            "- PYTHON PROCESSORS DETECTED ({}) — Legate/cuNumeric application likely\n\
                             - Check for blocking Python operations and materialization syncs\n",
                            count
                        ));
                    } else {
                        out.push_str("- No Python processors (pure C++/Regent/CUDA application)\n");
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

    Ok(out)
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
                        LEAD(running.start) OVER (ORDER BY running.start) AS next_start\n\
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
                      ROUND(SUM(size) / 1e6, 1) AS total_mb\n\
                    FROM items\n\
                    WHERE entry_slug LIKE '%chan%' AND running IS NOT NULL\n\
                    GROUP BY entry_slug ORDER BY total_ms DESC\n\n\
                 IMPORTANT: You can call this tool multiple times per response to batch independent queries. \
                 Do NOT include LIMIT in your query — a hard cap of 50 rows is applied automatically.",
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
                 Use list_files first to discover available files.",
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
             verify idle gaps, and understand spatial patterns across processors.",
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
             gaps, and inspect processor utilization within the zoomed range.",
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
            "Combined zoom + optional scroll in one call. Zooms to the given \
             nanosecond range and optionally scrolls to a specific processor row. \
             More efficient than separate zoom_to + scroll_to calls. Returns a \
             screenshot with metadata.",
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
                }
            },
            "required": ["start_ns", "stop_ns"]
        }
    }));

    tools
}
