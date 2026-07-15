//! Tool implementations for Legion AI diagnosis: plain, transport-free Rust
//! functions, split by concern — `defs` (advertised JSON schemas), `query`
//! (hardened DuckDB execution), `overview` (pre-computed diagnostic signals),
//! `source` (sandboxed file tools), `wiki` (Legion knowledge corpus).
//!
//! TWO consumers share these functions and neither re-implements tool logic:
//! the MCP dispatch core (`mcp_core.rs`, serving Claude Code over stdio/HTTP —
//! the live path) and the built-in API loop (`agent.rs`, currently dormant).
//! In particular, every model-authored query funnels through
//! `query::execute_run_query_raw`.
//!
//! `run_query`/`gather_overview` need the `duckdb` feature; the file and wiki
//! tools need only `ai`.

mod defs;
mod overview;
mod query;
mod source;
mod wiki;

#[allow(unused_imports)]
pub use defs::*;
#[allow(unused_imports)]
pub use overview::*;
#[allow(unused_imports)]
pub use query::*;
#[allow(unused_imports)]
pub use source::*;
#[allow(unused_imports)]
pub use wiki::*;

#[cfg(all(test, feature = "duckdb"))]
mod tests {
    use super::query::mark_truncation_if_over;
    use super::*;

    /// Path to the shared bg4N2 test profile. It is an untracked fixture living in
    /// the repo root (one level above the crate dir), so resolve it relative to
    /// `CARGO_MANIFEST_DIR`. Tests soft-skip if it is absent on this machine.
    fn test_db_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb")
    }

    /// Path to the MiniAero 160³ fixture — the profile behind the one recorded
    /// WRONG live verdict ("under-sized mesh"). Large (172MB) and untracked;
    /// tests gate on its presence.
    fn miniaero_db_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/miniaero8_160cubed/prof.duckdb")
    }

    /// Data-Size Evidence on bg4N2: the unit-aware, item_uid-deduped count must
    /// equal the documented 207 distinct channel copies (pins BOTH the dedup and
    /// the unit parsing — a naive un-deduped count would be 235).
    #[test]
    fn test_data_size_evidence_bg4n2() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("bg4N2 fixture missing; skipping");
            return;
        }
        let out = execute_run_query_raw(db.to_str().unwrap(), DATA_SIZE_EVIDENCE_SQL).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        let row = rows.first().expect("one aggregate row");
        assert_eq!(
            row.get("sized_copies").and_then(|v| v.as_u64()),
            Some(207),
            "bg4N2 has exactly 207 distinct sized channel copies"
        );
        assert!(row.get("max_mib").and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.0);
    }

    /// MiniAero REGRESSION (the guardrail's reason to exist): the evidence that
    /// refutes "under-sized mesh" MUST surface — per-copy ghost exchanges of
    /// ~167–176 MiB and tens of GiB moved. If this data stops surfacing, the
    /// agent is back to guessing about sizes.
    #[test]
    fn test_data_size_evidence_miniaero_regression() {
        let db = miniaero_db_path();
        if !db.exists() {
            eprintln!("MiniAero fixture missing; skipping");
            return;
        }
        let db = db.to_str().unwrap().to_owned();

        let out = execute_run_query_raw(&db, DATA_SIZE_EVIDENCE_SQL).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        let row = rows.first().expect("one aggregate row");
        let max_mib = row.get("max_mib").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let total_mib = row.get("total_mib").and_then(|v| v.as_f64()).unwrap_or(0.0);
        assert!(
            max_mib > 150.0,
            "MiniAero's largest ghost-exchange copies (~175.8 MiB) must surface; got {max_mib}"
        );
        assert!(
            total_mib > 30_000.0,
            "MiniAero moves ~57 GiB (~58,000 MiB) total; got {total_mib}"
        );

        let tops = execute_run_query_raw(&db, DATA_SIZE_TOP_SQL).unwrap();
        assert!(
            tops.contains("175.8") && tops.contains("167.6"),
            "the two refuting per-copy sizes must appear in the top list: {tops}"
        );
    }

    /// Unit (no DB): the 50-row cap is marked only when MORE than 50 rows
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
        let fifty = serde_json::to_string(
            &(0..50)
                .map(|i| serde_json::json!({ "a": i }))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(mark_truncation_if_over(fifty.clone()), fifty);

        // 51 -> first 50 kept + ONE marker appended (51 elements total).
        let over = serde_json::to_string(
            &(0..51)
                .map(|i| serde_json::json!({ "a": i }))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let marked: Vec<serde_json::Value> =
            serde_json::from_str(&mark_truncation_if_over(over)).unwrap();
        assert_eq!(marked.len(), 51, "50 data + 1 marker");
        assert_eq!(
            marked[49]["a"],
            serde_json::json!(49),
            "last real row preserved"
        );
        assert_eq!(marked[50]["_truncated"], serde_json::json!(true));
        assert_eq!(marked[50]["_shown"], serde_json::json!(50));
    }

    /// Integration: a >50-row query is marked; a small one is not.
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
        assert!(
            arr[..50]
                .iter()
                .all(|r| r.get("entry_slug").is_some() && r.get("_truncated").is_none())
        );
        assert_eq!(arr[50]["_truncated"], serde_json::json!(true));
        assert_eq!(arr[50]["_shown"], serde_json::json!(50));

        // A small aggregate is returned UNCHANGED (no marker).
        let small = execute_run_query_raw(db, "SELECT COUNT(*) AS n FROM items").expect("query");
        assert!(
            !small.contains("_truncated"),
            "small result must NOT be marked: {small}"
        );
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
            assert!(
                out.contains(header),
                "kept signal section missing: {header}"
            );
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
        assert_eq!(
            rows.len(),
            1,
            "sample must be exactly one row, got {}",
            rows.len()
        );
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

    /// Exfil hardening: the read-only + `enable_external_access(false)` hardening must block
    /// table-function file reads (e.g. `read_text`) in a FROM clause, while benign
    /// SELECTs still work. The probe MUST use the FROM form: scalar `SELECT
    /// read_text(...)` raises a Binder Error regardless of hardening (false positive).
    #[test]
    fn test_run_query_blocks_external_file_read() {
        let db = test_db_path();
        if !db.exists() {
            eprintln!("skipping exfil test: test DB absent at {}", db.display());
            return;
        }
        let db = db.to_str().unwrap();

        // Benign query still works through the hardened connection.
        let ok = execute_run_query_raw(db, "SELECT COUNT(*) AS cnt FROM items")
            .expect("benign SELECT should succeed through hardened connection");
        assert!(
            ok.starts_with('['),
            "benign query should return a JSON array, got: {ok}"
        );
        assert!(
            ok.contains("cnt"),
            "benign query JSON missing alias, got: {ok}"
        );

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
            unhardened.query_row("SELECT content FROM read_text('/etc/hosts')", [], |r| {
                r.get(0)
            });
        assert!(
            leaked.is_ok(),
            "positive control: an unhardened connection should read the file, got {leaked:?}"
        );
    }

    /// Regression (duration dedup): the canonical per-`item_uid` dedup (`dedup_select_sql`) must yield
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
        assert!(
            close(lifetime_ms, 1546.0172),
            "lifetime_ms = {lifetime_ms}, want 1546.0172"
        );
        assert!(
            close(running_ms, 432.7334),
            "running_ms = {running_ms}, want 432.7334"
        );
        assert!(
            close(longest_ms, 13.1611),
            "longest_running_slice_ms = {longest_ms}, want 13.1611"
        );
        assert!(
            close(naive_ms, 808566.9726),
            "naive_ms = {naive_ms}, want 808566.9726"
        );
        // The dedup removes a ~523x inflation in the naive wall-clock estimate.
        assert!(
            (naive_ms / lifetime_ms - 523.0).abs() < 1.0,
            "inflation ratio = {}, want ~523",
            naive_ms / lifetime_ms
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// Cycle guard: the find_blockers critical-path walk must be cycle-guarded. Pins ROW
    /// COUNTS and the final uid (never the word "depth-N"). Uses a DIRECT
    /// connection — the unguarded variant's 100001 rows would be impossible
    /// through execute_run_query_raw's 50-row cap; mirrors the dedup test's
    /// writable temp-copy style.
    #[test]
    fn test_find_blockers_cycle_guard() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!(
                "skipping find_blockers: test DB absent at {}",
                src.display()
            );
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
                &format!(
                    "SELECT count(*), max(depth) FROM ({}) s",
                    find_blockers_sql(2220)
                ),
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
            eprintln!(
                "skipping unguarded runaway: test DB absent at {}",
                src.display()
            );
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
        assert_eq!(
            urows, 100001,
            "unguarded walk should run away to 100001 rows"
        );
        assert_eq!(
            umax, 100000,
            "unguarded walk should hit the 100000 depth cap"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// Regression (channel copies use `lifetime`, not `running`): a buggy overview
    /// query shape (`running IS NOT NULL` on `%chan%`) reports 0 copies / 0ms — a lie;
    /// the truth on bg4N2 is 207 distinct copies / 53.3ms via lifetime. Red→green
    /// on a DIRECT oracle connection, then asserts the real `gather_overview`
    /// (tool path) now surfaces the corrected numbers.
    #[test]
    fn test_channel_copy_lifetime_fix() {
        let src = test_db_path();
        if !src.exists() {
            eprintln!(
                "skipping channel-copy fix: test DB absent at {}",
                src.display()
            );
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
            assert_eq!(
                old_count, 0,
                "documents the bug: running-on-chan finds 0 copies"
            );

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
            assert!(
                (comm_ms - 53.3).abs() < 0.1,
                "corrected comm time, got {comm_ms}"
            );
        }

        // Tool path: the real gather_overview output must now surface the copies.
        let overview = gather_overview(db).expect("gather_overview");
        assert!(
            overview.contains("207 copies"),
            "overview should report 207 copies"
        );
        assert!(
            overview.contains("53.3"),
            "overview should report 53.3ms comm time"
        );
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

    /// Scope regression: "which task in [1.0s,1.5s] ran longest?" — scope matters. Uses a
    /// DETERMINISTIC per-slice argmax oracle (NOT `any_value` over multi-slice
    /// items, which is non-deterministic: uid 48 has ~33 running slices, uid 1 is a
    /// long-lived io thread). app-procs scope (cpu/gpudev) -> uid 48; all-items
    /// scope (no proc filter) -> uid 1. Asserts oracle == tool-path on both, and
    /// that the two scopes disagree.
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
        assert_ne!(
            oracle_app, oracle_all,
            "scope matters: app-scope != all-items"
        );

        // Tool path: same questions through execute_run_query_raw must agree.
        let tool_app = argmax_uid_by_dur(&execute_run_query_raw(db, app_sql).expect("tool app"));
        let tool_all = argmax_uid_by_dur(&execute_run_query_raw(db, all_sql).expect("tool all"));
        assert_eq!(tool_app, Some(oracle_app), "tool-path app == oracle");
        assert_eq!(tool_all, Some(oracle_all), "tool-path all == oracle");

        let _ = std::fs::remove_file(&tmp);
    }

    /// Compute- vs communication-bound in [1.8s,2.3s]: compute =
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
            let c = conn
                .query_row(compute_sql, [], |r| r.get(0))
                .expect("oracle compute");
            let m = conn
                .query_row(comm_sql, [], |r| r.get(0))
                .expect("oracle comm");
            (c, m)
        };
        assert!(
            (oracle_compute - 478.7).abs() < 0.1,
            "oracle compute, got {oracle_compute}"
        );
        assert!(
            (oracle_comm - 49.8).abs() < 0.1,
            "oracle comm, got {oracle_comm}"
        );
        assert!(
            oracle_compute > oracle_comm,
            "verdict computation-bound: {oracle_compute} > {oracle_comm}"
        );

        // Tool path: the comm query (lifetime-based) through execute_run_query_raw
        // must equal the oracle — proving the agent's copy path handles copies.
        let tool_comm = first_f64(
            &execute_run_query_raw(db, comm_sql).expect("tool comm"),
            "ms",
        )
        .expect("parse tool comm");
        assert!(
            (tool_comm - oracle_comm).abs() < 0.1,
            "tool-path comm parity: {tool_comm} vs oracle {oracle_comm}"
        );
        assert!(
            (tool_comm - 49.8).abs() < 0.1,
            "tool-path comm == 49.8, got {tool_comm}"
        );

        let _ = std::fs::remove_file(&tmp);
    }
}
