//! Transport-agnostic MCP dispatch core (data tools only).
//!
//! This is the durable part of every Legion MCP server: a pure
//! `(&Value, &ServerCtx) -> Option<Value>` request handler plus the tool-list and
//! tool-call builders. Two transports wrap it unchanged:
//!   - `src/bin/mcp.rs` — a hand-rolled synchronous stdio JSON-RPC server.
//!   - `ai/viewer_mcp.rs` — the in-viewer HTTP server (feature `viewer-mcp`).
//!
//! It exposes only the HEADLESS DATA tools (run_query / overview / find_blockers /
//! read_code / list_files / final_answer) by reusing the pure functions in
//! [`crate::ai::tools`]; it never re-implements tool logic and never opens its own
//! DuckDB connection (every query routes through the hardened
//! `execute_run_query_raw`). GUI/view tools are NEVER advertised here.

use super::tools::{
    execute_list_files, execute_read_code, execute_run_query_raw, find_blockers_sql,
    gather_overview, tool_definitions,
};
use serde_json::{json, Value};

/// Protocol version the stdio server speaks (the in-viewer HTTP server overrides
/// this to "2025-03-26" — the streamable-HTTP version Claude Code negotiates).
pub const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// The headless data tools advertised to clients (a subset of `tool_definitions`).
/// GUI/view tools (screenshot, zoom_to, highlight, ask_user, …) have no pure
/// backing fn and are NEVER advertised.
const HEADLESS_TOOLS: &[&str] = &["run_query", "overview", "list_files", "read_code"];

/// Valid `answer_type` values for the `final_answer` tool (the eval grader pins
/// this enum).
const ANSWER_TYPES: &[&str] = &["uid", "number", "set", "label", "tuple", "diagnosis"];

/// Server context: which case DB to query, an optional source root for the code
/// tools, and the protocol version this transport advertises. Held immutably
/// across requests.
pub struct ServerCtx {
    pub duckdb_path: String,
    pub code_root: Option<String>,
    pub protocol_version: &'static str,
}

impl ServerCtx {
    /// Construct a context with the default (stdio) protocol version.
    pub fn new(duckdb_path: String, code_root: Option<String>) -> Self {
        ServerCtx {
            duckdb_path,
            code_root,
            protocol_version: DEFAULT_PROTOCOL_VERSION,
        }
    }

    /// Override the advertised protocol version (the HTTP transport uses this).
    pub fn with_protocol(mut self, version: &'static str) -> Self {
        self.protocol_version = version;
        self
    }
}

/// Durable dispatch core. Returns `Some(response)` for a request, `None` for a
/// notification (no reply). Transport-agnostic and free of any I/O.
pub fn handle_request(req: &Value, ctx: &ServerCtx) -> Option<Value> {
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let id = req.get("id").cloned();

    // Notifications carry no `id` (and MCP names them `notifications/*`) — no reply.
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap();
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Two-tier outcome: Ok(result) for protocol successes (including tool
    // failures, which are results with isError:true); Err((code,msg)) only for
    // JSON-RPC protocol errors (e.g. unknown method).
    let outcome: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(initialize_result(&params, ctx)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result(ctx)),
        "tools/call" => Ok(tools_call_result(&params, ctx)),
        _ => Err((-32601, "method not found".to_owned())),
    };

    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

fn initialize_result(params: &Value, ctx: &ServerCtx) -> Value {
    if let Some(requested) = params.get("protocolVersion").and_then(Value::as_str) {
        if requested != ctx.protocol_version {
            eprintln!(
                "[legion-prof] client requested protocol {requested}; responding with {}",
                ctx.protocol_version
            );
        }
    }
    json!({
        "protocolVersion": ctx.protocol_version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "legion-prof", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// Build the advertised tool list: the headless subset of `tool_definitions`
/// (with `input_schema` renamed to MCP's `inputSchema` here in the dispatch core,
/// never in `tools.rs`), plus the inline `find_blockers` and `final_answer`
/// definitions. Code tools are omitted unless a `code_root` was configured.
fn tools_list_result(ctx: &ServerCtx) -> Value {
    let has_code = ctx.code_root.is_some();
    let mut tools: Vec<Value> = tool_definitions(true, true)
        .into_iter()
        .filter(|t| {
            let name = t.get("name").and_then(Value::as_str).unwrap_or("");
            HEADLESS_TOOLS.contains(&name)
                && (has_code || (name != "list_files" && name != "read_code"))
        })
        .map(|mut t| {
            if let Some(obj) = t.as_object_mut() {
                if let Some(schema) = obj.remove("input_schema") {
                    obj.insert("inputSchema".to_owned(), schema);
                }
            }
            t
        })
        .collect();

    tools.push(json!({
        "name": "find_blockers",
        "description": "Walk the cycle-guarded critical path from a task to its root \
                        blocker. Takes an integer start_uid and returns the chain \
                        (depth, uid, title, durations) as a JSON array, deepest row last.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "start_uid": {
                    "type": "integer",
                    "description": "item_uid to start the critical-path walk from"
                }
            },
            "required": ["start_uid"]
        }
    }));

    tools.push(json!({
        "name": "final_answer",
        "description": "Emit the FINAL structured answer for this question. Call exactly \
                        once to terminate. The grader reads this — return the computed \
                        value, not prose.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "answer_type": {
                    "type": "string",
                    "enum": ["uid", "number", "set", "label", "tuple", "diagnosis"],
                    "description": "shape of `value`"
                },
                "value": {
                    "description": "the answer: uid int | number | array | label string | tuple"
                },
                "evidence": {
                    "type": "string",
                    "description": "the query/reasoning that produced it (optional)"
                }
            },
            "required": ["answer_type", "value"]
        }
    }));

    json!({ "tools": tools })
}

