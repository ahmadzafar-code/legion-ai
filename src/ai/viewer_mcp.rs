//! In-viewer HTTP MCP server — serves the data, source, wiki, and visual tools
//! from the running viewer so an external agent (e.g. Claude Code) can drive the
//! live process via
//! `claude mcp add --transport http legion-viewer http://127.0.0.1:PORT/mcp`.
//!
//! Transport surface (empirically verified against claude-code 2.1.150+):
//! a single `POST /mcp` per request with a `Content-Length` JSON body; the server
//! replies with ONE plain `application/json` JSON-RPC message and `Connection:
//! close`. The client advertises `Accept: …text/event-stream` and `Connection:
//! keep-alive` but accepts plain JSON on fresh connections; it probes `GET /mcp`
//! for a server SSE stream, which we 405 (it proceeds fine). No SSE, no chunked,
//! no session-id, no keep-alive required.
//!
//! Protocol logic is the shared [`crate::ai::mcp_core`] dispatch core — this file
//! is only the HTTP transport (plus the `/approve` route for the Claude Code
//! backend's tool-approval bridge). The [`spawn`]ed server carries a `UiBridge`,
//! so visual tools (screenshot/zoom/highlight/…) drive the live timeline. Every
//! query still routes through the hardened `execute_run_query_raw` (no new
//! DuckDB connection).
//!
//! SECURITY: binds 127.0.0.1 ONLY (never 0.0.0.0), rejects any request whose
//! `Origin` header is present and not a loopback origin (DNS-rebinding / CSRF
//! defense — a real CVE class in MCP servers, not theoretical), and requires
//! `Authorization: Bearer <token>` on every `POST /mcp` when the [`ServerCtx`]
//! carries a token ([`spawn`] always sets one).
//! Without the token requirement, ANY local process could drive the tools: the
//! Origin check passes when no Origin header is present. The token is random
//! per session (override with `LEGION_VIEWER_MCP_TOKEN` for a stable external
//! registration) and is printed in the `claude mcp add` line at startup.

use crate::ai::mcp_core::{handle_request, ServerCtx};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// The streamable-HTTP protocol version Claude Code negotiates (observed: the client
/// requested 2025-11-25 but accepted our 2025-03-26 and echoed it thereafter).
const HTTP_PROTOCOL_VERSION: &str = "2025-03-26";

/// Max request size we will buffer from a single connection (DoS guard).
const MAX_REQUEST_BYTES: usize = 1_048_576;

/// Well-known port the server tries first, so external `claude mcp add
/// …:8765/mcp` registrations keep working across restarts (an ephemeral port
/// is the fallback when it is taken).
pub const DEFAULT_MCP_PORT: u16 = 8765;

/// Pure parse → dispatch → serialize: raw HTTP request bytes → raw HTTP response
/// bytes. No sockets, no GUI — unit-testable. Enforces POST /mcp, the loopback
/// Origin check, and notification (202) vs result (200) framing.
pub fn handle_http_request(raw: &[u8], ctx: &ServerCtx) -> Vec<u8> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    let header_len = match req.parse(raw) {
        Ok(httparse::Status::Complete(n)) => n,
        _ => return http_response(400, "text/plain", b"bad request"),
    };

    // DNS-rebinding / CSRF defense: a present Origin MUST be loopback.
    if let Some(origin) = header_value(&req, "origin") {
        if !is_localhost_origin(origin) {
            return http_response(403, "text/plain", b"forbidden: non-local Origin");
        }
    }

    // Only POST /mcp serves JSON-RPC. GET /mcp (the client's SSE-stream probe) and
    // everything else get 405 — we have no streaming endpoint. Checked BEFORE auth
    // so the GET probe keeps the exact 405 behavior verified against claude-code.
    if req.method != Some("POST") || req.path != Some("/mcp") {
        return http_response(405, "text/plain", b"method not allowed");
    }

    // Server hardening: when the ctx carries a token, POST /mcp requires
    // `Authorization: Bearer <token>`. This closes the no-Origin hole (the Origin
    // check above only rejects PRESENT non-loopback origins; plain local processes
    // send no Origin at all).
    if let Some(expected) = ctx.auth_token.as_deref() {
        let presented = header_value(&req, "authorization")
            .and_then(|v| strip_bearer(v.trim()));
        if presented.is_none_or(|t| !token_eq(expected, t)) {
            return http_response(401, "text/plain", b"unauthorized: missing or bad bearer token");
        }
    }

    let content_length = header_value(&req, "content-length")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_end = header_len.saturating_add(content_length).min(raw.len());
    let body = &raw[header_len.min(raw.len())..body_end];

    let json_req: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            let err = json!({
                "jsonrpc": "2.0", "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            });
            return json_response(200, &err);
        }
    };

    match handle_request(&json_req, ctx) {
        Some(resp) => json_response(200, &resp),
        None => http_response(202, "text/plain", b""), // notification: no body
    }
}

