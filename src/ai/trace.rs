//! Structured tracing for the AI agent loop.
//!
//! Two independent layers:
//!
//! 1. **Span timings** ([`init_subscriber`], OPT-IN via
//!    `LEGION_PROF_AI_TRACE_DIR` + the `agent.*` spans in `agent.rs`): one JSON
//!    Line per closed span to `{out_dir}/agent_traces/agent.jsonl`.
//! 2. **Session reasoning transcripts** ([`SessionTrace`], ON BY DEFAULT):
//!    one JSONL file per chat session recording CONTENT — user turns,
//!    assistant narration, tool calls with full inputs, image-redacted tool
//!    results, per-turn usage — so the team can replay how a diagnosis was
//!    reached and improve the product. `LEGION_PROF_AI_TRACE=off` disables;
//!    `LEGION_PROF_AI_TRACE_DIR` overrides the directory (default
//!    `~/.legion_prof_viewer/traces`). Best-effort everywhere: any I/O failure
//!    silently disables tracing rather than disturbing the session.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::{Value, json};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Marker so `init_subscriber` is a no-op on subsequent calls.
static INIT: OnceLock<()> = OnceLock::new();

/// Initialize the global tracing subscriber. Idempotent: calling twice is a
/// no-op. The first caller wins; subsequent calls return Ok(()) without
/// reinitializing.
///
/// Writes JSON Lines to `{out_dir}/agent_traces/agent.jsonl` (append mode).
/// Each closed span produces one line. Honors the `RUST_LOG` env var via
/// `EnvFilter` (default filter: `agent=info`).
pub fn init_subscriber(out_dir: &Path) -> Result<(), String> {
    if INIT.get().is_some() {
        return Ok(());
    }

    let trace_dir = out_dir.join("agent_traces");
    fs::create_dir_all(&trace_dir)
        .map_err(|e| format!("Failed to create {}: {e}", trace_dir.display()))?;

    let log_path = trace_dir.join("agent.jsonl");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("Failed to open {}: {e}", log_path.display()))?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("agent=info"));

    let layer = fmt::layer()
        .json()
        .with_writer(file)
        .with_target(true)
        .with_span_events(fmt::format::FmtSpan::CLOSE);

    let result = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();

    if result.is_ok() {
        let _ = INIT.set(());
        Ok(())
    } else {
        // Another subscriber was already installed (e.g. by a test). Treat as
        // success — best-effort instrumentation.
        let _ = INIT.set(());
        Ok(())
    }
}

/// Generate a session ID. Format: hex-encoded systime nanos + 4 hex chars of
/// thread-id, sufficient for human-paced sessions without a UUID dep.
pub fn new_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tid_suffix = format!("{:?}", std::thread::current().id());
    let tid_hex: String = tid_suffix
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(4)
        .collect();
    format!("{nanos:x}-{tid_hex}")
}

/// One JSONL reasoning transcript per chat session (see module docs). Shared
/// as `Arc` between the panel (user turns, stop clicks, turn outcomes) and the
/// Claude Code stdout pump (assistant text, tool calls/results, usage).
pub struct SessionTrace {
    file: Mutex<fs::File>,
    /// Where this session's transcript lives (shown once on stderr).
    pub path: PathBuf,
}