/// Execute a `tools/call`. Tool failures are RESULTS with `isError:true` (the
/// model sees the message); only protocol failures become JSON-RPC errors.
fn tools_call_result(params: &Value, ctx: &ServerCtx) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    let (text, is_error) = match name {
        "run_query" => {
            let sql = args.get("sql").and_then(Value::as_str).unwrap_or("");
            // Inherits read-only + enable_external_access(false) + 50-row cap from
            // P0(a); the JSON (incl. any future `_truncated` marker) is verbatim.
            into_tool_result(execute_run_query_raw(&ctx.duckdb_path, sql))
        }
        "find_blockers" => match args.get("start_uid").and_then(Value::as_u64) {
            // Typed u64 -> find_blockers_sql formats only this integer; no model SQL.
            Some(uid) => {
                into_tool_result(execute_run_query_raw(&ctx.duckdb_path, &find_blockers_sql(uid)))
            }
            None => ("find_blockers requires start_uid (integer).".to_owned(), true),
        },
        "overview" => into_tool_result(gather_overview(&ctx.duckdb_path)),
        "list_files" => match &ctx.code_root {
            Some(root) => {
                let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
                into_tool_result(execute_list_files(root, path))
            }
            None => (
                "list_files unavailable: server started without a code root.".to_owned(),
                true,
            ),
        },
        "read_code" => match &ctx.code_root {
            Some(root) => match args.get("path").and_then(Value::as_str) {
                Some(path) => into_tool_result(execute_read_code(root, path)),
                None => ("read_code requires path (string).".to_owned(), true),
            },
            None => (
                "read_code unavailable: server started without a code root.".to_owned(),
                true,
            ),
        },
        "final_answer" => final_answer_result(&args),
        other => (format!("unknown tool: {other}"), true),
    };

    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

/// Validate and acknowledge a `final_answer`. On success, echo the normalized
/// `{answer_type, value}` as a deterministic, server-controlled capture point.
fn final_answer_result(args: &Value) -> (String, bool) {
    let answer_type = args.get("answer_type").and_then(Value::as_str);
    let value = args.get("value");
    match answer_type {
        Some(at) if ANSWER_TYPES.contains(&at) => match value {
            Some(v) if !v.is_null() => {
                (json!({ "answer_type": at, "value": v }).to_string(), false)
            }
            _ => (
                format!("final_answer requires a non-null `value` (answer_type {at})."),
                true,
            ),
        },
        _ => (
            format!("final_answer requires answer_type in {ANSWER_TYPES:?} and a non-null value."),
            true,
        ),
    }
}

/// Map a pure-fn `Result<String, String>` into `(text, is_error)`.
fn into_tool_result(r: Result<String, String>) -> (String, bool) {
    match r {
        Ok(text) => (text, false),
        Err(err) => (err, true),
    }
}

#[cfg(all(test, feature = "duckdb"))]
mod tests {
    use super::*;

