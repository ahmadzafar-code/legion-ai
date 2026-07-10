//! P0 SPIKE (throwaway, feature-gated) — proves the production-shaped, security-locked,
//! NON-interactive `claude` invocation the "Backend B" embedded-chat plan rests on.
//! See ../../IMPLEMENTATION-PLAN-cc-backend.md § P0.
//!
//! This is a GATE, not a feature. It exercises the assumptions that are undocumented /
//! upstream-"not planned" (claude-code#24594) and whose failure invalidates the whole
//! "keep one claude child alive across turns" architecture:
//!
//!   (C) tool round-trip:  a `mcp__legion-viewer__*` DATA tool call round-trips.
//!   (D) approval-in-pipe: `--allowedTools` alone suppresses the tool-approval prompt in a
//!                         piped, no-human context — a stall must FAIL (bounded read timeout),
//!                         never hang.
//!   (E) PERSISTENCE:      a SECOND user turn on the SAME long-lived stdin produces output
//!                         (vs. one-shot / requiring --resume). *Highest-probability kill.*
//!   (F) built-in lockdown: with Bash/Edit/Write/Read/... disabled, no built-in tool is used.
//!   (B) flag existence:   which of --input-format / --strict-mcp-config / --disallowedTools
//!                         / --mcp-config / --permission-mode actually exist on this CLI.
//!   (G) auth:             report how the child authenticates (env key vs. subscription login).
//!
//! It also PRINTS the observed stdin/stdout message shapes so the parser (P2b) can be written
//! against ground truth instead of guesses.
//!
//! Transport note: uses the in-viewer HTTP MCP server (`viewer_mcp::spawn`, the Backend-B
//! production transport) with a HEADLESS `UiBridge` — only DATA tools are allowed, and data
//! tools execute directly against DuckDB in `mcp_core`, never touching the bridge, so no UI
//! thread is required. Visual tools are intentionally NOT in the allow-list here.
//!
//! Usage: `cargo run --features viewer-mcp --bin cc_spike -- \
//!            [--duckdb <path>] [--model <id>] [--idle-secs N] [--prove-stderr-deadlock]`
//! Exit code 0 = gate PASSES (C && D && E && !F-violation); non-zero = FAILS.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use legion_prof_viewer::ai::agent::{AgentEvent, UiCommand};
use legion_prof_viewer::ai::bridge::{UiBridge, ViewportToken, MCP_CONSUMER_ID};
use serde_json::{json, Value};

/// Claude Code built-in tools to deny (lockdown). The spawned child inherits the user's full
/// harness + auth, and profile-derived strings are attacker-influenceable prompt-injection
/// vectors, so the coding agent's filesystem/web tools must be off. This list is what the spike
/// PROVES a viewer tool still round-trips under.
const DENY_BUILTINS: &[&str] = &[
    "Bash", "Edit", "Write", "Read", "WebFetch", "WebSearch", "NotebookEdit", "Glob", "Grep",
    "Task", "TodoWrite", "MultiEdit", "KillShell", "BashOutput",
];

/// Only these viewer DATA tools are allowed (headless-safe; no viewport/bridge needed).
const ALLOW_VIEWER: &[&str] = &["mcp__legion-viewer__run_query", "mcp__legion-viewer__overview"];

fn unix_now() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// One classified stdout event line.
#[derive(Debug)]
enum Ev {
    /// assistant turn; carries any tool_use names seen in its content blocks.
    Assistant(Vec<String>),
    /// a tool_result was returned (usually a `user`-role message).
    ToolResult,
    /// terminal message for a turn (`type":"result"`), with optional subtype.
    Result(String),
    /// `system` message (e.g. init). Subtype is visible in the raw shape preview.
    System,
    /// `control_request`/`control_cancel_request` — carries the request subtype. In
    /// stream-json mode `claude` delegates things like `oauth_token_refresh` to the
    /// PARENT via these; a tool-permission subtype means approval was NOT auto-granted.
    Control(String),
    /// any type we didn't special-case (prints so we learn the real shapes).
    Other(String),
    /// line was not valid JSON.
    Unparseable,
}

