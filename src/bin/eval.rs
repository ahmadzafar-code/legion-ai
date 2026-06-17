//! `eval run` — oracle-graded tool-correctness eval (Rung 0).
//!
//! Drives a natural-language question through Claude Code over the P1.1 `mcp`
//! server, captures the agent's `final_answer`, computes the ground truth
//! INDEPENDENTLY, and grades programmatically.
//!
//! ## The one principle: ORACLE INDEPENDENCE
//! The ground truth is computed by running the case's `oracle_sql` on a DIRECT,
//! SEPARATE `duckdb::Connection` — NEVER through `execute_run_query_raw` or the
//! MCP. This file deliberately does NOT import `legion_prof_viewer`: the oracle
//! path and the agent path share no code, so a tool bug cannot corrupt both. The
//! `oracle_sql` is author-trusted (in the manifest), so it runs on a plain direct
//! connection — the anti-exfil hardening is only for model-authored SQL.
//!
//! ## Structure
//! The deterministic core (load_case / verify_sha / compute_oracle / grade /
//! build_result_row) is pure and unit-tested WITHOUT Claude Code. The
//! non-deterministic LLM driver is isolated behind the `Harness` trait
//! (`StubHarness` for tests, `McpHarness` shelling out to `claude`).
//!
//! Usage: `eval run --case <DIR_or_ID> --harness <mcp|embedded> --seed <N>
//!         [--out <path.jsonl>] [--model <id>]`

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

// ── Case manifest (§4) ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Case {
    case_id: String,
    question: String,
    duckdb_relpath: String,
    sha256: String,
    range_ns: Option<(i64, i64)>,
    answer_type: String,
    tolerance: f64,
    oracle_sql: String,
    expected: Option<String>,
    model: Option<String>,
    case_dir: PathBuf,
}

impl Case {
    /// Absolute path to the case's `.duckdb`, resolved relative to `case.toml`.
    fn duckdb_abs_path(&self) -> PathBuf {
        self.case_dir.join(&self.duckdb_relpath)
    }
}

/// Resolve `--case` (a dir, a `case.toml` path, or an id under
/// `eval_fixtures/`), parse it, and return the `Case`.
fn load_case(arg: &str) -> Result<Case, String> {
    let candidate = Path::new(arg);
    let case_dir = if candidate.join("case.toml").is_file() {
        candidate.to_path_buf()
    } else if candidate.is_file() && candidate.file_name() == Some("case.toml".as_ref()) {
        candidate.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("eval_fixtures").join(arg)
    };
    let toml_path = case_dir.join("case.toml");
    let content = std::fs::read_to_string(&toml_path)
        .map_err(|e| format!("read {}: {e}", toml_path.display()))?;
    let map = parse_toml(&content)?;

    Ok(Case {
        case_id: get_str(&map, "case_id").ok_or("missing case_id")?,
        question: get_str(&map, "question").unwrap_or_default(),
        duckdb_relpath: get_str(&map, "duckdb_relpath").ok_or("missing duckdb_relpath")?,
        sha256: get_str(&map, "sha256").ok_or("missing sha256")?,
        range_ns: get_int_pair(&map, "range_ns"),
        answer_type: get_str(&map, "answer_type").ok_or("missing answer_type")?,
        tolerance: get_f64(&map, "tolerance").unwrap_or(0.0),
        oracle_sql: map
            .get("oracle_sql")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or("missing oracle_sql")?,
        expected: get_str(&map, "expected"),
        model: get_str(&map, "model"),
        case_dir,
    })
}

/// Minimal TOML reader for the flat case manifest: `key = scalar`, quoted strings,
/// `[a, b]` int pairs, `#` comments, and triple-quoted `"""…"""` multi-line
/// strings. Raw values are stored verbatim; the `get_*` helpers type them.
fn parse_toml(content: &str) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut lines = content.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq].trim().to_string();
        let rest = trimmed[eq + 1..].trim();

        if let Some(after_open) = rest.strip_prefix("\"\"\"") {
            // Multi-line string until the closing """.
            let mut body = String::new();
            if let Some(end) = after_open.find("\"\"\"") {
                body.push_str(&after_open[..end]);
            } else {
                if !after_open.is_empty() {
                    body.push_str(after_open);
                    body.push('\n');
                }
                loop {
                    let Some(l) = lines.next() else {
                        return Err(format!("unterminated \"\"\" for key {key}"));
                    };
                    if let Some(end) = l.find("\"\"\"") {
                        body.push_str(&l[..end]);
                        break;
                    }
                    body.push_str(l);
                    body.push('\n');
                }
            }
            map.insert(key, body);
        } else {
            map.insert(key, rest.to_string());
        }
    }
    Ok(map)
}