/// Case-insensitive header lookup.
fn header_value<'a>(req: &httparse::Request<'a, '_>, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .and_then(|h| std::str::from_utf8(h.value).ok())
}

/// Strip a case-insensitive `Bearer ` auth-scheme prefix (RFC 7235: schemes are
/// case-insensitive), returning the trimmed credential.
fn strip_bearer(value: &str) -> Option<&str> {
    let (scheme, rest) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
}

/// Constant-time token comparison (length leak is fine — the token length is
/// public; the VALUE must not be timing-recoverable byte by byte).
fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Generate an unpredictable per-session token with NO extra dependency:
/// `RandomState` hash keys are seeded from OS entropy per process; mixing in
/// time + pid and chaining two hashers yields 128 bits, hex-encoded. NOT a
/// general-purpose CSPRNG — sufficient for a loopback-only bearer token whose
/// threat model is browsers (can't read it) and other-user local processes
/// (can't read this process's memory or environment). Same-user processes are
/// out of scope: they could read process memory regardless.
///
/// `LEGION_VIEWER_MCP_TOKEN` (non-empty) overrides — lets external users keep a
/// STABLE `claude mcp add … --header` registration across viewer restarts.
fn session_token() -> String {
    if let Ok(t) = std::env::var("LEGION_VIEWER_MCP_TOKEN") {
        let t = t.trim().to_owned();
        if !t.is_empty() {
            return t;
        }
    }
    use std::hash::{BuildHasher, Hasher};
    let mut h1 = std::collections::hash_map::RandomState::new().build_hasher();
    let mut h2 = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    h1.write_u128(now);
    h1.write_u32(std::process::id());
    h2.write_u64(h1.finish());
    h2.write_u128(now.rotate_left(64));
    format!("{:016x}{:016x}", h1.finish(), h2.finish())
}

