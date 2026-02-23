//! Claude API client and agentic tool-use loop for Legion Prof analysis.
//!
//! The agent runs the agentic loop:
//! 1. POST messages to `api.anthropic.com/v1/messages`
//! 2. Execute tool calls returned by Claude (run_query, read_code, etc.)
//! 3. Send tool results back, repeat until `stop_reason == "end_turn"`
//!
//! Session state persists across turns so follow-up questions continue
//! the same conversation with full context.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::mpsc;

// ── Public response types ────────────────────────────────────────────────────

/// A timeline highlight returned by the agent.
///
/// `entry_slug` is the DuckDB entry slug (e.g. `"n0_cpu_c6"`).
/// Core.rs resolves it to an `EntryID` for overlay rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    pub entry_slug: String,
    pub start_ns: i64,
    pub stop_ns: i64,
    /// "critical" | "high" | "medium"
    pub severity: String,
    pub label: String,
}

/// Final output from one agent invocation (scan or follow-up).
#[derive(Debug)]
pub struct AgentResponse {
    /// Markdown analysis text for display in the chat panel.
    pub text: String,
    /// Structured highlights for timeline overlay.
    pub highlights: Vec<Highlight>,
    pub queries_executed: usize,
    pub turns_used: usize,
}

// ── Internal API types (serde for Claude wire format) ────────────────────────

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[allow(dead_code)]
    pub id: String,
    pub content: Vec<Value>,
    pub stop_reason: String,
    #[allow(dead_code)]
    pub usage: Value,
}

// ── Bidirectional channel types ──────────────────────────────────────────────

/// Messages sent FROM the agent thread TO the UI thread.
///
/// The agent emits these events as it progresses through the agentic loop,
/// allowing the chat panel to show progressive status updates.
#[derive(Debug)]
pub enum AgentEvent {
    /// Agent is about to execute a tool.
    ToolCall { name: String, purpose: String },
    /// Tool returned a result (summary = first ~100 chars or row count).
    ToolResult { name: String, summary: String },
    /// Agent needs a screenshot from the UI thread.
    ScreenshotRequest { request_id: u64 },
    /// Agent needs the UI to zoom to a time range and return a screenshot.
    ZoomRequest {
        request_id: u64,
        start_ns: i64,
        stop_ns: i64,
    },
    /// Agentic loop finished successfully.
    Complete(AgentResponse),
    /// Agentic loop failed with an error.
    Error(String),
}

/// Messages sent FROM the UI thread TO the agent thread.
///
/// Currently only carries screenshot data in response to a
/// `ScreenshotRequest` or `ZoomRequest`.
#[derive(Debug)]
pub enum UiCommand {
    /// Screenshot data in response to a ScreenshotRequest or ZoomRequest.
    ScreenshotData {
        request_id: u64,
        png_bytes: Vec<u8>,
        /// Visible time range + entry slugs for follow-up queries.
        metadata: String,
    },
}

// ── Agent session ────────────────────────────────────────────────────────────

/// Persistent agent session. Holds the full conversation history so follow-up
/// questions continue in the same context as the initial scan.
pub struct AgentSession {
    /// Accumulated messages for the Claude API (role / content pairs as JSON Values).
    messages: Vec<Value>,
    pub api_key: String,
    pub model: String,
    /// Path to the Legion DuckDB file.
    pub duckdb_path: String,
    /// Optional path to application source code root.
    pub code_path: String,
    /// Free-text application context from the user (e.g. goals, configuration).
    pub app_context: String,
    /// Maximum agent turns before forcing a summary response.
    pub max_turns: usize,

    // Computed once at session creation
    system_prompt: String,
    tools: Vec<Value>,

    // Bidirectional channel endpoints (agent ↔ UI thread)
    event_tx: mpsc::Sender<AgentEvent>,
    command_rx: mpsc::Receiver<UiCommand>,

    /// Monotonically increasing ID for screenshot/zoom requests.
    next_request_id: u64,
}