/// Extract a quoted string value (text between the first pair of `"`).
fn get_str(map: &HashMap<String, String>, key: &str) -> Option<String> {
    let v = map.get(key)?;
    let start = v.find('"')?;
    let after = &v[start + 1..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// A bare scalar token with any trailing `# comment` and whitespace stripped.
fn raw_scalar(map: &HashMap<String, String>, key: &str) -> Option<String> {
    let v = map.get(key)?;
    Some(v.split('#').next().unwrap_or("").trim().to_string())
}

fn get_f64(map: &HashMap<String, String>, key: &str) -> Option<f64> {
    raw_scalar(map, key)?.parse().ok()
}

fn get_int_pair(map: &HashMap<String, String>, key: &str) -> Option<(i64, i64)> {
    let raw = raw_scalar(map, key)?;
    let inner = raw.trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<i64> = inner.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    (parts.len() == 2).then(|| (parts[0], parts[1]))
}

// ── SHA verification ─────────────────────────────────────────────────────────

/// `sha256` of the case's `.duckdb` bytes must equal the manifest's. Hashes via
/// `std::fs::read` + sha2 (no shelled `shasum`, no duckdb connection).
fn verify_sha(case: &Case) -> Result<(), String> {
    let path = case.duckdb_abs_path();
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let got = hex(&Sha256::digest(&bytes));
    if got.eq_ignore_ascii_case(&case.sha256) {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch for {}: manifest {} != file {}",
            case.case_id, case.sha256, got
        ))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ── Oracle (independent ground truth on a DIRECT connection) ─────────────────

/// Open a DIRECT read-only connection to the case DB. Trusted SQL → no anti-exfil
/// hardening; read-only avoids mutating the shared fixture.
fn open_oracle_conn(path: &Path) -> Result<duckdb::Connection, String> {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadOnly)
        .map_err(|e| format!("oracle config: {e}"))?;
    duckdb::Connection::open_with_flags(path, config)
        .map_err(|e| format!("open oracle DB {}: {e}", path.display()))
}

/// Convert a result cell to its canonical string form (robust across DuckDB
/// integer/float/text types). `None` for NULL/unhandled.
fn cell_to_string(v: duckdb::types::ValueRef<'_>) -> Option<String> {
    use duckdb::types::ValueRef;
    Some(match v {
        ValueRef::Null => return None,
        ValueRef::Boolean(b) => b.to_string(),
        ValueRef::TinyInt(i) => i.to_string(),
        ValueRef::SmallInt(i) => i.to_string(),
        ValueRef::Int(i) => i.to_string(),
        ValueRef::BigInt(i) => i.to_string(),
        ValueRef::HugeInt(i) => i.to_string(),
        ValueRef::UTinyInt(i) => i.to_string(),
        ValueRef::USmallInt(i) => i.to_string(),
        ValueRef::UInt(i) => i.to_string(),
        ValueRef::UBigInt(i) => i.to_string(),
        ValueRef::Float(f) => f.to_string(),
        ValueRef::Double(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
        _ => return None,
    })
}

/// Run `case.oracle_sql` VERBATIM on a direct connection → the ground-truth
/// value. If `case.expected` is present, asserts ground_truth == expected and
/// FAILS LOUD on mismatch (catches oracle/fixture drift before grading).
fn compute_oracle(case: &Case) -> Result<String, String> {
    let conn = open_oracle_conn(&case.duckdb_abs_path())?;
    let value = if case.answer_type == "set" {
        let mut stmt = conn
            .prepare(&case.oracle_sql)
            .map_err(|e| format!("oracle prepare: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok(cell_to_string(row.get_ref(0)?)))
            .map_err(|e| format!("oracle query: {e}"))?;
        let mut vals: Vec<String> = Vec::new();
        for r in rows {
            if let Some(s) = r.map_err(|e| format!("oracle row: {e}"))? {
                vals.push(s);
            }
        }
        vals.sort();
        vals.join(",")
    } else {
        conn.query_row(&case.oracle_sql, [], |row| Ok(cell_to_string(row.get_ref(0)?)))
            .map_err(|e| format!("oracle query: {e}"))?
            .ok_or("oracle returned NULL")?
    };

    if let Some(expected) = &case.expected {
        let g = grade(&case.answer_type, &value, expected, case.tolerance);
        if !g.pass {
            return Err(format!(
                "ORACLE DRIFT: case {} computed {value:?} but manifest expected {expected:?}",
                case.case_id
            ));
        }
    }
    Ok(value)
}

// ── Grader (§7) ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Grade {
    pass: bool,
    divergence: Option<Value>,
}

fn to_set(s: &str) -> BTreeSet<String> {
    s.split(',').map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect()
}

/// Parse a uid tolerantly: accepts `"48"`, `" 48 "`, and integer-valued floats
/// like `"48.0"` (an LLM may emit the value as a JSON float). Rejects
/// non-integers (`"48.5"`, `"abc"`). The oracle uid is always a clean integer, so
/// this only ever rescues a false-negative — it can never create a false-positive.
fn parse_uid(s: &str) -> Option<i64> {
    let t = s.trim();
    if let Ok(i) = t.parse::<i64>() {
        return Some(i);
    }
    let f = t.parse::<f64>().ok()?;
    (f.is_finite() && f.fract() == 0.0).then_some(f as i64)
}

/// Grade `agent` vs `oracle` by `answer_type`, normalizing both sides.
fn grade(answer_type: &str, agent: &str, oracle: &str, tolerance: f64) -> Grade {
    let pass = match answer_type {
        // `uid` = an item identifier, `int` = a count; both grade as exact integer
        // equality (tolerant of an integer-valued float, e.g. an LLM's "6.0").
        "uid" | "int" => {
            let a = parse_uid(agent);
            a.is_some() && a == parse_uid(oracle)
        }
        "label" => agent.trim().eq_ignore_ascii_case(oracle.trim()),
        "number" => match (agent.trim().parse::<f64>(), oracle.trim().parse::<f64>()) {
            (Ok(a), Ok(o)) => (a - o).abs() / o.abs().max(1e-9) <= tolerance,
            _ => false,
        },
        "set" => to_set(agent) == to_set(oracle),
        "tuple" => grade_tuple(agent, oracle, tolerance),
        _ => false,
    };

    let divergence = if pass {
        None
    } else {
        let mut d = serde_json::Map::new();
        d.insert("agent".into(), Value::String(agent.to_string()));
        d.insert("oracle".into(), Value::String(oracle.to_string()));
        if answer_type == "set" {
            let (sa, so) = (to_set(agent), to_set(oracle));
            let inter = sa.intersection(&so).count() as f64;
            let union = sa.union(&so).count().max(1) as f64;
            d.insert("jaccard".into(), json!(inter / union));
        }
        Some(Value::Object(d))
    };
    Grade { pass, divergence }
}

fn grade_tuple(agent: &str, oracle: &str, tolerance: f64) -> bool {
    let a: Vec<&str> = agent.split(',').map(str::trim).collect();
    let o: Vec<&str> = oracle.split(',').map(str::trim).collect();
    a.len() == o.len()
        && a.iter().zip(&o).all(|(x, y)| match (x.parse::<f64>(), y.parse::<f64>()) {
            (Ok(xv), Ok(yv)) => (xv - yv).abs() / yv.abs().max(1e-9) <= tolerance,
            _ => x.eq_ignore_ascii_case(y),
        })
}

/// The agent's `final_answer` value, flattened to a comparison string.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(value_to_string).collect::<Vec<_>>().join(","),
        other => other.to_string(),
    }
}

// ── Harness (the isolated, non-deterministic LLM driver) ─────────────────────

#[derive(Debug, Clone)]
struct FinalAnswer {
    #[allow(dead_code)]
    answer_type: String,
    value: Value,
}

#[derive(Debug, Clone)]
struct AgentRun {
    final_answer: Option<FinalAnswer>,
    tools_called: Vec<String>,
    turns_used: u32,
    /// Full transcript, retained for diagnostics (e.g. dumping on a grading miss).
    #[allow(dead_code)]
    raw_transcript: String,
    error: Option<String>,
}

trait Harness {
    fn run(&self, prompt: &str, mcp_config: &Path, allowed: &[&str]) -> Result<AgentRun, String>;
}

/// Test harness: returns a canned `AgentRun`, so the deterministic core is
/// testable with no Claude Code / network / auth.
#[cfg(test)]
struct StubHarness {
    canned: AgentRun,
}

#[cfg(test)]
impl Harness for StubHarness {
    fn run(&self, _prompt: &str, _mcp_config: &Path, _allowed: &[&str]) -> Result<AgentRun, String> {
        Ok(self.canned.clone())
    }
}

/// Real harness: shells out to `claude -p … --output-format stream-json --verbose`
/// (result-only `json` omits tool_use blocks — confirmed live on claude 2.1.150),
/// and parses the streamed messages.
struct McpHarness {
    model: Option<String>,
}

impl Harness for McpHarness {
    fn run(&self, prompt: &str, mcp_config: &Path, allowed: &[&str]) -> Result<AgentRun, String> {
        let mut cmd = std::process::Command::new("claude");
        cmd.arg("-p")
            .arg(prompt)
            .arg("--mcp-config")
            .arg(mcp_config)
            .arg("--allowedTools")
            .arg(allowed.join(","))
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose");
        if let Some(m) = &self.model {
            cmd.arg("--model").arg(m);
        }
        let out = cmd.output().map_err(|e| format!("spawn claude: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        if stdout.trim().is_empty() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("claude produced no output (status {}): {stderr}", out.status));
        }
        Ok(parse_stream_json(&stdout))
    }
}

/// Parse `stream-json --verbose` output: each line is a message. Collect tool_use
/// block names + count, and the LAST `final_answer` input.
fn parse_stream_json(transcript: &str) -> AgentRun {
    let mut tools_called = Vec::new();
    let mut final_answer = None;
    let mut turns = 0u32;

    for line in transcript.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = v.pointer("/message/content").and_then(Value::as_array) else { continue };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            turns += 1;
            let name = block.get("name").and_then(Value::as_str).unwrap_or("");
            let short = name.strip_prefix("mcp__legion__").unwrap_or(name).to_string();
            if short == "final_answer" {
                if let Some(input) = block.get("input") {
                    final_answer = Some(FinalAnswer {
                        answer_type: input
                            .get("answer_type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        value: input.get("value").cloned().unwrap_or(Value::Null),
                    });
                }
            }
            tools_called.push(short);
        }
    }

    let error = final_answer.is_none().then(|| "no final_answer".to_string());
    AgentRun { final_answer, tools_called, turns_used: turns, raw_transcript: transcript.to_string(), error }
}

// ── Result row (one JSON object per run) ─────────────────────────────────────

#[derive(Debug, Serialize)]
struct ResultRow {
    schema_version: u32,
    case_id: String,
    case_sha256: String,
    /// prof-viewer git HEAD at run time — the tool-layer attribution the hard
    /// gate requires.
    tools_commit: String,
    harness: String,
    model: String,
    seed: u64,
    started_at: String,
    finished_at: String,
    duration_s: f64,
    turns_used: u32,
    tools_called: Vec<String>,
    answer_type: String,
    agent_answer: Option<Value>,
    oracle_result: String,
    expected: Option<String>,
    graded: String,
    divergence: Option<Value>,
    error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn build_result_row(
    case: &Case,
    oracle: &str,
    tools_commit: String,
    harness: &str,
    model: &str,
    seed: u64,
    started_at: String,
    finished_at: String,
    duration_s: f64,
    run: &AgentRun,
) -> ResultRow {
    let agent_answer = run.final_answer.as_ref().map(|fa| fa.value.clone());
    let (graded, divergence) = if run.error.is_some() {
        ("error".to_string(), None)
    } else if let Some(fa) = &run.final_answer {
        let g = grade(&case.answer_type, &value_to_string(&fa.value), oracle, case.tolerance);
        (if g.pass { "pass" } else { "fail" }.to_string(), g.divergence)
    } else {
        ("error".to_string(), None)
    };

    ResultRow {
        schema_version: 1,
        case_id: case.case_id.clone(),
        case_sha256: case.sha256.clone(),
        tools_commit,
        harness: harness.to_string(),
        model: model.to_string(),
        seed,
        started_at,
        finished_at,
        duration_s,
        turns_used: run.turns_used,
        tools_called: run.tools_called.clone(),
        answer_type: case.answer_type.clone(),
        agent_answer,
        oracle_result: oracle.to_string(),
        expected: case.expected.clone(),
        graded,
        divergence,
        error: run.error.clone(),
    }
}

// ── Driver plumbing ──────────────────────────────────────────────────────────

const ALLOWED_TOOLS: &[&str] = &[
    "mcp__legion__run_query",
    "mcp__legion__overview",
    "mcp__legion__find_blockers",
    "mcp__legion__final_answer",
];

fn build_prompt(case: &Case) -> String {
    let mut p = case.question.clone();
    if let Some((s, e)) = case.range_ns {
        p.push_str(&format!("\n\nHighlighted range: {s} ns to {e} ns."));
    }
    p.push_str(&format!(
        "\n\nUse the legion tools (run_query, overview, find_blockers) to investigate the \
         profiling database, then finish by calling final_answer with answer_type=\"{}\" and \
         the computed value.",
        case.answer_type
    ));
    p
}

/// Write a per-run mcp-config pointing the `legion` server at the sibling `mcp`
/// binary (same target dir as this `eval` binary) with the case's DuckDB.
fn write_mcp_config(db: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mcp_bin = exe.parent().ok_or("no exe parent")?.join("mcp");
    let cfg = json!({
        "mcpServers": { "legion": {
            "command": mcp_bin.to_string_lossy(),
            "args": ["--duckdb", db.to_string_lossy()]
        } }
    });
    let tmp = std::env::temp_dir().join(format!("legion_eval_mcp_{}.json", unix_now()));
    std::fs::write(&tmp, cfg.to_string()).map_err(|e| format!("write mcp config: {e}"))?;
    Ok(tmp)
}

fn git_head() -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC RFC3339 (`YYYY-MM-DDTHH:MM:SSZ`) from Unix seconds — no chrono dep
/// (days-from-civil, Howard Hinnant).
fn rfc3339_utc(unix_secs: u64) -> String {
    let (days, rem) = ((unix_secs / 86400) as i64, unix_secs % 86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn stub_embedded_run() -> AgentRun {
    AgentRun {
        final_answer: None,
        tools_called: Vec::new(),
        turns_used: 0,
        raw_transcript: String::new(),
        error: Some("embedded harness not implemented (P2)".to_string()),
    }
}

fn emit_row(row: &ResultRow, out: Option<&str>) -> Result<(), String> {
    let line = serde_json::to_string(row).map_err(|e| format!("serialize row: {e}"))?;
    match out {
        Some(path) => {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("open {path}: {e}"))?;
            writeln!(f, "{line}").map_err(|e| format!("write {path}: {e}"))?;
        }
        None => println!("{line}"),
    }
    Ok(())
}

/// Full `eval run` flow. Returns `Ok(None)` for a soft-skip (DB absent).
fn run_eval(
    case_arg: &str,
    harness_name: &str,
    seed: u64,
    out: Option<&str>,
    model_override: Option<String>,
) -> Result<Option<ResultRow>, String> {
    let case = load_case(case_arg)?;
    let db = case.duckdb_abs_path();
    if !db.exists() {
        eprintln!("[eval] SKIP {}: duckdb absent at {}", case.case_id, db.display());
        return Ok(None);
    }
    verify_sha(&case)?; // refuse on mismatch
    let oracle = compute_oracle(&case)?; // asserts == expected; errs on drift
    let tools_commit = git_head();
    let model = model_override
        .or_else(|| case.model.clone())
        .unwrap_or_else(|| "sonnet".to_string());

    let started_at = rfc3339_utc(unix_now());
    let start = std::time::Instant::now();

    let run = match harness_name {
        "embedded" => stub_embedded_run(),
        "mcp" => {
            let mcp_cfg = write_mcp_config(&db)?;
            let harness = McpHarness { model: Some(model.clone()) };
            let result = harness.run(&build_prompt(&case), &mcp_cfg, ALLOWED_TOOLS);
            let _ = std::fs::remove_file(&mcp_cfg);
            match result {
                Ok(run) => run,
                Err(e) => AgentRun {
                    final_answer: None,
                    tools_called: Vec::new(),
                    turns_used: 0,
                    raw_transcript: String::new(),
                    error: Some(e),
                },
            }
        }
        other => return Err(format!("unknown harness: {other} (expected mcp|embedded)")),
    };

    let duration_s = start.elapsed().as_secs_f64();
    let finished_at = rfc3339_utc(unix_now());

    let row = build_result_row(
        &case,
        &oracle,
        tools_commit,
        harness_name,
        &model,
        seed,
        started_at,
        finished_at,
        duration_s,
        &run,
    );
    emit_row(&row, out)?;
    Ok(Some(row))
}

const USAGE: &str =
    "usage: eval run --case <DIR_or_ID> --harness <mcp|embedded> --seed <N> [--out <path.jsonl>] [--model <id>]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("run") {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }

    let (mut case, mut harness, mut out, mut model) = (None, None, None, None);
    let mut seed = 0u64;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--case" => case = it.next().cloned(),
            "--harness" => harness = it.next().cloned(),
            "--seed" => seed = it.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--out" => out = it.next().cloned(),
            "--model" => model = it.next().cloned(),
            other => eprintln!("[eval] ignoring unknown arg: {other}"),
        }
    }

    let Some(case) = case else {
        eprintln!("error: --case is required\n{USAGE}");
        std::process::exit(2);
    };
    let harness = harness.unwrap_or_else(|| "mcp".to_string());

    match run_eval(&case, &harness, seed, out.as_deref(), model) {
        Ok(Some(_row)) => {}
        Ok(None) => std::process::exit(0), // soft-skip
        Err(e) => {
            eprintln!("[eval] error: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(all(test, feature = "duckdb"))]
mod tests {
    use super::*;

    // ── grader ──────────────────────────────────────────────────────────────
    #[test]
    fn test_grade_uid() {
        assert!(grade("uid", "48", "48", 0.0).pass);
        assert!(!grade("uid", "999", "48", 0.0).pass);
        assert!(!grade("uid", "abc", "48", 0.0).pass);
        // tolerate an integer-valued float / whitespace (LLMs emit value:48.0)
        assert!(grade("uid", "48.0", "48", 0.0).pass);
        assert!(grade("uid", " 48 ", "48", 0.0).pass);
        assert!(!grade("uid", "48.5", "48", 0.0).pass); // non-integer float rejected
        // both unparseable must NOT pass (no None==None false-positive)
        assert!(!grade("uid", "abc", "xyz", 0.0).pass);
        // divergence localizes the miss
        let g = grade("uid", "999", "48", 0.0);
        assert_eq!(g.divergence.unwrap()["oracle"], "48");
    }

    #[test]
    fn test_grade_int() {
        // `int` (counts) grades exactly like `uid` — exact integer equality.
        assert!(grade("int", "6", "6", 0.0).pass);
        assert!(!grade("int", "7", "6", 0.0).pass);
        assert!(grade("int", "6.0", "6", 0.0).pass); // integer-valued float tolerated
        assert!(!grade("int", "abc", "6", 0.0).pass);
        assert!(!grade("int", "abc", "xyz", 0.0).pass); // no None==None false-positive
    }

    #[test]
    fn test_grade_number_tolerance() {
        assert!(grade("number", "100.5", "100.0", 0.01).pass); // 0.5% <= 1%
        assert!(!grade("number", "150.0", "100.0", 0.01).pass); // 50% > 1%
        assert!(grade("number", "100.0", "100.0", 0.0).pass);
    }

    #[test]
    fn test_grade_label_case_insensitive() {
        assert!(grade("label", "Computation-Bound", "computation-bound", 0.0).pass);
        assert!(!grade("label", "communication-bound", "computation-bound", 0.0).pass);
    }

    #[test]
    fn test_grade_set() {
        assert!(grade("set", "1,2,3", "3, 2, 1", 0.0).pass); // order/space-insensitive
        let g = grade("set", "1,2", "1,2,3", 0.0);
        assert!(!g.pass);
        assert_eq!(g.divergence.unwrap()["jaccard"], json!(2.0 / 3.0)); // |∩|/|∪|
    }

    // ── sha ─────────────────────────────────────────────────────────────────
    fn case_for_file(dir: &Path, relpath: &str, sha: &str) -> Case {
        Case {
            case_id: "t".into(),
            question: String::new(),
            duckdb_relpath: relpath.into(),
            sha256: sha.into(),
            range_ns: None,
            answer_type: "uid".into(),
            tolerance: 0.0,
            oracle_sql: String::new(),
            expected: None,
            model: None,
            case_dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn test_verify_sha() {
        let dir = std::env::temp_dir();
        let file = dir.join("legion_eval_sha_test.bin");
        std::fs::write(&file, b"hello legion").unwrap();
        let real = hex(&Sha256::digest(b"hello legion"));

        assert!(verify_sha(&case_for_file(&dir, "legion_eval_sha_test.bin", &real)).is_ok());
        let corrupt = format!("{}0", &real[..real.len() - 1]); // flip last nibble
        assert!(verify_sha(&case_for_file(&dir, "legion_eval_sha_test.bin", &corrupt)).is_err());
        let _ = std::fs::remove_file(&file);
    }

    // ── toml parsing ─────────────────────────────────────────────────────────
    #[test]
    fn test_parse_toml_multiline_and_scalars() {
        let src = "case_id = \"X-1\"   # a comment\nrange_ns = [10, 20]\ntolerance = 0.5\noracle_sql = \"\"\"\nSELECT 1\nFROM t\n\"\"\"\nexpected = \"48\"\n";
        let m = parse_toml(src).unwrap();
        assert_eq!(get_str(&m, "case_id").as_deref(), Some("X-1"));
        assert_eq!(get_int_pair(&m, "range_ns"), Some((10, 20)));
        assert_eq!(get_f64(&m, "tolerance"), Some(0.5));
        assert!(m["oracle_sql"].contains("SELECT 1") && m["oracle_sql"].contains("FROM t"));
        assert_eq!(get_str(&m, "expected").as_deref(), Some("48"));
    }

    // ── oracle (needs bg4N2; soft-skip) ──────────────────────────────────────
    #[test]
    fn test_compute_oracle_l1_matches_expected() {
        let case = match load_case("L1-longest-in-range-001") {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping L1 oracle: {e}");
                return;
            }
        };
        if !case.duckdb_abs_path().exists() {
            eprintln!("skipping L1 oracle: bg4N2 absent");
            return;
        }
        let oracle = compute_oracle(&case).expect("oracle should compute + match expected");
        assert_eq!(oracle, "48");
        assert_eq!(case.expected.as_deref(), Some("48"));
    }

    /// The answer-key LOCK: for every fixture, `compute_oracle` must compute the
    /// ground truth on a direct connection AND match the manifest `expected`
    /// (compute_oracle returns Err on drift, so `.is_ok()` IS the assertion — and
    /// it grades by answer_type, so set order doesn't matter). Soft-skips if bg4N2
    /// is absent. This is the same discipline that caught the L1 any_value bug.
    #[test]
    fn test_all_fixture_oracles_locked() {
        const CASES: &[&str] = &[
            "L1-longest-in-range-001",
            "L1-longest-anyitem-002",
            "L1-total-bytes-003",
            "L1-data-movement-002",
            "L1-distinct-types-g2d-004",
            "L1-find-long-tasks-005",
            "L2-cp-root-001",
            "L2-first-util-002",
            "L2-children-002",
            "L3-bound-in-range-001",
            "L3-busiest-proc-kind-002",
        ];
        // All fixtures share bg4N2; check presence once via the first case.
        let probe = load_case(CASES[0]).expect("load first case");
        if !probe.duckdb_abs_path().exists() {
            eprintln!("skipping oracle lock: bg4N2 absent at {}", probe.duckdb_abs_path().display());
            return;
        }
        for id in CASES {
            let case = load_case(id).unwrap_or_else(|e| panic!("load {id}: {e}"));
            // Err == oracle drift (computed value != manifest expected).
            let value = compute_oracle(&case)
                .unwrap_or_else(|e| panic!("ORACLE LOCK FAILED for {id}: {e}"));
            assert!(case.expected.is_some(), "{id} has no expected to lock against");
            eprintln!("locked {id}: oracle={value:?}");
        }
    }

    // ── end-to-end with StubHarness (no Claude/network) ──────────────────────
    fn fake_case(answer_type: &str) -> Case {
        Case {
            case_id: "FAKE-001".into(),
            question: "q".into(),
            duckdb_relpath: "x.duckdb".into(),
            sha256: "deadbeef".into(),
            range_ns: Some((1, 2)),
            answer_type: answer_type.into(),
            tolerance: 0.0,
            oracle_sql: "SELECT 1".into(),
            expected: Some("48".into()),
            model: None,
            case_dir: std::env::temp_dir(),
        }
    }

    fn run_with_stub(answer: Option<Value>) -> ResultRow {
        let canned = AgentRun {
            final_answer: answer.map(|v| FinalAnswer { answer_type: "uid".into(), value: v }),
            tools_called: vec!["run_query".into(), "final_answer".into()],
            turns_used: 2,
            raw_transcript: "stub".into(),
            error: None,
        };
        // mirror run_eval's grading wiring, but with the stub (no DB/LLM).
        let stub = StubHarness { canned };
        let run = stub
            .run("p", Path::new("/dev/null"), ALLOWED_TOOLS)
            .unwrap();
        let run = if run.final_answer.is_none() {
            AgentRun { error: Some("no final_answer".into()), ..run }
        } else {
            run
        };
        build_result_row(
            &fake_case("uid"),
            "48",
            "testcommit".into(),
            "mcp",
            "sonnet",
            0,
            rfc3339_utc(0),
            rfc3339_utc(1),
            1.0,
            &run,
        )
    }

    #[test]
    fn test_stub_pass_fail_error_and_roundtrip() {
        let pass = run_with_stub(Some(json!(48)));
        assert_eq!(pass.graded, "pass");

        let fail = run_with_stub(Some(json!(999)));
        assert_eq!(fail.graded, "fail");
        assert_eq!(fail.divergence.as_ref().unwrap()["oracle"], "48");

        let err = run_with_stub(None);
        assert_eq!(err.graded, "error");
        assert_eq!(err.error.as_deref(), Some("no final_answer"));

        // serializes and round-trips via serde_json
        let s = serde_json::to_string(&pass).unwrap();
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back["case_id"], "FAKE-001");
        assert_eq!(back["graded"], "pass");
        assert_eq!(back["oracle_result"], "48");
        assert_eq!(back["tools_commit"], "testcommit");
        assert_eq!(back["schema_version"], 1);
    }

    #[test]
    fn test_parse_stream_json_extracts_last_final_answer() {
        let transcript = concat!(
            r#"{"type":"system","subtype":"init"}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__legion__run_query","input":{"sql":"SELECT 1"}}]}}"#, "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__legion__final_answer","input":{"answer_type":"uid","value":48}}]}}"#, "\n",
            r#"{"type":"result","subtype":"success"}"#, "\n",
        );
        let run = parse_stream_json(transcript);
        assert_eq!(run.turns_used, 2);
        assert_eq!(run.tools_called, vec!["run_query", "final_answer"]);
        let fa = run.final_answer.unwrap();
        assert_eq!(fa.value, json!(48));
        assert!(run.error.is_none());

        let none = parse_stream_json(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#);
        assert_eq!(none.error.as_deref(), Some("no final_answer"));
    }

    #[test]
    fn test_rfc3339_epoch() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}
