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
const HEADLESS_TOOLS: &[&str] = &["run_query", "overview", "list_files", "read_code"];

/// The VISUAL tools advertised ONLY when a [`UiBridge`](super::bridge::UiBridge) is
/// present (the in-viewer HTTP server). They drive the live timeline over the
/// bridge. `ask_user` (no human at the MCP end) and `update_findings`
/// (embedded-only scratchpad) are intentionally NOT exposed. (V1.2)
const VISUAL_TOOLS: &[&str] = &[
    "screenshot",
    "zoom_to",
    "pan",
    "scroll_to",
    "set_view",
    "search",
    "reset_view",
    "highlight",
    "clear_highlights",
];

/// The JIT wiki tools advertised ONLY when a `wiki_root` is configured. They give
/// the client on-demand retrieval over the Legion knowledge wiki. (wiki-tool)
const WIKI_TOOLS: &[&str] = &["wiki_index", "wiki_read", "wiki_search"];

/// Valid `answer_type` values for the `final_answer` tool (the eval grader pins
/// this enum).
const ANSWER_TYPES: &[&str] = &["uid", "number", "set", "label", "tuple", "diagnosis"];

/// Server context: which case DB to query, an optional source root for the code
/// tools, the protocol version this transport advertises, and an optional
/// [`UiBridge`](super::bridge::UiBridge) to drive the live viewer. Held immutably
/// across requests (the bridge's `request` takes `&self`).
pub struct ServerCtx {
    pub duckdb_path: String,
    pub code_root: Option<String>,
    /// Legion wiki root. When set, the `wiki_*` tools are advertised + routed
    /// (mirrors `code_root` gating `read_code`). (wiki-tool)
    pub wiki_root: Option<String>,
    pub protocol_version: &'static str,
    /// Present only on the in-viewer HTTP server: enables the VISUAL tools, which
    /// drive the live timeline over this handle. Absent (stdio bin / eval) =>
    /// data tools only, unchanged.
    pub ui_bridge: Option<super::bridge::UiBridge>,
    /// Bearer token the HTTP transport requires on every `POST /mcp` (server
    /// hardening — closes the "any local process can drive the tools" hole).
    /// `None` (stdio bin / direct dispatch in tests) => no HTTP-layer auth; the
    /// stdio transport has no network exposure so it never sets one. Enforced
    /// ONLY in `viewer_mcp::handle_http_request`, never in this dispatch core.
    pub auth_token: Option<String>,
}

impl ServerCtx {
    /// Construct a context with the default (stdio) protocol version and NO UI
    /// bridge (data tools only).
    pub fn new(duckdb_path: String, code_root: Option<String>) -> Self {
        ServerCtx {
            duckdb_path,
            // Normalize an empty code root to None (consistent with `with_wiki_root`)
            // so the source clause / source-line / code tools are never gated on "".
            code_root: code_root.filter(|r| !r.is_empty()),
            wiki_root: None,
            protocol_version: DEFAULT_PROTOCOL_VERSION,
            ui_bridge: None,
            auth_token: None,
        }
    }

    /// Require `Authorization: Bearer <token>` at the HTTP transport layer
    /// (server hardening). No effect on stdio / direct dispatch.
    pub fn with_auth_token(mut self, token: Option<String>) -> Self {
        self.auth_token = token.filter(|t| !t.is_empty());
        self
    }

    /// Override the advertised protocol version (the HTTP transport uses this).
    pub fn with_protocol(mut self, version: &'static str) -> Self {
        self.protocol_version = version;
        self
    }

    /// Attach a Legion wiki root, enabling the `wiki_*` tools. (wiki-tool)
    pub fn with_wiki_root(mut self, wiki_root: Option<String>) -> Self {
        self.wiki_root = wiki_root.filter(|r| !r.is_empty());
        self
    }

    /// Attach a [`UiBridge`](super::bridge::UiBridge), enabling the VISUAL tools.
    /// Used by the in-viewer HTTP server (wired live in V1.3).
    pub fn with_ui_bridge(mut self, bridge: super::bridge::UiBridge) -> Self {
        self.ui_bridge = Some(bridge);
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

/// Build the MCP `instructions` briefing (the designed server→client channel an
/// external agent reads on connect). Always includes the framing; the source-root
/// and wiki clauses appear ONLY when those roots are configured. Generic — no task
/// names and no absolute paths beyond the configured roots themselves.
fn server_instructions(ctx: &ServerCtx) -> String {
    let mut parts: Vec<String> = vec![
        "Legion Profiler Co-Pilot — diagnose Legion task-based runtime performance from \
         this profile. Verify every number with run_query before stating it; rank issues \
         by share of total time, not ratios; state root causes as hypotheses, not \
         certainties; never invent speedups. Before ANY sizing/config verdict (e.g. \
         'the mesh/problem is under-sized'), derive the observed size from the profile \
         (the overview's Data-Size Evidence: per-copy and total bytes moved) and \
         reconcile — if the observed sizes contradict the hypothesis, say so instead of \
         asserting it."
            .to_owned(),
    ];
    if let Some(code_root) = &ctx.code_root {
        parts.push(format!(
            "Application source root: `{code_root}`. Read the relevant task's source (your \
             own file tools, or read_code/list_files) BEFORE explaining what a kernel \
             computes or why it is slow."
        ));
    }
    if ctx.wiki_root.is_some() {
        parts.push(
            "A curated Legion wiki is available via wiki_index / wiki_read / wiki_search. \
             BEFORE asserting a Legion concept, a flag's meaning, or a diagnostic verdict \
             (compute-/communication-/runtime-bound, mapper stall, lifecycle phases such as \
             waiting vs deferred), consult it and follow the page's Related links."
                .to_owned(),
        );
    }
    parts.join("\n\n")
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
        "serverInfo": { "name": "legion-prof", "version": env!("CARGO_PKG_VERSION") },
        "instructions": server_instructions(ctx)
    })
}