fn classify(line: &str) -> Ev {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Ev::Unparseable };
    match v.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let mut tools = Vec::new();
            if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                for b in content {
                    if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                        if let Some(n) = b.get("name").and_then(Value::as_str) {
                            tools.push(n.to_string());
                        }
                    }
                }
            }
            Ev::Assistant(tools)
        }
        Some("user") => {
            // tool_result echoes come back as a user-role message with tool_result blocks.
            let has_tr = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|c| c.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")))
                .unwrap_or(false);
            if has_tr { Ev::ToolResult } else { Ev::Other("user".into()) }
        }
        Some("result") => Ev::Result(
            v.get("subtype").and_then(Value::as_str).unwrap_or("").to_string(),
        ),
        Some("system") => Ev::System,
        Some("control_request") => Ev::Control(
            v.pointer("/request/subtype").and_then(Value::as_str).unwrap_or("").to_string(),
        ),
        Some("control_cancel_request") => Ev::Control("cancel".into()),
        Some(other) => Ev::Other(other.to_string()),
        None => Ev::Other("<no-type>".into()),
    }
}

/// Serialize one user turn in the stream-json INPUT shape (a HYPOTHESIS — if wrong, the child
/// will emit an error we print, which is itself the P0 answer).
fn user_turn(text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

struct Spike {
    idle: Duration,
    model: Option<String>,
    prove_stderr_deadlock: bool,
    db: PathBuf,
}

/// Result of pumping one turn's events until a `result` or a timeout.
struct TurnOutcome {
    saw_viewer_tool: bool,
    saw_builtin_tool: bool,
    saw_permission_event: bool,
    saw_result: bool,
    stalled: bool,
    exited: bool,
    /// The child hit a 401 / authentication_failed — a PRECONDITION problem (claude not
    /// logged in), NOT an architecture failure. Makes [C]/[D] inconclusive, not failed.
    auth_failed: bool,
    lines: usize,
}

impl Spike {
    /// Read events off `rx` until a `result` line, a stall (idle timeout -> FAIL), or the
    /// child closing stdout (disconnect). Prints a sample of each new shape it sees.
    fn pump_turn(&self, rx: &Receiver<String>, seen_shapes: &mut Vec<String>) -> TurnOutcome {
        let mut o = TurnOutcome {
            saw_viewer_tool: false, saw_builtin_tool: false, saw_permission_event: false,
            saw_result: false, stalled: false, exited: false, auth_failed: false, lines: 0,
        };
        let overall_deadline = Instant::now() + Duration::from_secs(240);
        loop {
            if Instant::now() >= overall_deadline {
                eprintln!("[spike] turn exceeded 240s overall budget");
                o.stalled = true;
                return o;
            }
            match rx.recv_timeout(self.idle) {
                Ok(line) => {
                    o.lines += 1;
                    // Auth precondition: a 401 means claude isn't logged in on this machine —
                    // NOT an architecture failure. Flag it so the verdict marks C/D inconclusive.
                    if line.contains("\"api_error_status\":401") || line.contains("authentication_failed") {
                        o.auth_failed = true;
                    }
                    let ev = classify(&line);
                    // Print a first sample of each distinct shape so P2b has ground truth.
                    let key = match &ev {
                        Ev::Assistant(_) => "assistant",
                        Ev::ToolResult => "tool_result",
                        Ev::Result(_) => "result",
                        Ev::System => "system",
                        Ev::Control(_) => "control",
                        Ev::Other(t) => t.as_str(),
                        Ev::Unparseable => "unparseable",
                    };
                    if !seen_shapes.iter().any(|s| s == key) {
                        seen_shapes.push(key.to_string());
                        let preview: String = line.chars().take(300).collect();
                        println!("  [shape:{key}] {preview}");
                    }
                    match ev {
                        Ev::Assistant(tools) => {
                            for t in tools {
                                if ALLOW_VIEWER.iter().any(|a| t == *a) || t.starts_with("mcp__legion-viewer__") {
                                    o.saw_viewer_tool = true;
                                    println!("  -> viewer tool_use: {t}");
                                } else if DENY_BUILTINS.iter().any(|d| t == *d) {
                                    o.saw_builtin_tool = true;
                                    println!("  !! LOCKDOWN LEAK: built-in tool_use: {t}");
                                }
                            }
                        }
                        Ev::ToolResult => {}
                        Ev::Result(sub) => {
                            o.saw_result = true;
                            println!("  -> result (subtype={sub})");
                            return o;
                        }
                        Ev::Control(sub) => {
                            // A TOOL-permission control_request means --allowedTools did NOT
                            // auto-approve (a real [D] stall). Non-tool controls (oauth_token_refresh,
                            // cancel) are benign harness plumbing, NOT an approval stall.
                            if sub.contains("permission") || sub.contains("tool") || sub.contains("can_use") {
                                o.saw_permission_event = true;
                                println!("  !! TOOL-permission control_request (subtype={sub}) -> approval NOT auto-granted");
                            } else {
                                println!("  .. benign control ({sub}) — parent must answer this in prod (see finding)");
                            }
                        }
                        _ => {}
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    eprintln!("[spike] STALL: no stdout line for {:?} (bounded read fails the gate)", self.idle);
                    o.stalled = true;
                    return o;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    o.exited = true;
                    return o;
                }
            }
        }
    }

    fn run(&self) -> bool {
        println!("== P0 spike: Backend-B production-shaped claude invocation ==\n");

        // --- (A) claude present + version ---
        let ver = Command::new("claude").arg("--version").output();
        match &ver {
            Ok(o) if o.status.success() => {
                println!("[A] claude: {}", String::from_utf8_lossy(&o.stdout).trim());
            }
            _ => {
                eprintln!("[A] FAIL: `claude` not found / not runnable. Install Claude Code first.");
                return false;
            }
        }

        // --- (B) flag existence (grep `claude --help`) ---
        let help = Command::new("claude").arg("--help").output().map(|o| {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }).unwrap_or_default();
        let flags = [
            "--input-format", "--output-format", "--mcp-config", "--allowedTools",
            "--disallowedTools", "--strict-mcp-config", "--permission-mode", "--verbose",
        ];
        print!("[B] flags present:");
        for f in flags { print!(" {f}={}", help.contains(f)); }
        println!();

        // --- (G) auth surface (report only) ---
        let has_env_key = std::env::var("ANTHROPIC_API_KEY").is_ok();
        println!("[G] auth: ANTHROPIC_API_KEY in env = {has_env_key} (else relies on `claude` subscription login; child inherits this env)");

        // --- start the in-viewer HTTP MCP server on a headless bridge ---
        if !self.db.exists() {
            eprintln!("[--] FAIL: fixture DuckDB not found: {}", self.db.display());
            return false;
        }
        let (event_tx, _event_rx) = mpsc::channel::<AgentEvent>();
        let (_cmd_tx, cmd_rx) = mpsc::channel::<UiCommand>();
        let bridge = UiBridge::new(event_tx, cmd_rx, ViewportToken::new(), MCP_CONSUMER_ID);
        let (port, token) = match legion_prof_viewer::ai::viewer_mcp::spawn(
            self.db.to_string_lossy().into_owned(), 0, bridge, None, None,
        ) {
            Ok(pt) => pt,
            Err(e) => { eprintln!("[--] FAIL: could not start in-viewer MCP server: {e}"); return false; }
        };
        println!("[--] in-viewer MCP (data tools) on http://127.0.0.1:{port}/mcp (bearer-token protected)");

        // --- write a private, 0600 mcp-config (http transport, server "legion-viewer").
        // Includes the Authorization header the hardened server now REQUIRES — so a
        // green spike run also proves claude forwards mcp-config headers end-to-end.
        let cfg_path = std::env::temp_dir().join(format!("cc_spike_mcp_{}.json", unix_now()));
        let cfg = json!({ "mcpServers": { "legion-viewer": {
            "type": "http", "url": format!("http://127.0.0.1:{port}/mcp"),
            "headers": { "Authorization": format!("Bearer {token}") }
        }}});
        if let Err(e) = std::fs::write(&cfg_path, cfg.to_string()) {
            eprintln!("[--] FAIL: write mcp-config: {e}"); return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&cfg_path, std::fs::Permissions::from_mode(0o600));
        }

        // --- spawn PERSISTENT claude (stream-json in AND out) with full lockdown ---
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--input-format").arg("stream-json")
            .arg("--output-format").arg("stream-json")
            .arg("--verbose")
            .arg("--mcp-config").arg(&cfg_path)
            .arg("--allowedTools").arg(ALLOW_VIEWER.join(","))
            .arg("--disallowedTools").arg(DENY_BUILTINS.join(","));
        if help.contains("--strict-mcp-config") {
            cmd.arg("--strict-mcp-config"); // ignore the user's global MCP servers
        }
        if let Some(m) = &self.model { cmd.arg("--model").arg(m); }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        println!("[--] spawning: claude -p --input-format stream-json --output-format stream-json \\\n     --verbose --mcp-config <cfg> --allowedTools \"{}\" --disallowedTools \"<builtins>\"{}\n",
            ALLOW_VIEWER.join(","), self.model.as_ref().map(|m| format!(" --model {m}")).unwrap_or_default());

        let mut child: Child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => { eprintln!("[--] FAIL: spawn claude: {e}"); let _ = std::fs::remove_file(&cfg_path); return false; }
        };
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Two reader threads. Draining BOTH pipes is mandatory: --verbose writes to stderr and an
        // undrained stderr pipe deadlocks the child at the ~64KB buffer.
        let (out_tx, out_rx) = mpsc::channel::<String>();
        let out_join = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if out_tx.send(line).is_err() { break; }
            }
        });
        // Optional: prove the stderr-non-drain deadlock (opt-in; bounded so it can't hang the spike).
        let err_join = if self.prove_stderr_deadlock {
            println!("[--] --prove-stderr-deadlock: NOT draining stderr (expect a turn stall) ...");
            None
        } else {
            Some(std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() { eprintln!("  [stderr] {line}"); }
                }
            }))
        };

        let mut seen_shapes: Vec<String> = Vec::new();

        // === TURN 1: force a viewer tool call (would prompt without --allowedTools) ===
        println!("\n[C/D/F] TURN 1 — force overview tool, non-interactive:");
        let t1 = user_turn(
            "You are running fully non-interactively; never ask the user anything. Call the `overview` \
             tool now, then reply with the single word DONE. Do not use any other tools.",
        );
        let write1 = writeln!(stdin, "{t1}").and_then(|_| stdin.flush());
        if let Err(e) = write1 { eprintln!("[C] FAIL: write turn 1 to stdin: {e}"); }
        let o1 = self.pump_turn(&out_rx, &mut seen_shapes);

        // === TURN 2: PERSISTENCE — second turn on the SAME stdin ===
        println!("\n[E] TURN 2 — persistence test (same live stdin):");
        let t2 = user_turn(
            "Now call `run_query` with exactly: SELECT count(*) AS n FROM items; then reply with the number and the word DONE2.",
        );
        let write2 = writeln!(stdin, "{t2}").and_then(|_| stdin.flush());
        let turn2_write_ok = write2.is_ok();
        if let Err(e) = &write2 { eprintln!("[E] note: writing turn 2 to stdin failed (broken pipe => not persistent): {e}"); }
        let o2 = if turn2_write_ok {
            self.pump_turn(&out_rx, &mut seen_shapes)
        } else {
            TurnOutcome { saw_viewer_tool: false, saw_builtin_tool: false, saw_permission_event: false, saw_result: false, stalled: false, exited: true, auth_failed: false, lines: 0 }
        };

        // --- shutdown: EOF -> kill -> wait -> join ---
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        let _ = out_join.join();
        if let Some(j) = err_join { let _ = j.join(); }
        let _ = std::fs::remove_file(&cfg_path);

        // === verdict ===
        // AUTH is a precondition: if the child 401'd, no model call happened, so the tool-dependent
        // checks C/D are INCONCLUSIVE (can't be proven here), NOT failed. Persistence/lockdown are
        // observed at the protocol level regardless of auth.
        let auth_failed = o1.auth_failed || o2.auth_failed;
        let check_e = o2.saw_result || o2.saw_viewer_tool || (turn2_write_ok && o2.lines > 0 && !o2.exited);
        let check_f = !o1.saw_builtin_tool && !o2.saw_builtin_tool; // lockdown held
        // C/D only meaningful under valid auth.
        let cd_state = if auth_failed {
            "INCONCLUSIVE (claude not authenticated on this machine)"
        } else if o1.saw_viewer_tool && o1.saw_result && !o1.saw_permission_event && !o1.stalled {
            "PASS"
        } else {
            "FAIL"
        };

        println!("\n================ P0 GATE ================");
        println!("  observed stdout shapes: {seen_shapes:?}");
        println!("  [E] PERSISTENT multi-turn stdin ..... {}   (turn2: lines={}, result={}, exited_after_t1={})",
            pf(check_e), o2.lines, o2.saw_result, o2.exited && o2.lines == 0);
        println!("  [F] built-in lockdown held .......... {}", pf(check_f));
        println!("  [C/D] tool round-trip + approval .... {cd_state}");
        println!("  ----------------------------------------");
        // Load-bearing (architecture-killing) risks = E + F. C/D need valid auth to decide.
        let load_bearing_ok = check_e && check_f;
        if auth_failed {
            println!("  GATE: INCONCLUSIVE — load-bearing risks (persistence + lockdown) {}.",
                if load_bearing_ok { "PASSED" } else { "did NOT pass" });
            println!("        AUTH PRECONDITION UNMET: `claude` returned 401 on this machine (even a plain");
            println!("        one-shot 401s). Run `claude login` OR set ANTHROPIC_API_KEY, then re-run to");
            println!("        decide [C]/[D]. FINDING: in stream-json mode claude delegates token refresh to");
            println!("        the PARENT via a `control_request` (oauth_token_refresh) — Backend B's pump must");
            println!("        be control-protocol-aware, not a dumb text pipe. Fold into plan P2a.");
        } else if load_bearing_ok && cd_state == "PASS" {
            println!("  GATE: PASS -> proceed to P1");
        } else {
            println!("  GATE: FAIL -> STOP + escalate (see plan §P0)");
            if !check_e {
                println!("        persistence is the highest-probability kill; `claude` may be one-shot per");
                println!("        stdin and Backend-B must re-spawn per turn (worse) or be reconsidered.");
            }
        }
        println!("========================================");
        // Exit non-zero only on a genuine FAIL of a load-bearing check; INCONCLUSIVE-on-auth is exit 2.
        if auth_failed { std::process::exit(2); }
        load_bearing_ok && cd_state == "PASS"
    }
}

fn pf(b: bool) -> &'static str { if b { "PASS" } else { "FAIL" } }

fn main() {
    let mut db: Option<String> = None;
    let mut model: Option<String> = None;
    let mut idle_secs: u64 = 90;
    let mut prove_stderr_deadlock = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--duckdb" => db = args.next(),
            "--model" => model = args.next(),
            "--idle-secs" => idle_secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(90),
            "--prove-stderr-deadlock" => prove_stderr_deadlock = true,
            other => eprintln!("[spike] ignoring unknown arg: {other}"),
        }
    }
    let db = db.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb")
    });
    let spike = Spike { idle: Duration::from_secs(idle_secs), model, prove_stderr_deadlock, db };
    std::process::exit(if spike.run() { 0 } else { 1 });
}
