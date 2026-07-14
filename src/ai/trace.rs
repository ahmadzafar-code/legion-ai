//! Structured tracing for the AI agent loop.
//!
//! Emits one JSON Line per closed span to `{out_dir}/agent_traces/agent.jsonl`.
//! Span hierarchy: `agent.run` → `agent.turn` → {`agent.tool_call`, `agent.claude_api`}.
//!
//! This module only wires up the subscriber; the actual `info_span!`/`record`
//! calls live in `agent.rs`.

use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::OnceLock;
use std::time::SystemTime;

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
