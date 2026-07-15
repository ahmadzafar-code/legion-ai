//! `gather_overview`: ~25 pre-computed diagnostic signal sections plus the
//! navigation anchors, built on top of `query::execute_run_query_raw`.

#[cfg(feature = "duckdb")]
use super::query::execute_run_query_raw;

/// Cycle-guarded critical-path walk from `start_uid`, enriched with corrected
/// (deduped) durations — the single source of truth for the `find_blockers` tool.
///
/// Walks `critical_path.item_uid` edges from `start_uid` toward the root blocker,
/// carrying a visited-uid `path` array. The `list_contains` guard plus the
/// `cycle` flag are MANDATORY: there is no self-loop, but real 2-cycles exist
/// (e.g. 2220↔1481), and `DuckDB`'s recursive UNION cannot dedup them because the
/// growing `path`/`depth` keeps rows distinct — a depth cap alone would walk
/// 100k+ rows. The depth cap (64) is a secondary backstop.
///
/// The enrichment join reuses the SAME dedup grain as `dedup_select_sql` (a
/// test-only helper that pins this contract)
/// (inner `DISTINCT (item_uid, entry_slug, lifetime, running, waiting)`, outer
/// `GROUP BY item_uid`) so the slice-row inflation cannot re-enter.
///
/// `start_uid` is a `u64` (never model text), so formatting it directly into the
/// two `CAST(... AS UBIGINT)` literals carries no injection surface. Returned
/// WITHOUT a trailing `;` so it composes as a subquery and passes the
/// `SELECT/WITH` prefix guard in [`execute_run_query_raw`].
///
/// Data-Size Evidence (`MiniAero` guardrail): distinct channel-copy sizes with
/// UNIT-AWARE parsing of the suffixed `size` strings ("96 B", "76.000 KiB",
/// "175.781 MiB", "1.2 GiB" — the column is TEXT, not numeric). Dedup by
/// `item_uid` (the 523× trap); `TRY_CAST` so a malformed size becomes NULL, not
/// an error. All numerics pre-computed in MiB/GiB (never raw bytes). Consts so
/// the regression tests run the EXACT SQL the overview runs.
#[cfg(feature = "duckdb")]
pub const DATA_SIZE_EVIDENCE_SQL: &str = "WITH sized AS ( \
  SELECT DISTINCT item_uid, size, \
    CASE \
      WHEN size LIKE '% GiB' THEN TRY_CAST(REPLACE(size, ' GiB', '') AS DOUBLE) * 1024.0 \
      WHEN size LIKE '% MiB' THEN TRY_CAST(REPLACE(size, ' MiB', '') AS DOUBLE) \
      WHEN size LIKE '% KiB' THEN TRY_CAST(REPLACE(size, ' KiB', '') AS DOUBLE) / 1024.0 \
      WHEN size LIKE '% B'   THEN TRY_CAST(REPLACE(size, ' B',   '') AS DOUBLE) / 1048576.0 \
    END AS mib \
  FROM items WHERE size IS NOT NULL AND entry_slug LIKE '%chan%' ) \
SELECT COUNT(*) AS sized_copies, \
  ROUND(MAX(mib), 1) AS max_mib, \
  ROUND(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY mib), 3) AS p50_mib, \
  ROUND(SUM(mib), 2) AS total_mib \
FROM sized WHERE mib IS NOT NULL";