impl AgentSession {
    /// Create a new session. Tools are computed from feature availability.
    ///
    /// `event_tx` sends [`AgentEvent`]s to the UI thread (progressive status).
    /// `command_rx` receives [`UiCommand`]s from the UI thread (screenshot data).
    pub fn new(
        api_key: String,
        model: String,
        duckdb_path: String,
        code_path: String,
        app_context: String,
        event_tx: mpsc::Sender<AgentEvent>,
        command_rx: mpsc::Receiver<UiCommand>,
    ) -> Self {
        let has_duckdb = cfg!(feature = "duckdb") && !duckdb_path.is_empty();
        let has_code = !code_path.is_empty();
        let tools = super::tools::tool_definitions(has_duckdb, has_code);

        let system_prompt = build_system_prompt();

        Self {
            messages: Vec::new(),
            api_key,
            model,
            duckdb_path,
            code_path,
            app_context,
            max_turns: 25,
            system_prompt,
            tools,
            event_tx,
            command_rx,
            next_request_id: 0,
        }
    }

    /// Initial scan: "Find performance issues in this application."
    /// Builds the overview-enriched user message and runs the agentic loop.
    pub fn run_scan(&mut self) -> Result<AgentResponse, String> {
        let initial_msg = self.build_scan_message()?;
        self.run_agent_loop(initial_msg)
    }

    /// Follow-up question. The full conversation history is preserved so Claude
    /// has context from the initial scan.
    pub fn ask(&mut self, question: &str) -> Result<AgentResponse, String> {
        self.run_agent_loop(question.to_owned())
    }

    /// Clear conversation history (start fresh).
    pub fn reset(&mut self) {
        self.messages.clear();
    }

    /// Replace the bidirectional channel endpoints.
    ///
    /// Called when reusing a session across `trigger_diagnosis` calls,
    /// since each call creates fresh channels (the old ones are disconnected).
    pub fn update_channels(
        &mut self,
        event_tx: mpsc::Sender<AgentEvent>,
        command_rx: mpsc::Receiver<UiCommand>,
    ) {
        self.event_tx = event_tx;
        self.command_rx = command_rx;
    }

    /// Send an event to the UI thread. Silently ignores send failures
    /// (which happen if the UI dropped its receiver).
    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Request a screenshot from the UI thread and wait for the response.
    ///
    /// Emits a `ScreenshotRequest` or `ZoomRequest` event, then blocks on
    /// `command_rx` until the UI sends back `ScreenshotData` with matching
    /// `request_id`. Returns the base64-encoded PNG string (prefixed with
    /// `__IMAGE_BASE64__` so the caller can build an image content block).
    fn request_screenshot(
        &mut self,
        zoom_range: Option<(i64, i64)>,
    ) -> Result<String, String> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;

        // Emit the appropriate event to the UI thread
        match zoom_range {
            Some((start_ns, stop_ns)) => {
                self.emit(AgentEvent::ZoomRequest {
                    request_id,
                    start_ns,
                    stop_ns,
                });
            }
            None => {
                self.emit(AgentEvent::ScreenshotRequest { request_id });
            }
        }

        // Wait for the UI thread to respond (timeout after 10 seconds)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let timeout = deadline.saturating_duration_since(std::time::Instant::now());
            match self.command_rx.recv_timeout(timeout) {
                Ok(UiCommand::ScreenshotData {
                    request_id: rid,
                    png_bytes,
                    metadata,
                }) => {
                    if rid == request_id {
                        if png_bytes.is_empty() {
                            return Err(
                                "Screenshot capture returned empty data.".into(),
                            );
                        }
                        // Base64-encode the PNG
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD
                            .encode(&png_bytes);
                        // Include metadata alongside image data
                        return Ok(format!(
                            "__IMAGE_BASE64__{encoded}\n__METADATA__{metadata}"
                        ));
                    }
                    // Wrong request_id — stale response, keep waiting
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(
                        "Screenshot request timed out (10s). \
                         UI thread may not be responding."
                            .into(),
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("UI command channel disconnected.".into());
                }
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn build_scan_message(&self) -> Result<String, String> {
        let mut msg = "Find performance issues in this Legion application.\n\n".to_owned();

        // Include pre-computed overview when DuckDB is available
        #[cfg(feature = "duckdb")]
        if !self.duckdb_path.is_empty() {
            match super::tools::gather_overview(&self.duckdb_path) {
                Ok(overview) => {
                    msg.push_str("## Profiling Database Overview\n\n");
                    msg.push_str(&overview);
                    msg.push('\n');
                }
                Err(e) => {
                    msg.push_str(&format!(
                        "Note: Could not load database overview: {}\n\n",
                        e
                    ));
                }
            }
        }

        // Pre-load application source code (up to 40 KB) directly into the
        // initial message so the model can immediately relate profiling data to
        // application parameters (e.g. num_pieces, mapper policy) without
        // needing an extra round-trip through the read_code tool.
        if !self.code_path.is_empty() {
            match gather_application_code(&self.code_path) {
                Some(code_block) => {
                    msg.push_str("## Application Source Code\n\n");
                    msg.push_str(&code_block);
                    msg.push('\n');
                }
                None => {
                    // Code path set but nothing readable; fall back to tool hint
                    msg.push_str(&format!(
                        "Application source code is available via the `read_code` tool \
                         (root: `{}`).\n\n",
                        self.code_path
                    ));
                }
            }
        }

        // Include user-provided application context if present
        if !self.app_context.is_empty() {
            msg.push_str("\n\n## Application Context\n");
            msg.push_str(&self.app_context);
            msg.push('\n');
        }

        msg.push_str(
            "\nUse the available tools to investigate the profiling data. \
             Call `run_query` multiple times per response to batch independent queries.",
        );

        Ok(msg)
    }

    /// Core agentic loop. Appends `user_message` to history, then iterates:
    /// tool_use → execute tools → send results → repeat until end_turn.
    fn run_agent_loop(&mut self, user_message: String) -> Result<AgentResponse, String> {
        // Append the new user message
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": user_message
        }));