    /// The bg4N2 fixture (untracked, in the repo root). Soft-skip if absent.
    fn test_db() -> Option<String> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb");
        p.exists().then(|| p.to_str().unwrap().to_owned())
    }

    fn req(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    /// Drive a `tools/call` and return `(content_text, is_error)`.
    fn call(ctx: &ServerCtx, name: &str, args: Value) -> (String, bool) {
        let resp = handle_request(
            &req("tools/call", json!({ "name": name, "arguments": args })),
            ctx,
        )
        .expect("tools/call must produce a response");
        let result = &resp["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap(),
        )
    }

    fn dummy_ctx() -> ServerCtx {
        ServerCtx::new("unused".to_owned(), None)
    }

    #[test]
    fn test_initialize() {
        let resp = handle_request(&req("initialize", json!({})), &dummy_ctx()).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], DEFAULT_PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "legion-prof");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn test_initialize_protocol_override() {
        let ctx = dummy_ctx().with_protocol("2025-03-26");
        let resp = handle_request(&req("initialize", json!({})), &ctx).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    }

    #[test]
    fn test_notifications_get_no_reply() {
        // No id + notifications/* name => notification => no reply.
        let n = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_request(&n, &dummy_ctx()).is_none());
    }

    #[test]
    fn test_unknown_method_is_protocol_error() {
        let resp = handle_request(&req("does/not/exist", json!({})), &dummy_ctx()).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp.get("result").is_none(), "must be an error, not a result");
    }

    #[test]
    fn test_tools_list_shape_no_code_root() {
        let resp = handle_request(&req("tools/list", json!({})), &dummy_ctx()).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

        for want in ["run_query", "overview", "find_blockers", "final_answer"] {
            assert!(names.contains(&want), "tools/list missing {want}");
        }
        // Code tools omitted without a code root.
        assert!(!names.contains(&"list_files"));
        assert!(!names.contains(&"read_code"));
        // GUI/view tools are NEVER advertised.
        for forbidden in [
            "screenshot", "zoom_to", "pan", "scroll_to", "set_view", "search", "reset_view",
            "highlight", "clear_highlights", "ask_user", "update_findings",
        ] {
            assert!(!names.contains(&forbidden), "must not advertise {forbidden}");
        }
        // MCP requires camelCase inputSchema; the Anthropic snake_case must not leak.
        for t in tools {
            assert!(t.get("inputSchema").is_some(), "tool {} missing inputSchema", t["name"]);
            assert!(t.get("input_schema").is_none(), "tool {} leaked input_schema", t["name"]);
        }
    }

    #[test]
    fn test_tools_list_includes_code_tools_with_root() {
        let ctx = ServerCtx::new("unused".to_owned(), Some("/tmp".to_owned()));
        let resp = handle_request(&req("tools/list", json!({})), &ctx).unwrap();
        let names: Vec<String> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.iter().any(|n| n == "list_files"));
        assert!(names.iter().any(|n| n == "read_code"));
    }

    #[test]
    fn test_run_query_ok() {
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let ctx = ServerCtx::new(path, None);
        let (text, is_error) = call(&ctx, "run_query", json!({ "sql": "SELECT COUNT(*) AS n FROM items" }));
        assert!(!is_error, "benign query should succeed: {text}");
        assert!(text.starts_with('['), "expected a JSON array, got {text}");
        assert!(text.contains("\"n\""), "expected the `n` alias, got {text}");
    }

    #[test]
    fn test_run_query_exfil_blocked() {
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let ctx = ServerCtx::new(path, None);
        // The exfil gate must hold end-to-end through the MCP route.
        let (text, is_error) =
            call(&ctx, "run_query", json!({ "sql": "SELECT content FROM read_text('/etc/hosts')" }));
        assert!(is_error, "external file read must be an error");
        assert!(!text.contains("localhost"), "must NOT leak /etc/hosts contents: {text}");
    }

    #[test]
    fn test_find_blockers_routes_to_chain() {
        // PROVENANCE: the authoritative pin for this chain is tools.rs
        // test_find_blockers_cycle_guard — bg4N2, chain uid 48 -> … -> root uid 1
        // ("External Thread"), cycle-guarded. This smoke test only confirms the MCP
        // routes start_uid through find_blockers_sql; it does not re-derive "7".
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let ctx = ServerCtx::new(path, None);
        let (text, is_error) = call(&ctx, "find_blockers", json!({ "start_uid": 48 }));
        assert!(!is_error, "find_blockers should succeed: {text}");
        let rows: Vec<Value> = serde_json::from_str(&text).expect("find_blockers returns a JSON array");
        assert_eq!(rows.len(), 7, "find_blockers(48) should route to the 7-row chain");
    }

    #[test]
    fn test_final_answer_validation() {
        let ctx = dummy_ctx();

        // Valid: echoes the normalized {answer_type, value}.
        let (text, is_error) = call(&ctx, "final_answer", json!({ "answer_type": "uid", "value": 221 }));
        assert!(!is_error);
        assert!(
            text.contains("\"answer_type\":\"uid\"") && text.contains("221"),
            "should echo the answer, got {text}"
        );

        // Missing value -> error.
        let (_t, is_error) = call(&ctx, "final_answer", json!({ "answer_type": "uid" }));
        assert!(is_error, "missing value must be rejected");

        // answer_type not in the enum -> error.
        let (_t, is_error) = call(&ctx, "final_answer", json!({ "answer_type": "bogus", "value": 1 }));
        assert!(is_error, "bogus answer_type must be rejected");
    }
}
