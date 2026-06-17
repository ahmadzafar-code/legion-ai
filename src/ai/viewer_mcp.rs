//! In-viewer HTTP MCP server (V1.1) — DATA tools only, served from the running
//! viewer so Claude Code can connect to the live process via
//! `claude mcp add --transport http legion-viewer http://127.0.0.1:PORT/mcp`.
//!
//! Transport surface (empirically verified against claude-code 2.1.150, Step 0):
//! a single `POST /mcp` per request with a `Content-Length` JSON body; the server
//! replies with ONE plain `application/json` JSON-RPC message and `Connection:
//! close`. The client advertises `Accept: …text/event-stream` and `Connection:
//! keep-alive` but accepts plain JSON on fresh connections; it probes `GET /mcp`
//! for a server SSE stream, which we 405 (it proceeds fine). No SSE, no chunked,
//! no session-id, no keep-alive required.
//!
//! Protocol logic is the shared [`crate::ai::mcp_core`] dispatch core — this file
//! is only the HTTP transport. Data tools only; NO visual tools, NO `UiBridge`,
//! NO screenshots (that is V1.2). Every query still routes through the hardened
//! `execute_run_query_raw` (no new DuckDB connection).
//!
//! SECURITY: binds 127.0.0.1 ONLY (never 0.0.0.0), and rejects any request whose
//! `Origin` header is present and not a loopback origin (DNS-rebinding / CSRF
//! defense — a real rmcp CVE class, not theoretical).

use crate::ai::mcp_core::{handle_request, ServerCtx};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// The streamable-HTTP protocol version Claude Code negotiates (Step 0: the client
/// requested 2025-11-25 but accepted our 2025-03-26 and echoed it thereafter).
const HTTP_PROTOCOL_VERSION: &str = "2025-03-26";

/// Max request size we will buffer from a single connection (DoS guard).
const MAX_REQUEST_BYTES: usize = 1_048_576;

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
    // everything else get 405 — we have no streaming endpoint.
    if req.method != Some("POST") || req.path != Some("/mcp") {
        return http_response(405, "text/plain", b"method not allowed");
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

/// Read one full HTTP request (headers + `Content-Length` body) from `stream`,
/// dispatch it, and write the response. One request per connection (the client
/// opens a fresh connection per POST).
fn serve_one(stream: &mut TcpStream, ctx: &ServerCtx) -> std::io::Result<()> {
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
    let resp = handle_http_request(&buf, ctx);
    stream.write_all(&resp)?;
    stream.flush()
}

/// Start the in-viewer HTTP MCP server on its OWN thread (never the egui main
/// thread). Binds 127.0.0.1 only. Returns the bound port. Logs the
/// `claude mcp add` line.
///
/// `bridge` is the [`UiBridge`](crate::ai::bridge::UiBridge) minted via
/// `Context::ui_bridge(MCP_CONSUMER_ID)`; attaching it to the `ServerCtx` (V1.3)
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
) -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let bound = listener.local_addr()?.port();
    eprintln!("[legion-viewer] in-viewer MCP (data + visual tools) on http://127.0.0.1:{bound}/mcp");
    eprintln!(
        "[legion-viewer] register: claude mcp add --transport http legion-viewer http://127.0.0.1:{bound}/mcp"
    );
    std::thread::Builder::new()
        .name("legion-viewer-mcp".to_owned())
        .spawn(move || {
            let ctx = ServerCtx::new(duckdb_path, None)
                .with_protocol(HTTP_PROTOCOL_VERSION)
                .with_ui_bridge(bridge);
            for mut stream in listener.incoming().flatten() {
                let _ = serve_one(&mut stream, &ctx);
            }
        })?;
    Ok(bound)
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

    #[test]
    fn test_http_initialize() {
        let req = post(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#, None);
        let (status, body) = split_response(&handle_http_request(&req, &ctx()));
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(v["result"]["serverInfo"]["name"], "legion-prof");
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

        // Exfil probe -> isError:true, no file contents (the P0 gate holds over HTTP).
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
}