        let mut turns = 0usize;
        let mut queries_executed = 0usize;
        let mut force_summary_sent = false;

        loop {
            turns += 1;

            let response = self.call_claude()?;

            // Collect text content for display
            let response_text: String = response
                .content
                .iter()
                .filter_map(|b| b.get("text")?.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
                .join("\n");

            // Append assistant turn to history
            self.messages.push(serde_json::json!({
                "role": "assistant",
                "content": response.content.clone()
            }));

            if response.stop_reason == "end_turn" || turns >= self.max_turns {
                let highlights = parse_highlights_from_text(&response_text);
                return Ok(AgentResponse {
                    text: response_text,
                    highlights,
                    queries_executed,
                    turns_used: turns,
                });
            }

            // Collect all tool_use blocks from this response
            let tool_use_blocks: Vec<&Value> = response
                .content
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                .collect();

            if tool_use_blocks.is_empty() {
                // No tool calls but stop_reason != "end_turn" — treat as done
                let highlights = parse_highlights_from_text(&response_text);
                return Ok(AgentResponse {
                    text: response_text,
                    highlights,
                    queries_executed,
                    turns_used: turns,
                });
            }

            // Execute all tool calls and collect results
            let tool_results: Vec<Value> = tool_use_blocks
                .iter()
                .map(|block| {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let input = block
                        .get("input")
                        .cloned()
                        .unwrap_or(Value::Object(serde_json::Map::new()));

                    if name == "run_query" {
                        queries_executed += 1;
                    }

                    // Emit progressive status: tool is about to execute
                    self.emit(AgentEvent::ToolCall {
                        name: name.to_owned(),
                        purpose: input
                            .get("purpose")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned(),
                    });

                    let (content, is_error) = match self.execute_tool(name, &input) {
                        Ok(result) => (result, false),
                        Err(e) => (format!("Error: {}", e), true),
                    };

                    // Emit progressive status: tool returned
                    self.emit(AgentEvent::ToolResult {
                        name: name.to_owned(),
                        summary: if content.starts_with("__IMAGE_BASE64__") {
                            "screenshot captured".to_owned()
                        } else if content.len() > 100 {
                            format!("{}…", &content[..100])
                        } else {
                            content.clone()
                        },
                    });

                    // Image results → image + metadata content blocks for Claude
                    if !is_error && content.starts_with("__IMAGE_BASE64__") {
                        // Split off metadata if present
                        let (base64_data, metadata) =
                            if let Some(meta_pos) = content.find("\n__METADATA__") {
                                (
                                    &content["__IMAGE_BASE64__".len()..meta_pos],
                                    &content[meta_pos + "\n__METADATA__".len()..],
                                )
                            } else {
                                (&content["__IMAGE_BASE64__".len()..], "")
                            };

                        let mut content_blocks = vec![serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": base64_data
                            }
                        })];

                        // Add metadata as a text block if present
                        if !metadata.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": metadata
                            }));
                        }

                        return serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": content_blocks
                        });
                    }

                    serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": content,
                        "is_error": is_error
                    })
                })
                .collect();

            // Send tool results back — all in a single user message (API requirement)
            // tool_result blocks FIRST, no additional text blocks after
            self.messages.push(serde_json::json!({
                "role": "user",
                "content": tool_results
            }));

            // If near turn limit, nudge Claude to wrap up
            if turns >= self.max_turns - 2 && !force_summary_sent {
                force_summary_sent = true;
                self.messages.push(serde_json::json!({
                    "role": "user",
                    "content": "You've run enough queries. Please provide your final analysis \
                                now, including the highlights JSON block."
                }));
            }
        }
    }

    /// Dispatch a tool call to the appropriate tool function.
    ///
    /// Screenshot and zoom_to results are returned with a `__IMAGE_BASE64__`
    /// prefix so the caller can build an image content block for Claude's
    /// vision capability.
    fn execute_tool(&mut self, name: &str, input: &Value) -> Result<String, String> {
        match name {
            "run_query" => {
                #[cfg(feature = "duckdb")]
                {
                    let sql = input
                        .get("sql")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "Missing 'sql' parameter".to_owned())?;
                    super::tools::execute_run_query(&self.duckdb_path, sql)
                }
                #[cfg(not(feature = "duckdb"))]
                {
                    let _ = input;
                    Err("DuckDB support not compiled in. Rebuild with --features duckdb.".into())
                }
            }

            "read_code" => {
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "Missing 'path' parameter".to_owned())?;
                super::tools::execute_read_code(&self.code_path, path)
            }

            "screenshot" => self.request_screenshot(None),

            "zoom_to" => {
                let start_ns = input
                    .get("start_ns")
                    .and_then(|v| v.as_i64())
                    .ok_or("zoom_to requires start_ns (integer)")?;
                let stop_ns = input
                    .get("stop_ns")
                    .and_then(|v| v.as_i64())
                    .ok_or("zoom_to requires stop_ns (integer)")?;
                self.request_screenshot(Some((start_ns, stop_ns)))
            }

            _ => Err(format!("Unknown tool: {name}")),
        }
    }

    /// POST the current messages to the Claude API with exponential backoff on 429/529.
    ///
    /// When using an Opus model, extended thinking (`"type": "adaptive"`) is
    /// enabled and max_tokens is doubled to 16 000. Thinking is NOT supported
    /// on Sonnet — the guard is `model.contains("opus")` exactly as documented
    /// in the project's MEMORY.md.
    fn call_claude(&self) -> Result<ApiResponse, String> {
        let use_opus = self.model.contains("opus");

        // Opus benefits from a larger token budget for its thinking + response.
        let max_tokens: u32 = if use_opus { 16_000 } else { 8_000 };

        let mut req_body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "system": self.system_prompt,
            "messages": self.messages,
            "tools": self.tools,
        });

        // Extended thinking — Opus only. "enabled" forces thinking on every turn
        // with an explicit token budget. ("adaptive" would let the model decide
        // whether to think, but doesn't accept budget_tokens.)
        if use_opus {
            req_body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": 10_000
            });
        }

        let body_str = serde_json::to_string(&req_body)
            .map_err(|e| format!("Failed to serialize request: {e}"))?;

        let mut retry_delay_ms = 1_000u64;

        for attempt in 0..5u32 {
            let result = ureq::post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", &self.api_key)
                .set("anthropic-version", "2023-06-01")
                .set("Content-Type", "application/json")
                .timeout(std::time::Duration::from_secs(300))
                .send_string(&body_str);

            match result {
                Ok(resp) => {
                    let text = resp
                        .into_string()
                        .map_err(|e| format!("Failed to read response body: {e}"))?;
                    return serde_json::from_str::<ApiResponse>(&text).map_err(|e| {
                        let preview = &text[..text.len().min(500)];
                        format!("Failed to parse Claude response: {e}\nBody: {preview}")
                    });
                }

                Err(ureq::Error::Status(429 | 529, resp)) => {
                    let wait_ms = resp
                        .header("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|secs| secs * 1_000)
                        .unwrap_or(retry_delay_ms);

                    if attempt < 4 {
                        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                        retry_delay_ms = (retry_delay_ms * 2).min(60_000);
                        continue;
                    }
                    return Err(format!("Rate limited after {} retries", attempt + 1));
                }

                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    return Err(format!(
                        "API error {code}: {}",
                        &body[..body.len().min(500)]
                    ));
                }

                Err(e) => {
                    return Err(format!("Network error: {e}"));
                }
            }
        }

        Err("Max retries exceeded".into())
    }
}

