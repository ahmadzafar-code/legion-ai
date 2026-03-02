//! Tool implementations for Legion Prof AI analysis.
//!
//! Plain Rust functions called directly by the built-in agent (zero overhead).
//! No MCP protocol layer — external client support can be added later as a
//! thin wrapper around these same functions.
//!
//! The `run_query` and `gather_overview` tools require the `duckdb` feature.
//! The `read_code` tool requires only the `ai` feature.

use std::path::Path;

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
    std::fs::read_to_string(&full_path)
        .map_err(|e| format!("Cannot read '{}': {}", full_path.display(), e))
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

    // ── Profile classification ────────────────────────────────────────────────
    let classification = execute_run_query(
        duckdb_path,
        "SELECT \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpudev%' AND type = 'slot') AS gpu_device_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%gpuhost%' AND type = 'slot') AS gpu_host_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%cpu%' AND type = 'slot') AS cpu_count, \
         (SELECT COUNT(DISTINCT entry_slug) FROM entries WHERE entry_slug LIKE '%util%' AND type = 'slot') AS util_count, \
         (SELECT COUNT(DISTINCT SPLIT_PART(entry_slug, '/', 1)) FROM entries WHERE type = 'panel' AND parent_slug IS NOT NULL) AS node_count",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Profile Classification\n{classification}\n\n"));

    // ── Tracing detection ─────────────────────────────────────────────────────
    let tracing = execute_run_query(
        duckdb_path,
        "SELECT \
         COUNT(*) FILTER (WHERE title LIKE '%Replay Physical Trace%') AS replay_trace_count, \
         COUNT(*) FILTER (WHERE title LIKE '%map_task%' OR title LIKE '%select_task_options%') AS mapper_call_count, \
         COUNT(*) FILTER (WHERE entry_slug LIKE '%util%') AS total_util_items \
         FROM items",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Tracing Status\n{tracing}\n\n"));

    // ── Per-kind utilization ──────────────────────────────────────────────────
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
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Per-Kind Utilization\n{utilization}\n\n"));

    // ── Deferred health ───────────────────────────────────────────────────────
    let deferred = execute_run_query(
        duckdb_path,
        "SELECT \
         ROUND(AVG(deferred.duration) / 1e6, 2) AS avg_deferred_ms, \
         ROUND(PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY deferred.duration) / 1e6, 2) AS p10_deferred_ms, \
         ROUND(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY deferred.duration) / 1e6, 2) AS p50_deferred_ms, \
         COUNT(*) FILTER (WHERE deferred.duration < 100000) AS items_under_100us \
         FROM items WHERE deferred IS NOT NULL AND deferred.duration IS NOT NULL",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {:?}}}]", e));
    out.push_str(&format!("## Deferred Health (runtime run-ahead)\n{deferred}\n\n"));

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
            "name": "read_code",
            "description":
                "Read an application source file (path relative to the configured code root). \
                 Use to understand task logic, mapper policies, and application structure.",
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

    tools
}