/// True iff `origin` (e.g. `http://localhost:8743`, `http://[::1]:8743`) is a
/// loopback host. Used only to REJECT a present non-loopback Origin.
///
/// The host must be the literal name `localhost` OR parse as a loopback IP
/// (`127.0.0.0/8`, `::1`). It deliberately does NOT prefix-match `127.` —
/// `http://127.0.0.1.evil.com` is an attacker domain, not a loopback address, and
/// must be rejected.
///
/// We parse the authority strictly as `[ userinfo "@" ] host [ ":" port ]` per
/// RFC 3986: the authority ends at the first `/`, `?`, or `#`, and any userinfo
/// (everything up to the last `@`) is stripped BEFORE extracting the host. This
/// closes spoofs like `http://[::1]@evil.com` and `http://evil.com#@127.0.0.1`,
/// where a loopback-looking userinfo/fragment hides the real attacker host.
fn is_localhost_origin(origin: &str) -> bool {
    let after_scheme = origin.trim().split("://").nth(1).unwrap_or(origin.trim());
    // Authority terminates at the first path/query/fragment delimiter.
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    // Drop userinfo: take the part AFTER the last '@' so a spoofed
    // `loopback@real-host` resolves to the real host, not the userinfo.
    let hostport = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split(']').next().unwrap_or("") // IPv6: [addr]:port -> addr
    } else {
        hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport)
    };
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn http_response(code: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let mut out = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn json_response(code: u16, value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    http_response(code, "application/json", &body)
}

/// Read one full HTTP request (headers + `Content-Length` body) from `stream`.
/// One request per connection (the client opens a fresh connection per POST).
fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        // Once headers parse, check whether the declared body has fully arrived.
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        if let Ok(httparse::Status::Complete(hlen)) = req.parse(&buf) {
            let clen = req
                .headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case("content-length"))
                .and_then(|h| std::str::from_utf8(h.value).ok())
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= hlen.saturating_add(clen) {
                break;
            }
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 || buf.len() >= MAX_REQUEST_BYTES {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Ok(buf)
}

/// Is this raw request `POST /approve` (the PreToolUse hook bridge)? Cheap check
/// on the request line only — full parsing/auth happens in the handler.
fn is_approve_request(raw: &[u8]) -> bool {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    req.parse(raw).is_ok() && req.method == Some("POST") && req.path == Some("/approve")
}

/// The PreToolUse approval bridge. The hook's stdin JSON (tool_name +
/// tool_input + …) arrives as the POST body; the response body is the
/// `hookSpecificOutput` decision JSON the hook prints on stdout. Same Origin +
/// bearer checks as /mcp (the curl command in the child's settings file carries
/// the same session token). BLOCKS (up to [`APPROVAL_DEADLINE`]) while the panel
/// shows the dialog — callers must run this on a detached thread, never the
/// serial /mcp loop.
pub fn handle_approve_request(
    raw: &[u8],
    expected_token: &str,
    broker: &crate::ai::claude_code::ApprovalBroker,
) -> Vec<u8> {
    use crate::ai::claude_code::{hook_decision_json, APPROVAL_DEADLINE};

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    let header_len = match req.parse(raw) {
        Ok(httparse::Status::Complete(n)) => n,
        _ => return http_response(400, "text/plain", b"bad request"),
    };
    if let Some(origin) = header_value(&req, "origin") {
        if !is_localhost_origin(origin) {
            return http_response(403, "text/plain", b"forbidden: non-local Origin");
        }
    }
    let presented = header_value(&req, "authorization").and_then(|v| strip_bearer(v.trim()));
    if presented.is_none_or(|t| !token_eq(expected_token, t)) {
        return http_response(401, "text/plain", b"unauthorized: missing or bad bearer token");
    }

    let content_length = header_value(&req, "content-length")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_end = header_len.saturating_add(content_length).min(raw.len());
    let body = &raw[header_len.min(raw.len())..body_end];
    let event: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return http_response(400, "text/plain", format!("bad hook JSON: {e}").as_bytes()),
    };
    let tool_name = event.get("tool_name").and_then(Value::as_str).unwrap_or("?");
    let tool_input = event.get("tool_input").cloned().unwrap_or(Value::Null);

    let decision = broker.decide(tool_name, &tool_input, APPROVAL_DEADLINE);
    json_response(200, &hook_decision_json(decision))
}