// ── Source-code pre-loader ────────────────────────────────────────────────────

/// Read source files from `code_root` into a single markdown block (≤ 40 KB).
///
/// Files with recognised source extensions at the top level of the directory
/// are concatenated in alphabetical order. This mirrors the Python sidecar's
/// `{application_code}` injection that let Opus immediately relate profiling
/// data to application parameters (e.g. `num_pieces`, mapper policy) without
/// an extra `read_code` round-trip.
///
/// Returns `None` if the directory is unreadable or contains no source files.
fn gather_application_code(code_root: &str) -> Option<String> {
    const MAX_TOTAL: usize = 40_000;
    const SOURCE_EXTS: &[&str] = &["cc", "cpp", "c", "h", "hpp", "cu", "cuh", "py", "rs"];

    let root = std::path::Path::new(code_root);

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            let ext = p.extension()?.to_str()?;
            SOURCE_EXTS.contains(&ext).then_some(p)
        })
        .collect();

    if paths.is_empty() {
        return None;
    }

    paths.sort(); // deterministic ordering

    let mut out = String::with_capacity(MAX_TOTAL + 512);

    for path in &paths {
        if out.len() >= MAX_TOTAL {
            out.push_str(
                "\n*(additional files truncated — use `read_code` tool for more)*\n",
            );
            break;
        }
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let remaining = MAX_TOTAL.saturating_sub(out.len());
        let (chunk, truncated) = if contents.len() > remaining {
            (&contents[..remaining], true)
        } else {
            (contents.as_str(), false)
        };
        out.push_str(&format!("### {filename}\n```\n{chunk}"));
        if truncated {
            out.push_str("\n... (file truncated)");
        }
        out.push_str("\n```\n\n");
    }

    if out.is_empty() { None } else { Some(out) }
}