/// Companion to [`DATA_SIZE_EVIDENCE_SQL`]: the 3 largest DISTINCT copy sizes
/// with counts — the per-copy figures a sizing verdict must reconcile against
/// (e.g. `MiniAero`'s 175.8 MiB ×56 ghost exchanges refute "under-sized mesh").
#[cfg(feature = "duckdb")]
pub const DATA_SIZE_TOP_SQL: &str = "WITH sized AS ( \
  SELECT DISTINCT item_uid, size, \
    CASE \
      WHEN size LIKE '% GiB' THEN TRY_CAST(REPLACE(size, ' GiB', '') AS DOUBLE) * 1024.0 \
      WHEN size LIKE '% MiB' THEN TRY_CAST(REPLACE(size, ' MiB', '') AS DOUBLE) \
      WHEN size LIKE '% KiB' THEN TRY_CAST(REPLACE(size, ' KiB', '') AS DOUBLE) / 1024.0 \
      WHEN size LIKE '% B'   THEN TRY_CAST(REPLACE(size, ' B',   '') AS DOUBLE) / 1048576.0 \
    END AS mib \
  FROM items WHERE size IS NOT NULL AND entry_slug LIKE '%chan%' ) \
SELECT ROUND(mib, 1) AS mib, COUNT(*) AS copies \
FROM sized WHERE mib IS NOT NULL \
GROUP BY ROUND(mib, 1) ORDER BY mib DESC";

/// Gather a pre-computed overview of the profiling database.
///
/// Runs several SQL queries and combines results into a structured text summary
/// (~4–8 KB) suitable for the agent's initial context message. Each section is
/// appended by one `overview_*` helper below, in the order the agent reads them.
#[cfg(feature = "duckdb")]
pub fn gather_overview(duckdb_path: &str) -> Result<String, String> {
    let mut out = String::with_capacity(8192);
    overview_schema(&mut out);
    overview_row_counts(duckdb_path, &mut out);
    overview_processor_hierarchy(duckdb_path, &mut out);
    overview_timeline_bounds(duckdb_path, &mut out);
    overview_task_distribution(duckdb_path, &mut out);
    overview_slots_by_kind(duckdb_path, &mut out);
    overview_sample_item(duckdb_path, &mut out);
    overview_profile_classification(duckdb_path, &mut out);
    overview_tracing_status(duckdb_path, &mut out);
    overview_per_kind_utilization(duckdb_path, &mut out);
    overview_deferred_health(duckdb_path, &mut out);
    overview_utility_breakdown(duckdb_path, &mut out);
    overview_mapper_calls(duckdb_path, &mut out);
    overview_task_granularity(duckdb_path, &mut out);
    overview_channel_copies(duckdb_path, &mut out);
    overview_data_size_evidence(duckdb_path, &mut out);
    overview_delayed_distribution(duckdb_path, &mut out);
    overview_triggering_latency(duckdb_path, &mut out);
    overview_python_detection(duckdb_path, &mut out);
    overview_gc_instance_activity(duckdb_path, &mut out);
    overview_node_utility_balance(duckdb_path, &mut out);
    overview_channel_direction(duckdb_path, &mut out);
    overview_copy_compute_ratio(duckdb_path, &mut out);
    overview_scheduling_overhead(duckdb_path, &mut out);
    overview_processor_balance(duckdb_path, &mut out);
    overview_navigation_anchors(duckdb_path, &mut out);
    Ok(out)
}

/// Overview section: Schema.
#[cfg(feature = "duckdb")]
fn overview_schema(out: &mut String) {
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
         size: TEXT — unit-suffixed string ('76.000 KiB', '96 B'); parse units before math \
         (see run_query example #7), never CAST directly.\n\
         All timestamps are NANOSECONDS. Access STRUCTs with dot notation: \
         running.start, critical_path.item_uid.\n\n",
    );
}

/// Overview section: Row counts.
#[cfg(feature = "duckdb")]
fn overview_row_counts(duckdb_path: &str, out: &mut String) {
    let entry_count = execute_run_query_raw(duckdb_path, "SELECT COUNT(*) AS cnt FROM entries")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    let item_count = execute_run_query_raw(duckdb_path, "SELECT COUNT(*) AS cnt FROM items")
        .unwrap_or_else(|_| "[{\"cnt\":\"?\"}]".into());
    out.push_str(&format!(
        "## Row Counts\nentries: {entry_count}  items: {item_count}\n\n"
    ));
}