/// Start the in-viewer HTTP MCP server on its OWN thread (never the egui main
/// thread). Binds 127.0.0.1 only. Returns the bound port. Logs the
/// `claude mcp add` line.
///
/// `bridge` is the [`UiBridge`](crate::ai::bridge::UiBridge) minted via
/// `Context::ui_bridge(MCP_CONSUMER_ID)`; attaching it to the `ServerCtx`
/// flips the server to advertise + route the 9 VISUAL tools, driving the live
/// timeline. The single accept loop processes ONE connection at a time, so MCP
/// `tools/call`s are SERIALIZED — there is no concurrent access to the bridge's
/// reply channel or the egui screenshot slot. The `ViewportToken` additionally
/// makes the embedded chat agent and this server mutually exclusive. CONTRACT:
/// single external driver at a time; do NOT make this loop multi-threaded without
/// per-connection viewport serialization (all MCP requests share MCP_CONSUMER_ID,
/// so concurrent same-id claims would be re-entrant, not mutually exclusive).
pub fn spawn(
    duckdb_path: String,
    port: u16,
    bridge: crate::ai::bridge::UiBridge,
    wiki_root: Option<String>,
    code_root: crate::ai::mcp_core::SharedCodeRoot,
) -> std::io::Result<(u16, String, std::sync::Arc<crate::ai::claude_code::ApprovalBroker>)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let bound = listener.local_addr()?.port();
    // Server hardening: every POST /mcp must present this bearer token. Random
    // per session; LEGION_VIEWER_MCP_TOKEN overrides for a stable registration.
    let token = session_token();
    // The approval broker behind POST /approve (the PreToolUse hook bridge).
    // Returned so core.rs can hand it to the chat panel (which renders the dialog).
    let broker = std::sync::Arc::new(crate::ai::claude_code::ApprovalBroker::new());
    eprintln!("[legion-viewer] in-viewer MCP (data + visual tools) on http://127.0.0.1:{bound}/mcp");
    eprintln!(
        "[legion-viewer] register: claude mcp add --transport http legion-viewer \
         http://127.0.0.1:{bound}/mcp --header \"Authorization: Bearer {token}\""
    );
    eprintln!(
        "[legion-viewer] (token is random per session; set LEGION_VIEWER_MCP_TOKEN for a stable one)"
    );
    let ctx_token = token.clone();
    let loop_broker = std::sync::Arc::clone(&broker);
    std::thread::Builder::new()
        .name("legion-viewer-mcp".to_owned())
        .spawn(move || {
            let ctx = ServerCtx::new(duckdb_path, None)
                .with_code_root_handle(code_root) // LIVE: the panel edits it any time
                .with_protocol(HTTP_PROTOCOL_VERSION)
                .with_wiki_root(wiki_root)
                .with_ui_bridge(bridge)
                .with_auth_token(Some(ctx_token.clone()));
            for mut stream in listener.incoming().flatten() {
                let Ok(buf) = read_request(&mut stream) else { continue };
                if is_approve_request(&buf) {
                    // An approval blocks for MINUTES on a human verdict — hand it to
                    // a detached thread so the serial /mcp loop (the viewport-
                    // serialization contract above) never stalls behind a dialog.
                    let tok = ctx_token.clone();
                    let brk = std::sync::Arc::clone(&loop_broker);
                    let _ = std::thread::Builder::new()
                        .name("legion-viewer-approve".to_owned())
                        .spawn(move || {
                            let resp = handle_approve_request(&buf, &tok, &brk);
                            let _ = stream.write_all(&resp);
                            let _ = stream.flush();
                        });
                } else {
                    let resp = handle_http_request(&buf, &ctx);
                    let _ = stream.write_all(&resp);
                    let _ = stream.flush();
                }
            }
        })?;
    Ok((bound, token, broker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Option<String> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb");
        p.exists().then(|| p.to_str().unwrap().to_owned())
    }

    /// Build a raw `POST /mcp` request with an optional Origin header.
    fn post(body: &str, origin: Option<&str>) -> Vec<u8> {
        let origin_line = origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default();
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Type: application/json\r\n{origin_line}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// Split a raw HTTP response into (status_code, body_str).
    fn split_response(raw: &[u8]) -> (u16, String) {
        let text = String::from_utf8_lossy(raw);
        let status = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap();
        let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        (status, body)
    }

    fn ctx() -> ServerCtx {
        ServerCtx::new("unused".to_owned(), None).with_protocol(HTTP_PROTOCOL_VERSION)
    }

    /// A ctx with a UiBridge attached (the LIVE-wired server). The UI-side
    /// channels dangle — fine for `tools/list`, which never drives the bridge.
    fn ctx_with_bridge() -> ServerCtx {
        use crate::ai::bridge::{UiBridge, ViewportToken, MCP_CONSUMER_ID};
        let (event_tx, _erx) = std::sync::mpsc::channel();
        let (_ctx_tx, cmd_rx) = std::sync::mpsc::channel();
        let bridge = UiBridge::new(event_tx, cmd_rx, ViewportToken::new(), MCP_CONSUMER_ID);
        ctx().with_ui_bridge(bridge)
    }

    /// Build a raw `POST /mcp` with an `Authorization` header (bearer-token tests).
    fn post_auth(body: &str, auth: &str) -> Vec<u8> {
        format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Type: application/json\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#;

    /// Server hardening: with a token on the ctx, POST /mcp without the header is
    /// rejected — the exact no-Origin-local-process hole this closes.
    #[test]
    fn test_auth_missing_token_is_401() {
        let ctx = ctx().with_auth_token(Some("sekrit-123".into()));
        let (status, _) = split_response(&handle_http_request(&post(INIT, None), &ctx));
        assert_eq!(status, 401);
    }

    #[test]
    fn test_auth_wrong_token_is_401() {
        let ctx = ctx().with_auth_token(Some("sekrit-123".into()));
        let req = post_auth(INIT, "Bearer wrong-token");
        let (status, _) = split_response(&handle_http_request(&req, &ctx));
        assert_eq!(status, 401);
        // Same length as the real token — still rejected (value, not length, matters).
        let req2 = post_auth(INIT, "Bearer sekrit-124");
        let (status2, _) = split_response(&handle_http_request(&req2, &ctx));
        assert_eq!(status2, 401);
    }

    #[test]
    fn test_auth_correct_token_is_200_and_scheme_case_insensitive() {
        let ctx = ctx().with_auth_token(Some("sekrit-123".into()));
        let (status, body) =
            split_response(&handle_http_request(&post_auth(INIT, "Bearer sekrit-123"), &ctx));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
        // RFC 7235: the auth scheme is case-insensitive.
        let (status2, _) =
            split_response(&handle_http_request(&post_auth(INIT, "bearer sekrit-123"), &ctx));
        assert_eq!(status2, 200);
    }

    /// Origin rejection must precede auth: a rebinding attacker who somehow learned
    /// the token still gets 403 on a non-loopback Origin.
    #[test]
    fn test_auth_origin_check_precedes_token() {
        let ctx = ctx().with_auth_token(Some("sekrit-123".into()));
        let body = INIT;
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Type: application/json\r\nOrigin: http://evil.com\r\nAuthorization: Bearer sekrit-123\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (status, _) = split_response(&handle_http_request(req.as_bytes(), &ctx));
        assert_eq!(status, 403);
    }

    /// The GET /mcp SSE probe keeps its verified 405 (method check precedes auth),
    /// so claude-code's connection handshake is unchanged by the token.
    #[test]
    fn test_auth_get_probe_still_405() {
        let ctx = ctx().with_auth_token(Some("sekrit-123".into()));
        let req = b"GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n";
        let (status, _) = split_response(&handle_http_request(req, &ctx));
        assert_eq!(status, 405);
    }

    /// Token generator sanity: 32 hex chars, unique across calls, env override wins.
    #[test]
    fn test_session_token_shape_and_override() {
        let a = session_token();
        let b = session_token();
        // Env override may be set in the environment running the tests; only assert
        // shape when it is not.
        if std::env::var("LEGION_VIEWER_MCP_TOKEN").is_err() {
            assert_eq!(a.len(), 32, "128-bit hex token");
            assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
            assert_ne!(a, b, "fresh entropy per call");
        }
        assert!(token_eq("abc", "abc") && !token_eq("abc", "abd") && !token_eq("abc", "ab"));
        assert_eq!(strip_bearer("Bearer  x  "), Some("x"));
        assert_eq!(strip_bearer("Basic x"), None);
        assert_eq!(strip_bearer("Bearerx"), None);
    }

    #[test]
    fn test_http_tools_list_visual_with_bridge() {
        // The LIVE-wired server (ServerCtx with a UiBridge, as ProfApp builds it)
        // advertises the 9 visual tools over HTTP, alongside the data tools.
        let req = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, None);
        let (status, body) = split_response(&handle_http_request(&req, &ctx_with_bridge()));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        let names: Vec<&str> =
            v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for t in [
            "screenshot", "zoom_to", "pan", "scroll_to", "set_view", "search", "reset_view",
            "highlight", "clear_highlights",
        ] {
            assert!(names.contains(&t), "live-wired tools/list must advertise visual tool {t}");
        }
        for t in ["run_query", "overview", "final_answer"] {
            assert!(names.contains(&t), "data tool {t} still present");
        }
        // Still never exposed.
        assert!(!names.contains(&"ask_user") && !names.contains(&"update_findings"));
        // camelCase inputSchema over HTTP, no snake_case leak (incl. visual tools).
        for t in v["result"]["tools"].as_array().unwrap() {
            assert!(t.get("inputSchema").is_some() && t.get("input_schema").is_none());
        }
    }

    #[test]
    fn test_http_initialize() {
        let req = post(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#, None);
        let (status, body) = split_response(&handle_http_request(&req, &ctx()));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(v["result"]["serverInfo"]["name"], "legion-prof");
        // The briefing channel is populated even with no roots (always-framing).
        let instr = v["result"]["instructions"].as_str().expect("instructions over HTTP");
        assert!(instr.contains("Legion Profiler Co-Pilot"), "framing must reach the client");
        assert!(!instr.contains("Application source root"), "no source clause without a code root");
    }

    /// Regression: a code root configured on the in-viewer server (as `spawn` now
    /// wires it from `chat_panel.code_path()`) must reach the external agent — the
    /// `instructions` source clause AND the read_code/list_files advertisement.
    #[test]
    fn test_http_code_root_briefs_source() {
        let ctx = ServerCtx::new("unused".to_owned(), Some("/app/src".to_owned()))
            .with_protocol(HTTP_PROTOCOL_VERSION);

        let init = post(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#, None);
        let (_s, body) = split_response(&handle_http_request(&init, &ctx));
        let v: Value = serde_json::from_str(&body).unwrap();
        let instr = v["result"]["instructions"].as_str().unwrap();
        assert!(instr.contains("Application source root: `/app/src`"), "source clause must reach the agent");

        let list = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, None);
        let (_s2, body2) = split_response(&handle_http_request(&list, &ctx));
        let v2: Value = serde_json::from_str(&body2).unwrap();
        let names: Vec<&str> =
            v2["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"read_code"), "read_code advertised with a code root");
        assert!(names.contains(&"list_files"), "list_files advertised with a code root");
    }

    #[test]
    fn test_http_notification_is_202_no_body() {
        let req = post(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, None);
        let (status, body) = split_response(&handle_http_request(&req, &ctx()));
        assert_eq!(status, 202);
        assert!(body.is_empty(), "notification reply must have no body");
    }

    #[test]
    fn test_http_tools_list_data_tools_only() {
        let req = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, None);
        let (status, body) = split_response(&handle_http_request(&req, &ctx()));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for want in ["run_query", "overview", "find_blockers", "final_answer"] {
            assert!(names.contains(&want), "missing data tool {want}");
        }
        // NO visual tools advertised.
        for forbidden in ["screenshot", "zoom_to", "set_view", "highlight", "ask_user"] {
            assert!(!names.contains(&forbidden), "must not advertise visual tool {forbidden}");
        }
        // camelCase inputSchema; no snake_case leak.
        for t in tools {
            assert!(t.get("inputSchema").is_some(), "tool {} missing inputSchema", t["name"]);
            assert!(t.get("input_schema").is_none(), "tool {} leaked input_schema", t["name"]);
        }
    }

    #[test]
    fn test_http_origin_mismatch_rejected() {
        // A present non-loopback Origin is forbidden (DNS-rebinding/CSRF defense).
        let req = post(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            Some("http://evil.example.com"),
        );
        let (status, _body) = split_response(&handle_http_request(&req, &ctx()));
        assert_eq!(status, 403, "non-local Origin must be rejected");

        // A loopback Origin is allowed.
        for ok_origin in ["http://localhost:8743", "http://127.0.0.1:8743", "http://[::1]:8743"] {
            let req = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, Some(ok_origin));
            let (status, _b) = split_response(&handle_http_request(&req, &ctx()));
            assert_eq!(status, 200, "loopback Origin {ok_origin} must be allowed");
        }

        // Look-alike attacker domains must NOT bypass the loopback check,
        // including userinfo/fragment spoofs where a loopback-looking prefix
        // hides the real attacker host (RFC 3986 authority parsing).
        for bad in [
            "http://127.0.0.1.evil.com",
            "http://localhost.evil.com",
            "https://evil.com",
            "http://10.0.0.1",
            "http://[::1]@evil.com",
            "http://127.0.0.1@evil.com",
            "http://localhost@evil.com",
            "http://evil.com#@127.0.0.1",
            "http://evil.com/@127.0.0.1",
            "http://evil.com?@127.0.0.1",
        ] {
            let req = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, Some(bad));
            let (status, _b) = split_response(&handle_http_request(&req, &ctx()));
            assert_eq!(status, 403, "look-alike Origin {bad} must be rejected");
        }

        // Legitimate userinfo with a real loopback host is still allowed.
        for ok in ["http://user@localhost:8743", "http://user@127.0.0.1:8765/mcp"] {
            let req = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, Some(ok));
            let (status, _b) = split_response(&handle_http_request(&req, &ctx()));
            assert_eq!(status, 200, "loopback Origin with userinfo {ok} must be allowed");
        }
    }

    #[test]
    fn test_http_get_is_405() {
        let req = b"GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:9\r\nAccept: text/event-stream\r\n\r\n";
        let (status, _body) = split_response(&handle_http_request(req, &ctx()));
        assert_eq!(status, 405, "no SSE stream endpoint; GET /mcp is 405");
    }

    #[test]
    fn test_http_run_query_benign_and_exfil() {
        let Some(path) = test_db() else {
            eprintln!("skipping: test DB absent");
            return;
        };
        let ctx = ServerCtx::new(path, None).with_protocol(HTTP_PROTOCOL_VERSION);

        // Benign query -> isError:false.
        let req = post(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_query","arguments":{"sql":"SELECT COUNT(*) AS n FROM items"}}}"#,
            None,
        );
        let (status, body) = split_response(&handle_http_request(&req, &ctx));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], false);

        // Exfil probe -> isError:true, no file contents (the anti-exfil hardening
        // of execute_run_query_raw holds over HTTP).
        let req = post(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_query","arguments":{"sql":"SELECT content FROM read_text('/etc/hosts')"}}}"#,
            None,
        );
        let (status, body) = split_response(&handle_http_request(&req, &ctx));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["isError"], true, "exfil must be a tool error");
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("localhost"), "must not leak /etc/hosts: {text}");
    }

    // ── /approve (PreToolUse hook bridge) ───────────────────────────────────

    /// Build a raw `POST /approve` request carrying a hook event body.
    fn post_approve(body: &str, auth: Option<&str>) -> Vec<u8> {
        let auth_line = auth.map(|a| format!("Authorization: {a}\r\n")).unwrap_or_default();
        format!(
            "POST /approve HTTP/1.1\r\nHost: 127.0.0.1:9\r\nContent-Type: application/json\r\n{auth_line}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    const HOOK_EVENT: &str = r#"{"session_id":"s","cwd":"/tmp","tool_name":"Bash","tool_input":{"command":"cargo check"}}"#;

    #[test]
    fn approve_requires_bearer_token() {
        let broker = crate::ai::claude_code::ApprovalBroker::new();
        let raw = post_approve(HOOK_EVENT, None);
        let (status, _) = split_response(&handle_approve_request(&raw, "tok-1", &broker));
        assert_eq!(status, 401);
        let raw2 = post_approve(HOOK_EVENT, Some("Bearer wrong"));
        let (status2, _) = split_response(&handle_approve_request(&raw2, "tok-1", &broker));
        assert_eq!(status2, 401);
        assert!(!broker.has_pending(), "unauthorized requests must never queue");
    }

    /// A pre-seeded session rule auto-allows without any pending dialog, and the
    /// response body is the documented hookSpecificOutput allow JSON.
    #[test]
    fn approve_auto_allows_on_session_rule() {
        use crate::ai::claude_code::{ApprovalBroker, ApprovalDecision};
        let broker = std::sync::Arc::new(ApprovalBroker::new());
        // Seed the BashPrefix("cargo") rule through the public path: resolve the
        // first request with always=true from a resolver thread.
        let b = std::sync::Arc::clone(&broker);
        let resolver = std::thread::spawn(move || {
            for _ in 0..200 {
                if let Some((id, _, _)) = b.front() {
                    b.resolve(id, ApprovalDecision::Allow, true);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let raw = post_approve(HOOK_EVENT, Some("Bearer tok-1"));
        let (status, body) = split_response(&handle_approve_request(&raw, "tok-1", &broker));
        resolver.join().unwrap();
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
        // Second matching call: no dialog needed — the rule answers instantly.
        let raw2 = post_approve(
            r#"{"tool_name":"Bash","tool_input":{"command":"cargo build -p x"}}"#,
            Some("Bearer tok-1"),
        );
        let (status2, body2) = split_response(&handle_approve_request(&raw2, "tok-1", &broker));
        assert_eq!(status2, 200);
        let v2: Value = serde_json::from_str(&body2).unwrap();
        assert_eq!(v2["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(!broker.has_pending());
    }

    /// A user Deny resolves the blocked handler with the documented deny JSON
    /// (reason included — the model continues its turn on it).
    #[test]
    fn approve_deny_resolves_with_reason() {
        use crate::ai::claude_code::{ApprovalBroker, ApprovalDecision};
        let broker = std::sync::Arc::new(ApprovalBroker::new());
        let b = std::sync::Arc::clone(&broker);
        let resolver = std::thread::spawn(move || {
            for _ in 0..200 {
                if let Some((id, tool, input)) = b.front() {
                    assert_eq!(tool, "Bash");
                    assert_eq!(input["command"], "rm -rf /");
                    b.resolve(id, ApprovalDecision::Deny, false);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("approval never queued");
        });
        let raw = post_approve(
            r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#,
            Some("Bearer tok-1"),
        );
        let (status, body) = split_response(&handle_approve_request(&raw, "tok-1", &broker));
        resolver.join().unwrap();
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        let reason = v["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
        assert!(reason.contains("denied"), "deny must carry an actionable reason");
    }

    #[test]
    fn approve_request_line_detection() {
        assert!(is_approve_request(&post_approve(HOOK_EVENT, None)));
        assert!(!is_approve_request(&post(INIT, None)));
    }
}