// ── Highlight extraction ─────────────────────────────────────────────────────

fn build_system_prompt() -> String {
    // Design notes:
    // - Causal analysis framework from best-practices HPC prompt engineering
    // - Bottleneck taxonomy forces explicit classification before recommendations
    // - Anti-hallucination: never fabricate schema/metrics; query first
    // - Tool guidance: aggregate before raw scan, interpret immediately
    // - Output structure is advisory (not rigid) to suit the agentic loop
    // - JSON highlights block at the end is NON-NEGOTIABLE — the app parses it
    r#"You are a world-class parallel runtime performance diagnostician specializing in Legion and task-based HPC systems.

You have access to a DuckDB profiling database (via run_query) and application source code (via read_code).

## Analytical Principles

**Causality over appearance.** Never stop at surface symptoms. For every observation (idle time, long tasks, serialization, excessive copies), trace backward through the dependency graph until you reach the *first controllable cause*:
  Symptom → Blocking event → Upstream cause → Controllable lever

**Classify before recommending.** Every root cause belongs to exactly one primary category:
  1. Insufficient parallelism (critical path / span too long)
  2. Load imbalance (straggler tasks, partition skew)
  3. Excessive data movement (copy volume, repeated materialization)
  4. Task granularity too fine (scheduling overhead dominates)
  5. Mapper / placement-induced serialization (physical instance conflicts)
  6. Hardware bottleneck (memory bandwidth or compute saturation)
  7. Runtime contention (meta-tasks, GC, or system overhead)

**Measurement before hypothesis.** Every claim must be backed by a DuckDB query result. Never fabricate schema, column names, timestamps, or task UIDs. If data is missing, say so. Prefer aggregates before raw event scans; filter aggressively to reduce noise.

**Respect theoretical limits.** Use span reasoning and Amdahl-style bounds. State when parallelism is inherently constrained. Do not imply unrealistic speedups.

**Prioritize the dominant bottleneck.** Focus on the single largest structural limiter first. Avoid lists of speculative micro-optimizations.

## Tool Usage

- Call `run_query` multiple times per response — batch independent queries together.
- Interpret query results immediately; link metrics to bottleneck class.
- Use `read_code` to connect measured behavior to specific application code (mapper policy, partition count, task structure).
- A pre-computed overview is provided in the user message — do not re-query basic schema unless you need something not covered.

## Visual Tools

You have access to the profiler's timeline visualization:

- `screenshot()` — Capture the current timeline view as a PNG image. Use this to:
  - See the overall timeline layout and identify visual patterns
  - Verify your findings by checking if gaps/overlaps match your query results
  - Get spatial context that raw numbers can't convey

- `zoom_to(start_ns, stop_ns)` — Zoom the timeline to a specific nanosecond range and capture a screenshot. Use this to:
  - Examine a specific time range you identified via queries
  - See fine-grained task scheduling within a bottleneck region
  - Verify that a gap or overlap exists where your data suggests it should

When to use visual tools:
- After initial queries identify a region of interest, zoom into it for visual confirmation
- When you need to understand the spatial layout of tasks across processors
- To verify that idle gaps or overlaps are genuine before reporting them
- Don't overuse — 1-3 screenshots per analysis is typically sufficient

Important: Screenshots show the timeline as the user sees it. The image includes processor rows (CPU cores, utility, IO) with colored task bars. Gaps between tasks indicate idle time. Overlapping or tightly packed regions indicate high utilization.

## Recommendations Format

For each issue found, provide:
- **Root cause** (causal chain in one sentence)
- **Evidence** (key metric with value)
- **Code linkage** (file/function if relevant)
- **Fix** (exact lever: config, mapper change, partition count, etc.)
- **Expected impact** (bound or range with assumptions)

Keep analysis rigorous and concise. Avoid verbose digressions. Every sentence should either present evidence, explain causality, or give a concrete recommendation.

## Timeline Highlights (Optional)

If your analysis identifies specific time-bounded issues worth marking on the timeline, include a JSON code block with highlights at the end of your response. This is optional — only include it when you have concrete issues with exact timestamps. The profiler UI parses this to create visual overlays:

```json
{"highlights": [{"entry_slug": "n0_cpu_c0", "start_ns": 670000000, "stop_ns": 759000000, "severity": "critical", "label": "89ms idle gap — mapper bottleneck"}]}
```

Rules for highlights:
- `entry_slug` must exactly match a slug from the `entries` table (e.g. `n0_cpu_c6`, `n0_gpu_0`)
- `severity`: `"critical"` (>100 ms impact), `"high"` (>50 ms), `"medium"` (>10 ms)
- `start_ns` / `stop_ns` are nanosecond timestamps from the profiling data
- Only annotate issues significant enough to warrant a visual marker on the timeline
- If no issues warrant highlights, output `{"highlights": []}`"#.to_owned()
}