impl SessionTrace {
    /// Trace directory per the environment; `None` = tracing disabled.
    /// Pure so tests can drive it: `toggle` = `LEGION_PROF_AI_TRACE`,
    /// `dir_override` = `LEGION_PROF_AI_TRACE_DIR`, `home` = `$HOME`.
    fn resolve_dir(
        toggle: Option<&str>,
        dir_override: Option<&str>,
        home: Option<&Path>,
    ) -> Option<PathBuf> {
        if matches!(
            toggle
                .map(str::trim)
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("off" | "0" | "false" | "no")
        ) {
            return None;
        }
        if let Some(d) = dir_override.map(str::trim).filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(d));
        }
        home.map(|h| h.join(".legion_prof_viewer").join("traces"))
    }

    /// Open a session transcript in `dir` (creating it). `None` on any I/O
    /// failure — tracing is best-effort, never a startup blocker.
    ///
    /// Traces record untruncated tool inputs/outputs — full SQL, Bash commands
    /// and their output, source snippets the agent read — so on unix the
    /// directory is created `0700` and each file `0600`: a co-tenant on a shared
    /// login node must not be able to read another user's session.
    pub fn open_in(dir: &Path) -> Option<Arc<Self>> {
        Self::create_dir_private(dir).ok()?;
        let path = dir.join(format!("session_{}.jsonl", new_session_id()));
        let mut opts = OpenOptions::new();
        opts.create_new(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&path).ok()?;
        Some(Arc::new(SessionTrace {
            file: Mutex::new(file),
            path,
        }))
    }

    /// Create `dir` (and parents) owner-only (`0700`) on unix; a plain
    /// create_dir_all elsewhere. Idempotent — an existing dir is fine.
    fn create_dir_private(dir: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(dir)
        }
    }

    /// Open per the environment (default ON; see module docs). `None` when
    /// disabled or the directory is unusable.
    pub fn open_default() -> Option<Arc<Self>> {
        let toggle = std::env::var("LEGION_PROF_AI_TRACE").ok();
        let dir_override = std::env::var("LEGION_PROF_AI_TRACE_DIR").ok();
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        let dir = Self::resolve_dir(toggle.as_deref(), dir_override.as_deref(), home.as_deref())?;
        Self::open_in(&dir)
    }

    /// Append one event line: `{"ts_ms":…,"kind":…,…payload}`. Object payloads
    /// merge their fields into the line; anything else lands under `"data"`.
    /// Write errors are swallowed (best-effort).
    pub fn event(&self, kind: &str, payload: Value) {
        let ts_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut line = json!({ "ts_ms": ts_ms, "kind": kind });
        match payload {
            Value::Object(map) => {
                for (k, v) in map {
                    line[k] = v;
                }
            }
            Value::Null => {}
            other => line["data"] = other,
        }
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Deep-copy `v` with every image content block's base64 payload replaced by a
/// short note — both the Claude API shape (`source.data`) and the MCP shape
/// (`data` + `mimeType`). A screenshot echo is ~400 KB of base64; the trace
/// keeps the fact that an image was returned, not the bytes.
pub fn redact_images(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("image") {
                let media = v
                    .pointer("/source/media_type")
                    .or_else(|| map.get("mimeType"))
                    .and_then(Value::as_str)
                    .unwrap_or("image");
                let b64_len = v
                    .pointer("/source/data")
                    .or_else(|| map.get("data"))
                    .and_then(Value::as_str)
                    .map(str::len)
                    .unwrap_or(0);
                let kb = (b64_len as f64 * 3.0 / 4.0 / 1024.0).round() as u64;
                return json!({
                    "type": "image",
                    "note": format!("[{media} ~{kb} KB - base64 elided from trace]"),
                });
            }
            Value::Object(
                map.iter()
                    .map(|(k, val)| (k.clone(), redact_images(val)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_images).collect()),
        other => other.clone(),
    }
}

/// Record token-usage fields from a Claude API `usage` JSON object onto the
/// current span. Missing fields default to 0.
pub fn record_usage(span: &tracing::Span, usage: &serde_json::Value) {
    span.record("tokens_input", usage["input_tokens"].as_u64().unwrap_or(0));
    span.record(
        "tokens_output",
        usage["output_tokens"].as_u64().unwrap_or(0),
    );
    span.record(
        "cache_read",
        usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
    );
    span.record(
        "cache_creation",
        usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_nonempty_and_unique() {
        let a = new_session_id();
        // sleep enough that systime nanos advance
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_session_id();
        assert!(!a.is_empty());
        assert!(!b.is_empty());
        assert_ne!(a, b, "session IDs should differ across calls");
    }

    /// Traces hold sensitive command/output/source data — on unix the dir must
    /// be 0700 and each session file 0600 so a co-tenant cannot read them.
    #[cfg(unix)]
    #[test]
    fn session_trace_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("legion_trace_perm_{}", new_session_id()));
        let t = SessionTrace::open_in(&dir).expect("open");
        t.event("user_turn", json!({ "text": "secret query" }));
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let file_mode = fs::metadata(&t.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "trace dir must be owner-only");
        assert_eq!(file_mode, 0o600, "trace file must be owner-only");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Env resolution: kill-switch beats everything, explicit dir beats the
    /// home default, home default is the documented path, no home -> None.
    #[test]
    fn session_trace_resolve_dir_precedence() {
        let home = Path::new("/home/u");
        for off in ["off", "0", "false", "no", " OFF "] {
            assert_eq!(
                SessionTrace::resolve_dir(Some(off), Some("/x"), Some(home)),
                None,
                "toggle {off:?} must disable"
            );
        }
        assert_eq!(
            SessionTrace::resolve_dir(None, Some("/custom/dir"), Some(home)),
            Some(PathBuf::from("/custom/dir"))
        );
        assert_eq!(
            SessionTrace::resolve_dir(Some("on"), None, Some(home)),
            Some(home.join(".legion_prof_viewer").join("traces"))
        );
        assert_eq!(SessionTrace::resolve_dir(None, None, None), None);
    }

    /// Events land as parseable JSONL with ts_ms + kind, object payloads
    /// merged flat.
    #[test]
    fn session_trace_writes_parseable_jsonl() {
        let dir = std::env::temp_dir().join(format!("legion_trace_test_{}", new_session_id()));
        let t = SessionTrace::open_in(&dir).expect("open");
        t.event(
            "user_turn",
            json!({ "text": "why slow?", "backend": "claude-code" }),
        );
        t.event("stop_click", Value::Null);
        let body = fs::read_to_string(&t.path).unwrap();
        let lines: Vec<Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid JSON line"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "user_turn");
        assert_eq!(lines[0]["text"], "why slow?");
        assert!(lines[0]["ts_ms"].as_u64().unwrap() > 0);
        assert_eq!(lines[1]["kind"], "stop_click");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Image blocks lose their base64 in both API and MCP shapes; sibling
    /// text blocks survive verbatim.
    #[test]
    fn redact_images_strips_base64_keeps_text() {
        let v = json!([{
            "type": "tool_result",
            "content": [
                { "type": "image",
                  "source": { "type": "base64", "media_type": "image/jpeg", "data": "/9j/AAAA".repeat(512) } },
                { "type": "image", "data": "iVBOR".repeat(100), "mimeType": "image/png" },
                { "type": "text", "text": "Visible range 0-100ms" }
            ]
        }]);
        let r = redact_images(&v);
        let s = r.to_string();
        assert!(!s.contains("/9j/"), "API-shape base64 must be gone");
        assert!(!s.contains("iVBOR"), "MCP-shape base64 must be gone");
        assert!(s.contains("base64 elided"));
        assert!(s.contains("image/jpeg") && s.contains("image/png"));
        assert!(s.contains("Visible range 0-100ms"));
    }

    #[test]
    fn init_subscriber_is_idempotent() {
        let dir = std::env::temp_dir().join("legion_test_trace_idempotent");
        let _ = fs::remove_dir_all(&dir);
        let r1 = init_subscriber(&dir);
        let r2 = init_subscriber(&dir);
        assert!(r1.is_ok());
        assert!(r2.is_ok());
        let _ = fs::remove_dir_all(&dir);
    }
}