/// Overview section: Processor hierarchy.
#[cfg(feature = "duckdb")]
fn overview_processor_hierarchy(duckdb_path: &str, out: &mut String) {
    let hier = execute_run_query_raw(
        duckdb_path,
        "SELECT parent_slug, type, COUNT(*) AS cnt, \
         STRING_AGG(entry_slug, ', ' ORDER BY entry_slug) AS slugs \
         FROM entries GROUP BY parent_slug, type ORDER BY parent_slug, type",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {e:?}}}]"));
    out.push_str(&format!("## Processor Hierarchy\n{hier}\n\n"));
}

/// Overview section: Timeline bounds.
#[cfg(feature = "duckdb")]
fn overview_timeline_bounds(duckdb_path: &str, out: &mut String) {
    let bounds = execute_run_query_raw(
        duckdb_path,
        "SELECT MIN(lifetime.start) AS earliest_ns, MAX(lifetime.stop) AS latest_ns, \
         ROUND((MAX(lifetime.stop) - MIN(lifetime.start)) / 1e6, 1) AS span_ms FROM items",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {e:?}}}]"));
    out.push_str(&format!("## Timeline Bounds\n{bounds}\n\n"));
}

/// Overview section: Task distribution.
///
/// Top-10 headline — orientation, not an exhaustive distribution; the
/// agent uses `run_query` for the full GROUP BY when it needs it. The LIMIT is
/// wrapped in a subquery because `execute_run_query_raw` strips a TRAILING
/// `LIMIT n` and re-applies its own 50-row cap — so an un-wrapped `LIMIT 10`
/// would still return up to 50 task types.
#[cfg(feature = "duckdb")]
fn overview_task_distribution(duckdb_path: &str, out: &mut String) {
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
    .unwrap_or_else(|e| format!("[{{\"error\": {e:?}}}]"));
    out.push_str(&format!("## Top Task Types (by count, top 10)\n{dist}\n\n"));
}

/// Overview section: Slot counts by kind.
#[cfg(feature = "duckdb")]
fn overview_slots_by_kind(duckdb_path: &str, out: &mut String) {
    let slots = execute_run_query_raw(
        duckdb_path,
        "SELECT parent_slug, COUNT(*) AS slot_cnt FROM entries WHERE type = 'slot' \
         GROUP BY parent_slug ORDER BY parent_slug",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {e:?}}}]"));
    out.push_str(&format!("## Slots by Kind\n{slots}\n\n"));
}

/// Overview section: Sample item (compact).
///
/// A `SELECT *` here dumps every lifecycle + cross-ref STRUCT — ~63 KB on
/// bg4N2, enough to overflow the MCP tool-result budget on its own.
/// The Schema section already lists the columns; a 4-column projection still
/// shows the populated STRUCT SHAPE (a lifecycle struct + a cross-ref struct)
/// without the dump. Full rows are one `run_query` away.
/// The inner LIMIT 1 is wrapped in a subquery: `execute_run_query_raw` strips a
/// TRAILING `LIMIT n` and re-applies its 50-row cap, so a bare `... LIMIT 1`
/// would return 50 FULL rows (the ~63 KB dump).
#[cfg(feature = "duckdb")]
fn overview_sample_item(duckdb_path: &str, out: &mut String) {
    let sample = execute_run_query_raw(
        duckdb_path,
        "SELECT item_uid, title, running, critical_path FROM (\
           SELECT * FROM items WHERE running IS NOT NULL LIMIT 1\
         ) s",
    )
    .unwrap_or_else(|e| format!("[{{\"error\": {e:?}}}]"));
    out.push_str(&format!(
        "## Sample Item Row (shape; SELECT * via run_query)\n{sample}\n\n"
    ));
}

