//! Headless runner for the EMBEDDED agent — the gradee side of the eval's
//! `embedded` harness (mirrors how `bin/mcp.rs` is the gradee side of the `mcp`
//! harness). `bin/eval.rs` SPAWNS this binary and never imports the crate, so
//! the oracle-independence invariant holds: grader and gradee share no code.
//!
//! Protocol: prompt on STDIN (read to EOF); ONE JSON envelope on STDOUT:
//!   {"text": ..., "`turns_used"`: N, "`queries_executed"`: N,
//!    "`tools_called"`: [...], "error": null | "..."}
//! Exit codes: 0 = envelope emitted (agent-level errors ride IN the envelope);
//! 2 = precondition failure (no `ANTHROPIC_API_KEY` / bad args) with stderr text.
//!
//! Headless hazards: a drainer thread auto-replies to the two
//! blocking event classes — `QuestionForUser` gets a canned `UserAnswer` and
//! every nav/screenshot request gets `ScreenshotData` with EMPTY png bytes,
//! which returns an immediate clean tool error ("Screenshot capture returned
//! empty data.") instead of a 10s timeout per call. Replies MUST echo the
//! event's `request_id` — mismatched ids are discarded by the session. The
//! drainer also collects tool names for the envelope; it is joined AFTER the
//! session drops (closing `event_tx`) so no trailing `ToolCall` events race.
//!
//! No tracing subscriber is installed — nothing but the envelope reaches stdout.
//!
//! Usage: `embedded_runner --duckdb <path.duckdb> --model <API model id>`

use legion_prof_viewer::ai::agent::{AgentEvent, AgentSession, UiCommand};
use serde_json::json;
use std::io::Read;
use std::sync::mpsc;

fn main() {
    let mut duckdb: Option<String> = None;
    let mut model: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--duckdb" => duckdb = args.next(),
            "--model" => model = args.next(),
            other => eprintln!("[embedded_runner] ignoring unknown arg: {other}"),
        }
    }
    let Some(duckdb) = duckdb else {
        eprintln!("error: --duckdb <path> is required");
        std::process::exit(2);
    };
    // The embedded agent talks to the raw Anthropic API — it needs a real key
    // (claude-CLI subscription login does NOT apply here).
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k.trim().to_owned(),
        _ => {
            eprintln!("error: ANTHROPIC_API_KEY is not set (the embedded agent uses the raw API)");
            std::process::exit(2);
        }
    };
    // Full API model id expected (eval.rs maps CLI aliases before spawning).
    let model = model.unwrap_or_else(|| "claude-sonnet-4-6".to_owned());

    let mut prompt = String::new();
    if std::io::stdin().read_to_string(&mut prompt).is_err() || prompt.trim().is_empty() {
        eprintln!("error: expected the prompt on stdin");
        std::process::exit(2);
    }

    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>();
    let (cmd_tx, cmd_rx) = mpsc::channel::<UiCommand>();

    // Drainer: collect tool names + auto-reply to blocking requests (echoing
    // request_id — the session discards mismatched replies).
    let drainer = std::thread::spawn(move || {
        let mut tools: Vec<String> = Vec::new();
        for ev in event_rx {
            match ev {
                AgentEvent::ToolCall { name, .. } => tools.push(name),
                AgentEvent::QuestionForUser { request_id, .. } => {
                    let _ = cmd_tx.send(UiCommand::UserAnswer {
                        request_id,
                        answer: "(non-interactive eval — no user available; proceed with \
                                 your best judgment)"
                            .to_owned(),
                    });
                }
                // Every nav/screenshot request: empty png => immediate clean
                // tool error, no 10s timeout burn.
                AgentEvent::ScreenshotRequest { request_id }
                | AgentEvent::ZoomRequest { request_id, .. }
                | AgentEvent::PanRequest { request_id, .. }
                | AgentEvent::ScrollToRequest { request_id, .. }
                | AgentEvent::SetViewRequest { request_id, .. }
                | AgentEvent::SearchRequest { request_id, .. }
                | AgentEvent::ResetViewRequest { request_id } => {
                    let _ = cmd_tx.send(UiCommand::ScreenshotData {
                        request_id,
                        png_bytes: Vec::new(),
                        metadata: String::new(),
                    });
                }
                _ => {}
            }
        }
        tools
    });

    let mut session = AgentSession::new(
        api_key,
        model,
        duckdb,
        String::new(), // no code root (parallels the mcp harness's data-tool restriction)
        String::new(), // no wiki root
        event_tx,
        cmd_rx,
    );
    let result = session.ask(&prompt);

    // Close event_tx (drops with the session) BEFORE joining the drainer, so
    // every ToolCall event is in and the envelope's tools_called is complete.
    drop(session);
    let tools_called = drainer.join().unwrap_or_default();

    let envelope = match result {
        Ok(resp) => json!({
            "text": resp.text,
            "turns_used": resp.turns_used,
            "queries_executed": resp.queries_executed,
            "tools_called": tools_called,
            "error": serde_json::Value::Null,
        }),
        Err(e) => json!({
            "text": "",
            "turns_used": 0,
            "queries_executed": 0,
            "tools_called": tools_called,
            "error": e,
        }),
    };
    println!("{envelope}");
}
