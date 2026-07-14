//! Headless stdio MCP server for the Legion profiler data tools.
//!
//! A hand-rolled SYNCHRONOUS, newline-delimited stdio JSON-RPC 2.0 transport. All
//! protocol logic lives in the shared, transport-agnostic dispatch core
//! [`legion_prof_viewer::ai::mcp_core`] (reused by the in-viewer HTTP server too);
//! this file is just the stdin/stdout read-loop.
//!
//! Usage: `mcp --duckdb <path.duckdb> [--code-root <dir>]`.

use legion_prof_viewer::ai::mcp_core::{ServerCtx, handle_request};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn main() {
    let mut duckdb_path: Option<String> = None;
    let mut code_root: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duckdb" => duckdb_path = args.next(),
            "--code-root" => code_root = args.next(),
            other => eprintln!("[legion-prof] ignoring unknown arg: {other}"),
        }
    }
    let Some(duckdb_path) = duckdb_path else {
        eprintln!("error: --duckdb <path> is required");
        std::process::exit(2);
    };
    let ctx = ServerCtx::new(duckdb_path, code_root);

    // Transport: newline-delimited JSON-RPC over stdin/stdout. Flush after every
    // response so the client (e.g. Claude Code) sees each message immediately.
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                if let Some(resp) = handle_request(&req, &ctx) {
                    let _ = writeln!(stdout, "{resp}");
                    let _ = stdout.flush();
                }
            }
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0", "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") }
                });
                let _ = writeln!(stdout, "{err}");
                let _ = stdout.flush();
            }
        }
    }
}