/// Overview section: Profile classification (human-readable).
#[cfg(feature = "duckdb")]
fn overview_profile_classification(duckdb_path: &str, out: &mut String) {
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
                    let gpu = row
                        .get("gpu_device_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let cpu = row
                        .get("cpu_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let util = row
                        .get("util_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let nodes = row
                        .get("node_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(1);
                    let profile_type = if gpu > 0 { "GPU-present" } else { "CPU-only" };
                    let node_str = if nodes <= 1 {
                        "single-node".to_string()
                    } else {
                        format!("{nodes}-node")
                    };
                    out.push_str(&format!(
                        "- Type: {profile_type} {node_str}\n- GPUs: {gpu} | CPUs: {cpu} | Utility procs: {util}\n"
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Tracing detection (human-readable).
#[cfg(feature = "duckdb")]
fn overview_tracing_status(duckdb_path: &str, out: &mut String) {
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
                    let rpt = row
                        .get("replay_trace_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let mapper = row
                        .get("mapper_call_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "- Replay Physical Trace tasks: {rpt}\n\
                         - Mapper calls: {mapper}\n"
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Per-kind utilization (human-readable).
#[cfg(feature = "duckdb")]
fn overview_per_kind_utilization(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("proc_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let avg = row
                        .get("avg_util_pct")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let max = row
                        .get("max_util_pct")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {kind}: {count} proc(s), avg {avg:.1}% util, max {max:.1}%\n"
                    ));
                }
                if parsed.is_empty() {
                    out.push_str("(no utilization data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Deferred health (human-readable).
#[cfg(feature = "duckdb")]
fn overview_deferred_health(duckdb_path: &str, out: &mut String) {
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
                    let avg = row
                        .get("avg_deferred_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let p10 = row
                        .get("p10_deferred_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let p50 = row
                        .get("p50_deferred_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let under_100us = row
                        .get("items_under_100us")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "- Avg: {avg:.2}ms | P10: {p10:.2}ms | P50: {p50:.2}ms | Items <100us: {under_100us}\n"
                    ));
                } else {
                    out.push_str("(no deferred data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Utility meta-task breakdown.
#[cfg(feature = "duckdb")]
fn overview_utility_breakdown(duckdb_path: &str, out: &mut String) {
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
                    let ms = row
                        .get("total_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let pct = row
                        .get("pct")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let label = match cat {
                        "analysis" => "Analysis (dependence/disjointness)",
                        "mapper" => "Mapper calls",
                        "trace_replay" => "Trace replay",
                        "scheduling" => "Scheduling (scheduler/prepipeline)",
                        _ => "Other meta-tasks",
                    };
                    out.push_str(&format!("- {label}: {pct:.1}% ({ms:.1}ms)\n"));
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
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Mapper call analysis.
#[cfg(feature = "duckdb")]
fn overview_mapper_calls(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("call_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let avg = row
                        .get("avg_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let p95 = row
                        .get("p95_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let max = row
                        .get("max_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {count} mapper calls | avg: {avg:.2}ms | P95: {p95:.2}ms | max: {max:.2}ms\n"
                    ));
                    if max > 10.0 {
                        out.push_str(
                            "- ANOMALOUS — individual mapper calls >10ms, \
                             possible OS descheduling or expensive mapper logic\n",
                        );
                    }
                    if count == 0 {
                        out.push_str(
                            "- No mapper calls found (tracing may be handling all mapping)\n",
                        );
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Task granularity.
#[cfg(feature = "duckdb")]
fn overview_task_granularity(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("app_task_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let avg = row
                        .get("avg_run_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let min = row
                        .get("min_run_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let median = row
                        .get("median_run_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {count} app tasks | median: {median:.3}ms | avg: {avg:.3}ms | min: {min:.3}ms\n"
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Channel copy patterns.
///
/// Copies live on %chan% slots and carry their cost in `lifetime` + `size`,
/// NEVER `running` (title = "Copy") — filtering on `running` reports zero
/// copies and zero comm time. Dedup by `item_uid`: a multi-hop copy appears
/// on several chan slots but must be counted once (the regression test pins
/// the numbers). Byte volume is intentionally omitted here: `size` is a
/// unit-suffixed TEXT column, and summing it across hops double-counts
/// multi-hop copies — the "Data-Size Evidence" section reports volume with
/// the unit-aware, deduplicated query instead.
#[cfg(feature = "duckdb")]
fn overview_channel_copies(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("copy_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let ms = row
                        .get("total_copy_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {count} copies | total comm time (lifetime): {ms:.1}ms\n"
                    ));
                    // Volume lives in "Data-Size Evidence" below (unit-aware
                    // parsing of the suffixed `size` strings — B/KiB/MiB/GiB).
                    if count == 0 {
                        out.push_str("- No channel copies (CPU-only or no data movement)\n");
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Data-Size Evidence (`MiniAero` guardrail: verify sizing verdicts vs DATA).
///
/// Root cause of the one recorded WRONG live verdict ("under-sized mesh, grow
/// it" on `MiniAero` 160³): the agent had 167–176 MiB ghost-exchange copies in
/// front of it — evidence that the mesh was already large — and never
/// reconciled. This section makes that evidence one glance away and carries
/// the reconcile instruction AT the evidence (result-level reminder, the
/// proven redundancy lever). `size` is a unit-suffixed STRING ("76.000 KiB",
/// "175.781 MiB"), so parsing is unit-aware; dedup by `item_uid` as always.
#[cfg(feature = "duckdb")]
fn overview_data_size_evidence(duckdb_path: &str, out: &mut String) {
    let evidence = execute_run_query_raw(duckdb_path, DATA_SIZE_EVIDENCE_SQL);
    let top_sizes = execute_run_query_raw(duckdb_path, DATA_SIZE_TOP_SQL);
    out.push_str("## Data-Size Evidence (sizing verdicts)\n");
    match &evidence {
        Ok(json_str) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
                if let Some(row) = parsed.first() {
                    let n = row
                        .get("sized_copies")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if n == 0 {
                        out.push_str("- no sized channel copies in this profile\n");
                    } else {
                        let max = row
                            .get("max_mib")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);
                        let p50 = row
                            .get("p50_mib")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);
                        let mib = row
                            .get("total_mib")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);
                        // Adaptive units so small totals stay PRECISE (channel
                        // volume on bg4N2 is 6.73 MiB — "0.01 GiB" is useless).
                        let total = if mib >= 1024.0 {
                            format!("{:.2} GiB", mib / 1024.0)
                        } else {
                            format!("{mib:.2} MiB")
                        };
                        out.push_str(&format!(
                            "- {n} sized copies (channels only — instances/fills also carry `size`; don't mix) | max: {max:.1} MiB | p50: {p50:.3} MiB | total moved: {total}\n"
                        ));
                        if let Ok(Ok(tops)) = top_sizes
                            .as_ref()
                            .map(|s| serde_json::from_str::<Vec<serde_json::Value>>(s))
                        {
                            // Top-3 cap lives HERE: execute_run_query_raw strips
                            // trailing LIMITs, so the SQL can't carry it.
                            let line = tops
                                .iter()
                                .take(3)
                                .filter_map(|r| {
                                    let mib = r.get("mib").and_then(serde_json::Value::as_f64)?;
                                    let c = r.get("copies").and_then(serde_json::Value::as_u64)?;
                                    Some(format!("{mib:.1} MiB ×{c}"))
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            if !line.is_empty() {
                                out.push_str(&format!("- largest distinct copy sizes: {line}\n"));
                            }
                        }
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push_str(
        "- GUARDRAIL: before ANY under-/over-sized verdict (mesh, problem size, \
         instances), derive the observed size from THIS data and reconcile — e.g. \
         per-copy ghost-exchange size scales with the mesh. If the observed sizes \
         contradict the hypothesis, say so instead of asserting it.\n",
    );
    out.push('\n');
}

/// Overview section: Delayed distribution (Realm pickup latency).
#[cfg(feature = "duckdb")]
fn overview_delayed_distribution(duckdb_path: &str, out: &mut String) {
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
                    let p50 = row
                        .get("p50_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let p90 = row
                        .get("p90_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let max = row
                        .get("max_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let over_1ms = row
                        .get("items_over_1ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "- P50: {p50:.3}ms | P90: {p90:.3}ms | max: {max:.2}ms | items >1ms: {over_1ms}\n"
                    ));
                } else {
                    out.push_str("(no delayed data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Triggering latency.
#[cfg(feature = "duckdb")]
fn overview_triggering_latency(duckdb_path: &str, out: &mut String) {
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
                    let p90 = row
                        .get("p90_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let max = row
                        .get("max_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let over_1ms = row
                        .get("items_over_1ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "- P90: {p90:.3}ms | max: {max:.2}ms | items >1ms: {over_1ms}\n"
                    ));
                } else {
                    out.push_str("(no triggering latency data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: Python/Legate detection.
#[cfg(feature = "duckdb")]
fn overview_python_detection(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("py_proc_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if count > 0 {
                        out.push_str(&format!(
                            "- Python processors: {count} (Legate/cuNumeric)\n"
                        ));
                    } else {
                        out.push_str("- Python processors: 0\n");
                    }
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => out.push_str(&format!("(error: {e})\n")),
    }
    out.push('\n');
}

/// Overview section: GC and instance activity.
#[cfg(feature = "duckdb")]
fn overview_gc_instance_activity(duckdb_path: &str, out: &mut String) {
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
                    let gc_count = row
                        .get("gc_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let gc_ms = row
                        .get("gc_total_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let inst = row
                        .get("instance_items")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if gc_count > 0 {
                        out.push_str(&format!(
                            "- GC activity detected: {gc_count} events, {gc_ms:.1}ms — check for memory pressure\n"
                        ));
                    } else {
                        out.push_str("- No GC activity detected\n");
                    }
                    out.push_str(&format!("- Instance-related items: {inst}\n"));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Per-node utility balance.
#[cfg(feature = "duckdb")]
fn overview_node_utility_balance(duckdb_path: &str, out: &mut String) {
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
                        let ms = row
                            .get("total_busy_ms")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);
                        out.push_str(&format!("- {node}: {ms:.1}ms utility busy\n"));
                    }
                } else {
                    let mut min_ms = f64::MAX;
                    let mut max_ms = 0.0_f64;
                    let mut min_node = "?".to_string();
                    let mut max_node = "?".to_string();
                    for row in &parsed {
                        let node = row.get("node").and_then(|v| v.as_str()).unwrap_or("?");
                        let ms = row
                            .get("total_busy_ms")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);
                        out.push_str(&format!("- {node}: {ms:.1}ms utility busy\n"));
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
                        "- Utility-work spread: {ratio:.1}x (busiest {max_node}, lightest {min_node})\n"
                    ));
                }
                if parsed.is_empty() {
                    out.push_str("(no utility data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Channel direction analysis.
///
/// Per-channel comm activity. Copies use `lifetime` (not `running`); dedup by
/// `item_uid`, assigning each copy to its `min(entry_slug)` so the per-channel
/// total matches the whole-run copy figure. Byte volume is reported by the
/// "Data-Size Evidence" section instead (unit-aware, deduplicated).
#[cfg(feature = "duckdb")]
fn overview_channel_direction(duckdb_path: &str, out: &mut String) {
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
                        let slug = row
                            .get("entry_slug")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let copies = row
                            .get("copy_count")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let ms = row
                            .get("total_ms")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0);

                        // Classify channel direction from slug
                        let direction = classify_channel_slug(slug);
                        if direction.contains("PCIe") {
                            has_pcie = true;
                        }
                        if direction.contains("inter-node") {
                            has_inter_node = true;
                        }

                        out.push_str(&format!(
                            "- {slug} [{direction}]: {copies} copies, {ms:.1}ms\n"
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
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Copy-to-compute ratio.
///
/// Copy time = SUM(lifetime.duration) on channels, dedup'd by `item_uid`: copies
/// have no `running`, and a copy's rows are the SAME transfer on 2 channel
/// slugs sharing ONE lifetime — true duplication, so dedup is required.
/// Compute time keeps the NAIVE
/// `SUM(running.duration)` on cpu/gpu non-util (2263.1ms on bg4N2): compute
/// items repeat as genuine RE-EXECUTIONS (distinct running slices), so the raw
/// sum is correct and collapsing per `item_uid` would wrongly drop them.
#[cfg(feature = "duckdb")]
fn overview_copy_compute_ratio(duckdb_path: &str, out: &mut String) {
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
                    let copy_ms = row
                        .get("copy_total_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let compute_ms = row
                        .get("compute_total_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let pct = row
                        .get("copy_pct")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- Copy: {copy_ms:.1}ms | Compute: {compute_ms:.1}ms | Copy fraction: {pct:.1}%\n"
                    ));
                } else {
                    out.push_str("(no data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Scheduling overhead.
#[cfg(feature = "duckdb")]
fn overview_scheduling_overhead(duckdb_path: &str, out: &mut String) {
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
                    let p90 = row
                        .get("p90_overhead_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let avg = row
                        .get("avg_overhead_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let count = row
                        .get("items_with_overhead")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "- P90: {p90:.2}ms | Avg: {avg:.2}ms ({count} items)\n"
                    ));
                } else {
                    out.push_str("(no scheduling overhead data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Application processor balance.
#[cfg(feature = "duckdb")]
fn overview_processor_balance(duckdb_path: &str, out: &mut String) {
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
                    let count = row
                        .get("proc_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let min_u = row
                        .get("min_util")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let max_u = row
                        .get("max_util")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let avg_u = row
                        .get("avg_util")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    out.push_str(&format!(
                        "- {kind}: {count} procs, min {min_u:.1}%, max {max_u:.1}%, avg {avg_u:.1}%\n"
                    ));
                    let ratio = max_u / min_u.max(0.1);
                    out.push_str(&format!("  spread: {ratio:.1}x (max/min)\n"));
                }
                if parsed.is_empty() {
                    out.push_str("(no application processor data)\n");
                }
            } else {
                out.push_str(&format!("{json_str}\n"));
            }
        }
        Err(e) => {
            if e.contains("not found") {
                out.push_str("Not available in this profile\n");
            } else {
                out.push_str(&format!("(error: {e})\n"));
            }
        }
    }
    out.push('\n');
}

/// Overview section: Navigation anchors.
#[cfg(feature = "duckdb")]
fn overview_navigation_anchors(duckdb_path: &str, out: &mut String) {
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
                    let start = row
                        .get("steady_start")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let end = row
                        .get("steady_end")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    if start > 0 && end > start {
                        out.push_str(&format!(
                            "- Steady-state zoom (middle 20%%): [{start}, {end}]\n"
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (midpoint error: {e})\n"));
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
                    let slug = row
                        .get("entry_slug")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let ms = row
                        .get("duration_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let start = row
                        .get("start_ns")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let stop = row
                        .get("stop_ns")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    if ms > 0.0 {
                        out.push_str(&format!(
                            "- Longest mapper call: {ms:.2}ms at [{start}, {stop}] on {slug}\n"
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (mapper anchor error: {e})\n"));
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
                    let slug = row
                        .get("entry_slug")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let ms = row
                        .get("gap_ms")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0);
                    let start = row
                        .get("gap_start_ns")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let end = row
                        .get("gap_end_ns")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    if ms > 0.0 {
                        out.push_str(&format!(
                            "- Largest app processor gap: {ms:.2}ms at [{start}, {end}] on {slug}\n"
                        ));
                    }
                }
            }
        }
        Err(e) => {
            if !e.contains("not found") {
                out.push_str(&format!("  (gap anchor error: {e})\n"));
            }
        }
    }

    out.push_str("Use zoom_to or set_view with these nanosecond ranges to navigate directly.\n");
    out.push('\n');
}

#[cfg(feature = "duckdb")]
/// Classify a channel `entry_slug` into a direction label.
///
/// Best-effort parsing:
/// - Two different node prefixes (e.g. "n0" and "n1") → "inter-node"
/// - Contains both 's' and 'f' components (system mem and framebuffer) → "SYS↔FB (`PCIe`)"
/// - Otherwise → "local"
fn classify_channel_slug(slug: &str) -> &'static str {
    // Extract the part after "chan_" (e.g. "n0s0_n1s0" or "fn0s0")
    let chan_part = slug.find("chan_").map(|i| &slug[i + 4..]).unwrap_or(slug);

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