/// Extract highlights from the agent's final text response.
///
/// Looks for the last ```json block that contains a "highlights" key.
fn parse_highlights_from_text(text: &str) -> Vec<Highlight> {
    // Find all ```json ... ``` blocks
    let mut last_json_block: Option<&str> = None;
    let mut search = text;

    while let Some(start) = search.find("```json") {
        let rest = &search[start + 7..];
        if let Some(end) = rest.find("```") {
            let block = rest[..end].trim();
            if block.contains("\"highlights\"") {
                last_json_block = Some(block);
            }
            search = &rest[end + 3..];
        } else {
            break;
        }
    }

    // Also check for a bare JSON object starting with {"highlights"
    if last_json_block.is_none() {
        if let Some(pos) = text.rfind("{\"highlights\"") {
            let candidate = &text[pos..];
            // Find the matching closing brace
            let mut depth = 0i32;
            let mut end = candidate.len();
            for (i, ch) in candidate.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let block = &candidate[..end];
            if block.contains("\"highlights\"") {
                last_json_block = Some(block);
            }
        }
    }

    let Some(json_block) = last_json_block else {
        return Vec::new();
    };

    // Parse the highlights array
    let Ok(parsed) = serde_json::from_str::<Value>(json_block) else {
        return Vec::new();
    };

    let Some(arr) = parsed.get("highlights").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|item| {
            Some(Highlight {
                entry_slug: item.get("entry_slug")?.as_str()?.to_owned(),
                start_ns: item.get("start_ns")?.as_i64()?,
                stop_ns: item.get("stop_ns")?.as_i64()?,
                severity: item
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("medium")
                    .to_owned(),
                label: item
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Performance issue")
                    .to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_highlights_json_block() {
        let text = r#"## Analysis

Found issues.

```json
{"highlights": [{"entry_slug": "n0_cpu_c0", "start_ns": 100, "stop_ns": 200, "severity": "critical", "label": "Test gap"}]}
```"#;
        let highlights = parse_highlights_from_text(text);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].entry_slug, "n0_cpu_c0");
        assert_eq!(highlights[0].start_ns, 100);
        assert_eq!(highlights[0].severity, "critical");
    }

    #[test]
    fn test_parse_highlights_empty() {
        let text = "No issues found. Good performance!";
        let highlights = parse_highlights_from_text(text);
        assert!(highlights.is_empty());
    }
}