/// Build the advertised tool list: the headless subset of `tool_definitions`
/// (with `input_schema` renamed to MCP's `inputSchema` here in the dispatch core,
/// never in `tools.rs`), plus the inline `find_blockers` and `final_answer`
/// definitions. Code tools are omitted unless a `code_root` was configured.
fn tools_list_result(ctx: &ServerCtx) -> Value {
    let has_code = ctx.code_root.is_some();
    let has_wiki = ctx.wiki_root.is_some();
    let has_bridge = ctx.ui_bridge.is_some();
    let mut tools: Vec<Value> = tool_definitions(true, true, true)
        .into_iter()
        .filter(|t| {
            let name = t.get("name").and_then(Value::as_str).unwrap_or("");
            // Data tools (code tools only with a code root) ...
            (HEADLESS_TOOLS.contains(&name)
                && (has_code || (name != "list_files" && name != "read_code")))
                // ... the JIT wiki tools when a wiki root is configured ...
                || (has_wiki && WIKI_TOOLS.contains(&name))
                // ... plus the VISUAL tools when a UI bridge is attached.
                || (has_bridge && VISUAL_TOOLS.contains(&name))
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

    // V1.4: get_selection is a non-driving READ, advertised only with a UI bridge
    // (like the visual tools). No args.
    if has_bridge {
        tools.push(json!({
            "name": "get_selection",
            "description": "Read the human's CURRENT timeline selection in the live viewer — the \
                            task bar(s) and/or dragged time range they have selected. Use this to \
                            resolve \"this\", \"that task\", \"here\" to concrete identifiers \
                            before querying. Returns selected_items [{item_uid, entry_slug, title, \
                            start_ns, stop_ns}] and selected_range {entry_label, start_ns, stop_ns} \
                            (or an explicit note when nothing is selected). Does NOT change the view.",
            "inputSchema": { "type": "object", "properties": {}, "required": [] }
        }));
    }

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

    // VISUAL tools drive the live viewer over the UI bridge (present only on the
    // in-viewer HTTP server). A "viewport busy"/timeout from the bridge is a tool
    // RESULT with isError:true (model-readable), not a protocol error.
    if VISUAL_TOOLS.contains(&name) {
        return match &ctx.ui_bridge {
            Some(bridge) => visual_tool_result(name, &args, bridge, &ctx.duckdb_path),
            None => text_result(
                &format!("visual tool '{name}' is unavailable: this server has no UI bridge."),
                true,
            ),
        };
    }

    // get_selection (V1.4): a non-driving READ over the bridge (no viewport claim).
    if name == "get_selection" {
        return match &ctx.ui_bridge {
            Some(bridge) => get_selection_result(bridge),
            None => text_result("get_selection is unavailable: this server has no UI bridge.", true),
        };
    }

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
        "overview" => {
            // Reinforce the source-read habit with ONE concise line (no multi-line
            // section — heeds the gather_overview-overflow lesson). gather_overview's
            // signature is untouched; we only append here, in the MCP handler.
            let mut res = into_tool_result(gather_overview(&ctx.duckdb_path));
            if let (false, Some(code_root)) = (res.1, &ctx.code_root) {
                res.0.push_str(&format!(
                    "\nSource root: `{code_root}` — read the relevant task's source before \
                     explaining what a kernel does or why it is slow."
                ));
            }
            res
        }
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
        "wiki_index" => match &ctx.wiki_root {
            Some(root) => {
                let section = args.get("section").and_then(Value::as_str);
                into_tool_result(super::tools::wiki_index(root, section))
            }
            None => (
                "wiki_index unavailable: server started without a wiki root.".to_owned(),
                true,
            ),
        },
        "wiki_read" => match &ctx.wiki_root {
            Some(root) => match args.get("path").and_then(Value::as_str) {
                Some(path) => {
                    let section = args.get("section").and_then(Value::as_str);
                    let max_chars = args
                        .get("max_chars")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize);
                    into_tool_result(super::tools::wiki_read(root, path, section, max_chars))
                }
                None => ("wiki_read requires path (string).".to_owned(), true),
            },
            None => (
                "wiki_read unavailable: server started without a wiki root.".to_owned(),
                true,
            ),
        },
        "wiki_search" => match &ctx.wiki_root {
            Some(root) => match args.get("query").and_then(Value::as_str) {
                Some(query) => {
                    let section = args.get("section").and_then(Value::as_str);
                    let tag = args.get("tag").and_then(Value::as_str);
                    let limit = args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize)
                        .unwrap_or(5);
                    into_tool_result(super::tools::wiki_search(root, query, section, tag, limit))
                }
                None => ("wiki_search requires query (string).".to_owned(), true),
            },
            None => (
                "wiki_search unavailable: server started without a wiki root.".to_owned(),
                true,
            ),
        },
        "final_answer" => final_answer_result(&args),
        other => (format!("unknown tool: {other}"), true),
    };

    text_result(&text, is_error)
}

