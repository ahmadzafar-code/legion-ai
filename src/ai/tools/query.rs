//! `DuckDB` query execution + hardening, result formatting, and the SQL
//! builders used by the MCP surface.

/// Execute a read-only SQL query against the Legion `DuckDB` database.
///
/// Wraps the user's SQL with `DuckDB`'s `json_group_array(to_json(t))` to serialize
/// all column types (including STRUCTs like Interval and `ItemLink`) as JSON.
/// Execute a query and return the result as a markdown table (for LLM consumption).
/// Falls back to raw JSON if table formatting fails.
///
/// # Errors
/// Returns `Err` (a model-readable message) if the DuckDB file cannot be
/// opened read-only or the `SELECT` fails to execute.
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
/// Hard cap on rows returned by `run_query` (applied in Rust — trailing SQL
/// LIMITs are stripped, so the cap cannot be talked around by the model).
#[cfg(feature = "duckdb")]
const QUERY_ROW_CAP: usize = 50;

#[cfg(feature = "duckdb")]
pub(crate) fn mark_truncation_if_over(result: String) -> String {
    match serde_json::from_str::<Vec<serde_json::Value>>(&result) {
        Ok(mut arr) if arr.len() > QUERY_ROW_CAP => {
            arr.truncate(QUERY_ROW_CAP);
            arr.push(serde_json::json!({
                "_truncated": true,
                "_shown": QUERY_ROW_CAP,
                "_hint": "result capped at 50 rows; refine with aggregation or a narrower filter"
            }));
            serde_json::to_string(&arr).unwrap_or(result)
        }
        _ => result,
    }
}

/// Execute a query and return raw JSON array string.
/// Used internally by `gather_overview()` which parses the JSON itself.
///
/// # Errors
/// Returns `Err` if the DuckDB file cannot be opened read-only or the query
/// fails to execute.
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
        .map_err(|e| format!("Failed to open DuckDB '{duckdb_path}': {e}"))?;

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
            let mut msg = format!("Query failed: {err_str}\n");

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
                     parent_slug, type.",
                );
            } else if err_str.contains("Conversion Error") || err_str.contains("Could not convert")
            {
                msg.push_str(
                    "\nHINT: All timestamp fields are BIGINT nanoseconds. \
                     Use arithmetic: running.duration / 1e6 for milliseconds. \
                     Use CAST() for explicit type conversions.",
                );
            } else if err_str.contains("Binder Error") || err_str.contains("No function matches") {
                msg.push_str(
                    "\nHINT: Type mismatch. Column types: entry_slug is TEXT, \
                     item_uid is UBIGINT, title is TEXT. \
                     size is TEXT — a unit-suffixed string like '76.000 KiB' or '96 B' \
                     (may be NULL); NEVER SUM/CAST it directly. Parse unit-aware: \
                     CASE WHEN size LIKE '% KiB' THEN TRY_CAST(REPLACE(size,' KiB','') AS DOUBLE)*1024 \
                     WHEN size LIKE '% MiB' THEN TRY_CAST(REPLACE(size,' MiB','') AS DOUBLE)*1048576 \
                     WHEN size LIKE '% GiB' THEN TRY_CAST(REPLACE(size,' GiB','') AS DOUBLE)*1073741824 \
                     WHEN size LIKE '% B' THEN TRY_CAST(REPLACE(size,' B','') AS DOUBLE) END. \
                     STRUCT fields (running.duration, etc.) are BIGINT. \
                     You cannot SUM/AVG text columns. Use COUNT(*) for text, \
                     SUM()/AVG() only on numeric columns."
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
#[cfg(all(test, feature = "duckdb"))]
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

#[cfg(feature = "duckdb")]
/// Convert a JSON array of objects into a markdown table.
/// Returns None if the input is empty, not an array, or parse fails.
pub(crate) fn json_array_to_markdown_table(json_str: &str) -> Option<String> {
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
        out.push_str(&format!(" {col} |"));
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
            out.push_str(&format!(" {cell} |"));
        }
        out.push('\n');
    }

    Some(out)
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
                serde_json::Value::String(s) => {
                    serde_json::from_str::<Vec<serde_json::Value>>(s).ok()?
                }
                _ => return None,
            };
            Some(arr.iter().any(|x| x.as_str() == Some(slug)))
        })
        .unwrap_or(false)
}