/// An MCP `tools/call` result with a single text content block.
fn text_result(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

/// Execute a VISUAL tool by translating its args into the corresponding
/// [`AgentEvent`](super::agent::AgentEvent) (mirroring `agent.rs` `execute_tool`'s
/// GUI arms), driving the live viewer over `bridge`, and translating the reply:
/// a screenshot -> an MCP image block; an `Ack` -> text; a `viewport busy`/timeout
/// -> `isError:true`. Typed args only (start_uid/entry_slug/range) — no model SQL.
fn visual_tool_result(
    name: &str,
    args: &Value,
    bridge: &super::bridge::UiBridge,
    duckdb_path: &str,
) -> Value {
    use super::agent::{AgentEvent, UiCommand};
    use super::bridge::DEFAULT_REQUEST_TIMEOUT;

    let i64_arg = |k: &str| args.get(k).and_then(Value::as_i64);
    let str_array = |k: &str| -> Option<Vec<String>> {
        args.get(k).and_then(|v| v.as_array()).map(|a| {
            a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()
        })
    };

    // Build the event for this tool (or return an arg-error result). request_id is
    // injected by the bridge.
    let builder: Box<dyn FnOnce(u64) -> AgentEvent> = match name {
        "screenshot" => Box::new(|rid| AgentEvent::ScreenshotRequest { request_id: rid }),

        "zoom_to" => {
            let (Some(start_ns), Some(stop_ns)) = (i64_arg("start_ns"), i64_arg("stop_ns")) else {
                return text_result("zoom_to requires start_ns and stop_ns (integers).", true);
            };
            Box::new(move |rid| AgentEvent::ZoomRequest { request_id: rid, start_ns, stop_ns })
        }

        "pan" => {
            let Some(direction) = args.get("direction").and_then(Value::as_str) else {
                return text_result("pan requires direction (\"left\" or \"right\").", true);
            };
            if direction != "left" && direction != "right" {
                return text_result("pan direction must be \"left\" or \"right\".", true);
            }
            let percent = args.get("percent").and_then(Value::as_f64).unwrap_or(25.0).clamp(1.0, 200.0);
            let direction = direction.to_owned();
            Box::new(move |rid| AgentEvent::PanRequest { request_id: rid, direction, percent })
        }

        "scroll_to" => {
            let Some(slug) = args.get("entry_slug").and_then(Value::as_str) else {
                return text_result("scroll_to requires entry_slug (string).", true);
            };
            let entry_slug = slug.to_owned();
            Box::new(move |rid| AgentEvent::ScrollToRequest { request_id: rid, entry_slug })
        }

        "set_view" => {
            let (Some(start_ns), Some(stop_ns)) = (i64_arg("start_ns"), i64_arg("stop_ns")) else {
                return text_result("set_view requires start_ns and stop_ns (integers).", true);
            };
            let entry_slug = args.get("entry_slug").and_then(Value::as_str).map(str::to_owned);
            let filter_kinds = str_array("filter_kinds");
            let expand_kinds = str_array("expand_kinds");
            let collapse_kinds = str_array("collapse_kinds");
            let vertical_scale = args.get("vertical_scale").and_then(Value::as_f64);
            Box::new(move |rid| AgentEvent::SetViewRequest {
                request_id: rid,
                start_ns,
                stop_ns,
                entry_slug,
                filter_kinds,
                expand_kinds,
                collapse_kinds,
                vertical_scale,
            })
        }

        "search" => {
            let Some(query) = args.get("query").and_then(Value::as_str) else {
                return text_result("search requires query (string).", true);
            };
            let query = query.to_owned();
            Box::new(move |rid| AgentEvent::SearchRequest { request_id: rid, query })
        }

        "reset_view" => Box::new(|rid| AgentEvent::ResetViewRequest { request_id: rid }),

        "highlight" => {
            let Some(slug) = args.get("entry_slug").and_then(Value::as_str) else {
                return text_result("highlight requires entry_slug (string).", true);
            };
            let (Some(start_ns), Some(stop_ns)) = (i64_arg("start_ns"), i64_arg("stop_ns")) else {
                return text_result("highlight requires start_ns and stop_ns (integers).", true);
            };
            // P0(b) parity: reject an unknown slug (same check the embedded agent
            // uses) BEFORE driving the UI — an invalid highlight is a tool error,
            // not a silent no-op overlay.
            if !super::tools::slug_exists(duckdb_path, slug) {
                return text_result(
                    &format!(
                        "highlight: unknown entry_slug '{slug}'. Query \
                         `SELECT entry_slug FROM entries` for valid slugs."
                    ),
                    true,
                );
            }
            let severity = args.get("severity").and_then(Value::as_str).unwrap_or("medium").to_owned();
            let label = args.get("label").and_then(Value::as_str).unwrap_or("").to_owned();
            let entry_slug = slug.to_owned();
            Box::new(move |rid| AgentEvent::HighlightRequest {
                request_id: rid,
                entry_slug,
                start_ns,
                stop_ns,
                severity,
                label,
            })
        }

        "clear_highlights" => Box::new(|rid| AgentEvent::ClearHighlightsRequest { request_id: rid }),

        other => return text_result(&format!("unknown visual tool: {other}"), true),
    };

    // Drive the live viewer, matching the reply by the bridge-assigned request_id.
    // The viewport token guarantees a single outstanding request, but matching by
    // id is still robust against a stale reply from a prior timed-out request.
    let rid = std::cell::Cell::new(u64::MAX);
    let reply = bridge.request(
        |id| {
            rid.set(id);
            builder(id)
        },
        |cmd| reply_request_id(cmd) == Some(rid.get()),
        DEFAULT_REQUEST_TIMEOUT,
    );

    match reply {
        Ok(UiCommand::ScreenshotData { png_bytes, metadata, .. }) => {
            screenshot_result(&png_bytes, &metadata)
        }
        Ok(UiCommand::Ack { message, .. }) => text_result(&message, false),
        Ok(other) => text_result(&format!("unexpected viewport reply: {other:?}"), true),
        // "viewport busy" / timeout / disconnect -> model-readable tool error.
        Err(e) => text_result(&e, true),
    }
}

/// Execute `get_selection` (V1.4): a non-driving READ of the human's timeline
/// selection over the bridge. Uses `request_read` — it does NOT claim the viewport
/// token, so it succeeds even while a driver holds the viewport. Formats
/// `SelectionData` as structured JSON, or an explicit note when nothing is selected.
fn get_selection_result(bridge: &super::bridge::UiBridge) -> Value {
    use super::agent::{AgentEvent, UiCommand};
    use super::bridge::DEFAULT_REQUEST_TIMEOUT;

    let rid = std::cell::Cell::new(u64::MAX);
    let reply = bridge.request_read(
        |id| {
            rid.set(id);
            AgentEvent::GetSelection { request_id: id }
        },
        |cmd| reply_request_id(cmd) == Some(rid.get()),
        DEFAULT_REQUEST_TIMEOUT,
    );

    match reply {
        Ok(UiCommand::SelectionData { items, range, .. }) => {
            if items.is_empty() && range.is_none() {
                return text_result(
                    "Nothing is selected in the viewer. Ask the user to click a task bar or \
                     shift-drag a time range, then call get_selection again.",
                    false,
                );
            }
            let items_json: Vec<Value> = items
                .iter()
                .map(|it| {
                    json!({
                        "item_uid": it.item_uid,
                        "entry_slug": it.entry_slug,
                        "title": it.title,
                        "start_ns": it.start_ns,
                        "stop_ns": it.stop_ns,
                    })
                })
                .collect();
            let range_json = range.as_ref().map(|(label, start, stop)| {
                json!({ "entry_label": label, "start_ns": start, "stop_ns": stop })
            });
            let payload = json!({
                "selected_items": items_json,
                "selected_range": range_json,
            });
            text_result(&payload.to_string(), false)
        }
        Ok(other) => text_result(&format!("unexpected selection reply: {other:?}"), true),
        Err(e) => text_result(&e, true),
    }
}

/// The request_id carried by any [`UiCommand`](super::agent::UiCommand) reply.
fn reply_request_id(cmd: &super::agent::UiCommand) -> Option<u64> {
    use super::agent::UiCommand;
    match cmd {
        UiCommand::ScreenshotData { request_id, .. }
        | UiCommand::UserAnswer { request_id, .. }
        | UiCommand::Ack { request_id, .. }
        | UiCommand::SelectionData { request_id, .. } => Some(*request_id),
    }
}

/// Build an MCP image content block from RAW PNG bytes. Claude Code requires the
/// `data` field to be BARE base64 (no `data:image/png;base64,` URI prefix). We
/// encode the raw bytes ourselves, so it is bare by construction; the prefix strip
/// is a defensive guard in case a future reply path ever carries a data-URI string.
/// The visible time-range / entry-slug metadata rides along as a text block so the
/// model can issue follow-up zoom/queries.
fn screenshot_result(png_bytes: &[u8], metadata: &str) -> Value {
    // Parity with the embedded path (wait_for_screenshot): an empty capture is a
    // tool error, not a degenerate empty image block.
    if png_bytes.is_empty() {
        return text_result("screenshot capture returned empty data.", true);
    }
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes);
    let bare = encoded.strip_prefix("data:image/png;base64,").unwrap_or(&encoded);
    json!({
        "content": [
            { "type": "image", "data": bare, "mimeType": "image/png" },
            { "type": "text", "text": metadata }
        ],
        "isError": false
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

    /// initialize_result must carry an `instructions` briefing; the source-root and
    /// wiki clauses are conditional on those roots being configured.
    #[test]
    fn test_initialize_instructions_both_roots() {
        let ctx = ServerCtx::new("db".to_owned(), Some("/app/src".to_owned()))
            .with_wiki_root(Some("/wiki".to_owned()));
        let resp = handle_request(&req("initialize", json!({})), &ctx).unwrap();
        let instr = resp["result"]["instructions"].as_str().expect("instructions present");
        assert!(!instr.is_empty(), "instructions must be non-empty");
        // Always-framing.
        assert!(instr.contains("Legion Profiler Co-Pilot"), "missing framing");
        assert!(instr.contains("run_query"), "missing the verify-with-run_query rule");
        // Both conditional clauses present, with the code_root path interpolated.
        assert!(instr.contains("/app/src"), "code_root path not interpolated");
        assert!(instr.contains("Application source root"), "missing source clause");
        assert!(instr.contains("wiki_index"), "missing wiki clause");
    }

    #[test]
    fn test_initialize_instructions_no_roots() {
        // dummy_ctx() has code_root=None, wiki_root=None.
        let resp = handle_request(&req("initialize", json!({})), &dummy_ctx()).unwrap();
        let instr = resp["result"]["instructions"].as_str().expect("instructions present");
        assert!(instr.contains("Legion Profiler Co-Pilot"), "framing must still be present");
        assert!(!instr.contains("Application source root"), "no source clause without code_root");
        assert!(!instr.contains("wiki_index"), "no wiki clause without wiki_root");
        assert!(!instr.contains("curated Legion wiki"), "no wiki clause without wiki_root");
        // MiniAero guardrail (verify-verdict-vs-data): sizing claims must be
        // reconciled against the overview's Data-Size Evidence.
        assert!(
            instr.contains("Data-Size Evidence") && instr.contains("reconcile"),
            "sizing-verdict guardrail must brief external agents"
        );
    }

    #[test]
    fn test_initialize_instructions_single_root() {
        // Only wiki configured.
        let wiki_only = ServerCtx::new("db".to_owned(), None).with_wiki_root(Some("/wiki".to_owned()));
        let i = handle_request(&req("initialize", json!({})), &wiki_only).unwrap()["result"]
            ["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(i.contains("wiki_index"), "wiki clause expected");
        assert!(!i.contains("Application source root"), "no source clause without code_root");

        // Only code configured (vice-versa).
        let code_only = ServerCtx::new("db".to_owned(), Some("/only/code".to_owned()));
        let j = handle_request(&req("initialize", json!({})), &code_only).unwrap()["result"]
            ["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(j.contains("/only/code"), "source clause with path expected");
        assert!(!j.contains("wiki_index"), "no wiki clause without wiki_root");
    }

    /// The MCP `overview` handler appends a one-line source-root reminder iff a
    /// code root is configured. Needs a live DuckDB; soft-skips if absent.
    #[test]
    fn test_overview_appends_source_line_with_code_root() {
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let with_code = ServerCtx::new(path.clone(), Some("/app/src".to_owned()));
        let (text, is_error) = call(&with_code, "overview", json!({}));
        assert!(!is_error, "overview should succeed: {text:.80}");
        assert!(
            text.contains("Source root: `/app/src`"),
            "overview missing the source-root line with a code root"
        );

        let no_code = ServerCtx::new(path, None);
        let (text2, is_error2) = call(&no_code, "overview", json!({}));
        assert!(!is_error2, "overview should succeed: {text2:.80}");
        assert!(!text2.contains("Source root:"), "source-root line must be absent without a code root");
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
    fn test_tools_list_wiki_gated_on_wiki_root() {
        // WITHOUT a wiki root: the wiki tools are not advertised.
        let names_no_wiki: Vec<String> = handle_request(&req("tools/list", json!({})), &dummy_ctx())
            .unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        for w in ["wiki_index", "wiki_read", "wiki_search"] {
            assert!(!names_no_wiki.contains(&w.to_owned()), "wiki tool {w} leaked without a wiki root");
        }

        // WITH a wiki root: all three appear, with camelCase inputSchema (no leak).
        let ctx = ServerCtx::new("unused".to_owned(), None).with_wiki_root(Some("/tmp".to_owned()));
        let tools = handle_request(&req("tools/list", json!({})), &ctx).unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .clone();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for w in ["wiki_index", "wiki_read", "wiki_search"] {
            assert!(names.contains(&w), "tools/list missing {w} with a wiki root");
        }
        for t in tools.iter().filter(|t| {
            let n = t["name"].as_str().unwrap();
            n == "wiki_index" || n == "wiki_read" || n == "wiki_search"
        }) {
            assert!(t.get("inputSchema").is_some(), "{} missing inputSchema", t["name"]);
            assert!(t.get("input_schema").is_none(), "{} leaked input_schema", t["name"]);
        }
    }

    #[test]
    fn test_wiki_call_routes_to_index() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../wiki-legion/wiki");
        if !p.is_dir() {
            eprintln!("skipping: wiki root absent");
            return;
        }
        let ctx = ServerCtx::new("unused".to_owned(), None)
            .with_wiki_root(Some(p.to_string_lossy().into_owned()));
        let (text, is_error) = call(&ctx, "wiki_index", json!({ "section": "pitfalls" }));
        assert!(!is_error, "wiki_index should succeed: {text}");
        assert!(text.contains("## pitfalls ("), "wiki_index output unexpected: {text:.80}");

        // Routed without a wiki root => a clear isError result, not a protocol error.
        let (text2, is_error2) = call(&dummy_ctx(), "wiki_index", json!({}));
        assert!(is_error2, "wiki_index without a wiki root must be an error result");
        assert!(text2.contains("without a wiki root"), "unexpected msg: {text2}");
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

    // ── V1.2 visual tools (stub UiBridge — no live window) ───────────────────
    use crate::ai::agent::{AgentEvent, SelectedItemInfo, UiCommand};
    use crate::ai::bridge::{UiBridge, ViewportGuard, ViewportToken, MCP_CONSUMER_ID};
    use std::sync::mpsc::channel;

    /// The request_id any of the visual / read events carries (test helper).
    fn event_request_id(ev: &AgentEvent) -> u64 {
        match ev {
            AgentEvent::ScreenshotRequest { request_id }
            | AgentEvent::ZoomRequest { request_id, .. }
            | AgentEvent::PanRequest { request_id, .. }
            | AgentEvent::ScrollToRequest { request_id, .. }
            | AgentEvent::SetViewRequest { request_id, .. }
            | AgentEvent::SearchRequest { request_id, .. }
            | AgentEvent::ResetViewRequest { request_id }
            | AgentEvent::HighlightRequest { request_id, .. }
            | AgentEvent::ClearHighlightsRequest { request_id }
            | AgentEvent::GetSelection { request_id } => *request_id,
            other => panic!("unexpected visual event: {other:?}"),
        }
    }

    /// A ctx with a bridge whose UI-side channels dangle — fine for `tools/list`
    /// and for arms that reject BEFORE driving the bridge (bad args / unknown slug).
    fn ctx_with_dangling_bridge(duckdb_path: &str) -> ServerCtx {
        let (event_tx, _event_rx) = channel::<AgentEvent>();
        let (_cmd_tx, cmd_rx) = channel::<UiCommand>();
        let bridge = UiBridge::new(event_tx, cmd_rx, ViewportToken::new(), MCP_CONSUMER_ID);
        ServerCtx::new(duckdb_path.to_owned(), None).with_ui_bridge(bridge)
    }

    /// A ctx with a bridge + a stub UI thread that handles ONE event and replies
    /// with `reply(&event)`. The join handle yields the event the server emitted,
    /// for assertions.
    fn ctx_with_stub_ui(
        duckdb_path: String,
        reply: impl Fn(&AgentEvent) -> UiCommand + Send + 'static,
    ) -> (ServerCtx, std::thread::JoinHandle<AgentEvent>) {
        let (event_tx, event_rx) = channel::<AgentEvent>();
        let (cmd_tx, cmd_rx) = channel::<UiCommand>();
        let bridge = UiBridge::new(event_tx, cmd_rx, ViewportToken::new(), MCP_CONSUMER_ID);
        let handle = std::thread::spawn(move || {
            let ev = event_rx.recv().expect("stub UI: event");
            let _ = cmd_tx.send(reply(&ev));
            ev
        });
        (ServerCtx::new(duckdb_path, None).with_ui_bridge(bridge), handle)
    }

    #[test]
    fn test_tools_list_visual_only_with_bridge() {
        // WITHOUT a bridge: the 9 visual tools are NOT advertised (regression — the
        // stdio path is unchanged).
        let resp = handle_request(&req("tools/list", json!({})), &dummy_ctx()).unwrap();
        let none: Vec<&str> =
            resp["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for v in VISUAL_TOOLS {
            assert!(!none.contains(v), "no-bridge tools/list must NOT advertise {v}");
        }

        // WITH a bridge: all 9 visual tools advertised, alongside the data tools.
        let resp = handle_request(&req("tools/list", json!({})), &ctx_with_dangling_bridge("unused")).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for v in VISUAL_TOOLS {
            assert!(names.contains(v), "bridge tools/list must advertise visual tool {v}");
        }
        for want in ["run_query", "overview", "final_answer"] {
            assert!(names.contains(&want), "data tool {want} still present");
        }
        // ask_user / update_findings are NEVER exposed over MCP.
        assert!(!names.contains(&"ask_user"), "ask_user must not be exposed");
        assert!(!names.contains(&"update_findings"), "update_findings must not be exposed");
        // camelCase inputSchema, no snake_case leak (incl. the visual tools).
        for t in tools {
            assert!(t.get("inputSchema").is_some(), "tool {} missing inputSchema", t["name"]);
            assert!(t.get("input_schema").is_none(), "tool {} leaked input_schema", t["name"]);
        }
    }

    #[test]
    fn test_visual_highlight_valid_slug_routes_and_acks() {
        // Needs the real DB: the highlight arm validates the slug (P0(b) parity)
        // against `entries` BEFORE driving the UI.
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        // The stub UI asserts the event is a HighlightRequest with the right typed
        // fields, then ACKs.
        let (ctx, handle) = ctx_with_stub_ui(path, |ev| {
            let rid = event_request_id(ev);
            match ev {
                AgentEvent::HighlightRequest { entry_slug, start_ns, stop_ns, severity, .. } => {
                    assert_eq!(entry_slug, "n0_cpu_c1");
                    assert_eq!((*start_ns, *stop_ns), (100, 200));
                    assert_eq!(severity, "high");
                }
                other => panic!("expected HighlightRequest, got {other:?}"),
            }
            UiCommand::Ack { request_id: rid, message: "Highlight added on n0_cpu_c1.".into() }
        });

        let (text, is_error) = call(
            &ctx,
            "highlight",
            json!({ "entry_slug": "n0_cpu_c1", "start_ns": 100, "stop_ns": 200, "severity": "high" }),
        );
        assert!(!is_error, "valid-slug highlight should ACK success: {text}");
        assert!(text.contains("Highlight added"), "ACK text echoed: {text}");
        handle.join().unwrap();
    }

    #[test]
    fn test_visual_highlight_unknown_slug_rejected() {
        // P0(b): an unknown slug is rejected as a tool error BEFORE the bridge is
        // driven (dangling bridge proves no event is sent — else it would block).
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let ctx = ctx_with_dangling_bridge(&path);
        let (text, is_error) = call(
            &ctx,
            "highlight",
            json!({ "entry_slug": "n0_not_a_real_slug", "start_ns": 1, "stop_ns": 2 }),
        );
        assert!(is_error, "unknown slug must be a tool error, not a silent no-op");
        assert!(text.contains("unknown entry_slug"), "actionable message: {text}");
    }

    #[test]
    fn test_visual_screenshot_returns_image_block() {
        let png = vec![0x89, 0x50, 0x4E, 0x47]; // \x89PNG magic
        let png_for_thread = png.clone();
        let (ctx, handle) = ctx_with_stub_ui("unused".into(), move |ev| {
            assert!(matches!(ev, AgentEvent::ScreenshotRequest { .. }), "expected ScreenshotRequest");
            UiCommand::ScreenshotData {
                request_id: event_request_id(ev),
                png_bytes: png_for_thread.clone(),
                metadata: "range=[0,1000] slugs=[n0_gpu_g0]".into(),
            }
        });

        let resp = handle_request(
            &req("tools/call", json!({ "name": "screenshot", "arguments": {} })),
            &ctx,
        )
        .unwrap();
        let result = &resp["result"];
        assert_eq!(result["isError"], false);
        let content = result["content"].as_array().unwrap();
        // First block is the image: bare base64 of the PNG, mimeType image/png.
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["mimeType"], "image/png");
        use base64::Engine;
        let want_b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        assert_eq!(content[0]["data"], want_b64, "image data is bare base64 of the PNG");
        assert!(!content[0]["data"].as_str().unwrap().starts_with("data:"), "no data-URI prefix");
        // Metadata rides along as a text block for follow-up queries.
        assert_eq!(content[1]["type"], "text");
        assert!(content[1]["text"].as_str().unwrap().contains("range="));
        handle.join().unwrap();
    }

    #[test]
    fn test_visual_viewport_busy_is_tool_error() {
        // Another consumer holds the viewport -> the bridge's request returns
        // "viewport busy", surfaced as a tool RESULT with isError:true (not a
        // protocol error) so the model can read it and retry.
        let (event_tx, _event_rx) = channel::<AgentEvent>();
        let (_cmd_tx, cmd_rx) = channel::<UiCommand>();
        let token = ViewportToken::new();
        let _held: ViewportGuard = token.try_claim(99).unwrap(); // someone else owns it
        let bridge = UiBridge::new(event_tx, cmd_rx, token, MCP_CONSUMER_ID);
        let ctx = ServerCtx::new("unused".into(), None).with_ui_bridge(bridge);

        let (text, is_error) = call(&ctx, "screenshot", json!({}));
        assert!(is_error, "viewport busy must be a tool error");
        assert!(text.contains("viewport busy"), "busy message is model-readable: {text}");
    }

    #[test]
    fn test_visual_screenshot_empty_is_tool_error() {
        // Parity with the embedded path: an empty PNG capture -> tool error, not a
        // degenerate empty image block.
        let (ctx, handle) = ctx_with_stub_ui("unused".into(), |ev| UiCommand::ScreenshotData {
            request_id: event_request_id(ev),
            png_bytes: vec![],
            metadata: String::new(),
        });
        let (text, is_error) = call(&ctx, "screenshot", json!({}));
        assert!(is_error, "empty screenshot must be a tool error");
        assert!(text.contains("empty"), "actionable message: {text}");
        handle.join().unwrap();
    }

    #[test]
    fn test_visual_bad_args_is_tool_error() {
        // zoom_to without the required integers -> tool error, and the bridge is
        // never driven (stub UI thread would block forever, so use a dangling one).
        let ctx = ctx_with_dangling_bridge("unused");
        let (text, is_error) = call(&ctx, "zoom_to", json!({ "start_ns": 5 }));
        assert!(is_error, "missing stop_ns must be a tool error");
        assert!(text.contains("zoom_to requires"), "actionable message: {text}");
    }

    // ── V1.4 get_selection (READ) ────────────────────────────────────────────
    #[test]
    fn test_get_selection_formats_json() {
        // Stub UI asserts the event is GetSelection, replies with a seeded selection.
        let (ctx, handle) = ctx_with_stub_ui("unused".into(), |ev| {
            assert!(matches!(ev, AgentEvent::GetSelection { .. }), "expected GetSelection");
            UiCommand::SelectionData {
                request_id: event_request_id(ev),
                items: vec![SelectedItemInfo {
                    item_uid: 48,
                    entry_slug: Some("n0_cpu_c1".into()),
                    title: "top_level <6>".into(),
                    start_ns: 100,
                    stop_ns: 200,
                }],
                range: Some(("CPU 1".into(), 50, 300)),
            }
        });
        let (text, is_error) = call(&ctx, "get_selection", json!({}));
        assert!(!is_error, "get_selection should succeed: {text}");
        let v: Value = serde_json::from_str(&text).expect("structured JSON");
        assert_eq!(v["selected_items"][0]["item_uid"], 48);
        assert_eq!(v["selected_items"][0]["entry_slug"], "n0_cpu_c1");
        assert_eq!(v["selected_items"][0]["title"], "top_level <6>");
        assert_eq!(v["selected_items"][0]["start_ns"], 100);
        assert_eq!(v["selected_range"]["entry_label"], "CPU 1");
        assert_eq!(v["selected_range"]["start_ns"], 50);
        assert_eq!(v["selected_range"]["stop_ns"], 300);
        handle.join().unwrap();
    }

    #[test]
    fn test_get_selection_nothing_selected() {
        let (ctx, handle) = ctx_with_stub_ui("unused".into(), |ev| UiCommand::SelectionData {
            request_id: event_request_id(ev),
            items: vec![],
            range: None,
        });
        let (text, is_error) = call(&ctx, "get_selection", json!({}));
        assert!(!is_error, "nothing-selected is not an error");
        assert!(text.contains("Nothing is selected"), "explicit empty note: {text}");
        handle.join().unwrap();
    }

    #[test]
    fn test_tools_list_get_selection_gating() {
        // WITHOUT a bridge (stdio path): get_selection is NOT advertised.
        let resp = handle_request(&req("tools/list", json!({})), &dummy_ctx()).unwrap();
        let names: Vec<&str> =
            resp["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(!names.contains(&"get_selection"), "no-bridge tools/list must NOT advertise get_selection");

        // WITH a bridge: advertised, camelCase inputSchema, no args.
        let resp = handle_request(&req("tools/list", json!({})), &ctx_with_dangling_bridge("unused")).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        let gs = tools
            .iter()
            .find(|t| t["name"] == "get_selection")
            .expect("get_selection advertised with a bridge");
        assert!(gs.get("inputSchema").is_some(), "camelCase inputSchema");
        assert!(gs.get("input_schema").is_none(), "no snake_case leak");
        assert!(
            gs["inputSchema"]["properties"].as_object().map(|o| o.is_empty()).unwrap_or(false),
            "get_selection takes no args"
        );
    }
}
