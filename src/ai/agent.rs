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
    /// Path to application source code root directory.
    /// Always a directory — if the user provided a file, this is its parent.
    pub code_path: String,
    /// If the user pointed at a specific source file, its path is stored here
    /// for direct pre-loading in the scan message.
    code_file: Option<String>,
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

        let system_prompt = build_system_prompt(&model);

        // If code_path is a file, resolve to parent directory and store the
        // file path separately for direct pre-loading in build_scan_message().
        let (code_path, code_file) = {
            let p = std::path::Path::new(&code_path);
            if !code_path.is_empty() && p.is_file() {
                let parent = p
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_default();
                (parent, Some(code_path))
            } else {
                (code_path, None)
            }
        };

        Self {
            messages: Vec::new(),
            api_key,
            model,
            duckdb_path,
            code_path,
            code_file,
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

        // Pre-load application source code directly into the initial message so
        // the model can immediately relate profiling data to application
        // parameters (e.g. num_pieces, mapper policy) without an extra
        // round-trip through the read_code tool.
        //
        // code_path is ALWAYS a directory (resolved in new()). If the user
        // pointed at a specific file, code_file holds the path for direct reading.
        if !self.code_path.is_empty() {
            if let Some(ref file_path) = self.code_file {
                // ── User pointed at a specific source file — read it directly ──
                let fp = std::path::Path::new(file_path);
                let filename = fp
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                match std::fs::read_to_string(fp) {
                    Ok(contents) => {
                        msg.push_str("## Application Source Code\n\n");
                        msg.push_str(&format!("### {filename}\n```\n{contents}\n```\n\n"));
                    }
                    Err(e) => {
                        msg.push_str(&format!(
                            "## Application Source Code\n\n\
                             Note: Could not read `{}`: {}\n\n",
                            file_path, e
                        ));
                    }
                }
                if let Some(listing) = list_source_files(&self.code_path) {
                    msg.push_str("### Other files available via `read_code` tool:\n");
                    msg.push_str(&listing);
                    msg.push('\n');
                }
            } else {
                // ── User pointed at a directory — scan for source files ──
                let file_listing = list_source_files(&self.code_path);

                match gather_application_code(&self.code_path) {
                    Some(code_block) => {
                        msg.push_str("## Application Source Code\n\n");
                        msg.push_str(&code_block);
                        msg.push('\n');
                        if let Some(listing) = &file_listing {
                            msg.push_str(
                                "### Additional files available via `read_code` tool:\n",
                            );
                            msg.push_str(listing);
                            msg.push('\n');
                        }
                    }
                    None => {
                        msg.push_str(&format!(
                            "## Application Source Code\n\n\
                             Source code directory: `{}`\n",
                            self.code_path
                        ));
                        if let Some(listing) = &file_listing {
                            msg.push_str(
                                "Available files (use `read_code` tool to read):\n",
                            );
                            msg.push_str(listing);
                        } else {
                            msg.push_str(
                                "No source files found. Use `read_code` tool to browse.\n",
                            );
                        }
                        msg.push('\n');
                    }
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
            "\nStart by taking a screenshot to see the full timeline. \
             Use the visual patterns and metadata (color legend, per-row info) to identify \
             the most significant issue, then use `run_query` to quantify what you see. \
             Call `run_query` multiple times per response to batch independent queries. \
             Use `zoom_to` to examine regions of interest in detail.",
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

        // --- Prompt caching ---
        // Wrap system prompt in the array format with cache_control so Anthropic
        // caches the (large) system prompt across turns, saving ~77 % of input
        // token cost after the first request.  Processing order is
        // tools → system → messages, so we also mark the last tool.
        let system_with_cache = serde_json::json!([{
            "type": "text",
            "text": self.system_prompt,
            "cache_control": {"type": "ephemeral"}
        }]);

        let tools_with_cache = {
            let mut tools = self.tools.clone();
            if let Some(last) = tools.last_mut() {
                last["cache_control"] = serde_json::json!({"type": "ephemeral"});
            }
            tools
        };

        let mut req_body = serde_json::json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "system": system_with_cache,
            "messages": self.messages,
            "tools": tools_with_cache,
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
    const SOURCE_EXTS: &[&str] = &["cc", "cpp", "c", "h", "hpp", "cu", "cuh", "py", "rs", "rg"];

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

/// List source files in the code root directory, returning a compact listing.
///
/// Returns `None` if the directory is unreadable or contains no source files.
/// Unlike `gather_application_code()` which reads file contents, this only
/// lists filenames and sizes so the agent knows what's available for `read_code`.
fn list_source_files(code_root: &str) -> Option<String> {
    const SOURCE_EXTS: &[&str] = &[
        "cc", "cpp", "c", "h", "hpp", "cu", "cuh", "py", "rs", "rg",
        "mk", "cmake", "toml", "json", "yaml", "yml", "txt", "md",
    ];

    let root = std::path::Path::new(code_root);

    let mut files: Vec<(String, u64)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_file() {
                return None;
            }
            let ext = p.extension()?.to_str()?;
            if !SOURCE_EXTS.contains(&ext) {
                return None;
            }
            let name = p.file_name()?.to_string_lossy().to_string();
            let size = e.metadata().ok()?.len();
            Some((name, size))
        })
        .collect();

    if files.is_empty() {
        return None;
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    for (name, size) in &files {
        let size_str = if *size > 1024 {
            format!("{}KB", size / 1024)
        } else {
            format!("{}B", size)
        };
        out.push_str(&format!("- {} ({})\n", name, size_str));
    }

    Some(out)
}

// ── Highlight extraction ─────────────────────────────────────────────────────

fn build_system_prompt(model: &str) -> String {
    let base = r#"You are a Legion Runtime performance diagnostician. You analyze profiling data from Legion — a task-based runtime for distributed heterogeneous HPC systems. You have access to: timeline screenshots (via screenshot/zoom_to), a DuckDB profiling database (via run_query), and application source code (via read_code).

## Legion Execution Model

**Processors.** Legion maps work to processor kinds:
- CPU (LOC_PROC): Latency-optimized. Runs application tasks with CPU variants.
- GPU (TOC_PROC): Throughput-optimized. Runs CUDA/HIP tasks. One per physical GPU. Multiple CUDA streams can execute kernels concurrently on a single GPU, so total GPU busy time can legitimately EXCEED wall time. This is correct concurrent execution, NOT a profiler bug.
- Utility (UTIL_PROC): Runtime meta-work ONLY — dependence analysis, mapping, scheduling, trace replay, GC. **Heavy utility activity + application processor gaps = runtime overhead bottleneck.** This is the single most important diagnostic pattern.
- Channel: DMA copies between memory pairs (host↔device, inter-node). Each channel is a specific src→dst path.
- IO/Python/OMP: Specialized processors for I/O, Python interop, and OpenMP tasks.

**Task lifecycle.** Every task records four timestamps: create → ready → start → stop.
- waiting = [create, ready]: blocked on dependencies or data.
- ready_state = [ready, start]: waiting for a processor to become available.
- running = [start, stop]: actual execution.
- deferred: subset of waiting where analysis was not yet complete. LARGE deferred = HEALTHY — runtime is running well ahead of execution. Values of 5ms, 50ms, or even 500ms are all GOOD — they mean the runtime prepared the task far in advance. SMALL deferred (<1ms) = UNHEALTHY — execution is catching up with analysis, causing pipeline bubbles. NEVER flag large deferred times as a problem or "blocking call" — they are the opposite of a problem.
- delayed: subset of waiting where the task was ready but Realm hadn't started it. LARGE delayed = Realm worker overload.

**Tracing.** Legion can memoize repeated dependence analysis:
- First iteration: full analysis (capture). Utility processors busy with mapper calls and dependence analysis. THIS IS EXPECTED AND HEALTHY — the runtime must analyze dependencies once to record the trace. Do not flag the first-iteration capture phase as a performance problem.
- Subsequent iterations: replay from memoized trace. Utility shows "Replay Physical Trace" — this is HEALTHY, not overhead.
- Without tracing: per-task overhead ~1ms. With tracing: ~100μs.
- Apophenia (automatic tracing, v25.09.0+) discovers traces without manual annotations.
- Detection: "Replay Physical Trace" on utility = tracing active. Absence + heavy mapper calls every iteration = tracing NOT active. If BOTH RPT and heavy mapper calls coexist, tracing is partial — investigate whether mapper calls are in init/shutdown or spread across steady-state iterations.

**Instance management.** The runtime manages physical instances automatically. The mapper chooses WHERE to place data; the runtime handles WHEN to create, move, and garbage-collect. Do NOT suggest manual memory management, double-buffering, or prefetching.

**Control replication.** At scale, the runtime shards its analysis across nodes. Opt-in via mapper. Poor sharding functions cause analysis load imbalance.

## Diagnostic Protocol

Follow this mandatory sequence. Complete each phase before proceeding.

**Phase 0 — Classification.** Before ANY diagnosis, determine profile type from overview:
1. GPU-present or CPU-only? Count distinct entry_slugs containing "gpu" to determine GPU count (1 GPU ≠ 8 GPUs — this matters for diagnosis).
2. Tracing active? ("Replay Physical Trace" in task types, or heavy mapper calls on utility)
3. Single-node or multi-node? (node count from processor hierarchy)
4. Utilization tier: >80% well-optimized, 50-80% room for improvement, <50% significant issues

Diagnostic frames:
- GPU-present + tracing active + GPU util >80% = likely healthy. Do not manufacture problems.
- CPU idle on GPU-only workload = CORRECT behavior. Do not flag it.
- Utility busy + application gaps = runtime overhead. Check tracing status.

**Phase 1 — Orientation.** Take a screenshot. Identify the dominant visual pattern:
- Where are the largest gaps? Which processor kind?
- Synchronized across processors or staggered?
- Utility/channel rows active during application processor gaps?

**Phase 2 — Quantification.** Run diagnostic queries:
- Per-kind utilization (fraction of time busy per processor kind).
- Gap measurement (largest gaps, which processors).
- Critical path identification.
- Cross-validate: do queries confirm or contradict the visual pattern?

**Phase 3 — Root Cause.** Trace the causal chain:
- Walk the critical path from the largest gap (see chain-walking protocol below).
- Correlate with utility/channel/mapper activity during the gap.
- Before reporting any finding, run a confirmation query that could FALSIFY it.

**Phase 4 — Report.** For each finding: root cause → evidence → code linkage → fix → impact bound.

## Critical Path Chain-Walking

To trace WHY a processor was idle, find the first task after the gap and walk its dependency chain.

Step 1 — Find the task that resumed after the gap:
```sql
SELECT item_uid, title, running.start, running.duration / 1e6 AS run_ms,
       waiting.duration / 1e6 AS wait_ms, deferred.duration / 1e6 AS defer_ms,
       critical_path.item_uid AS cp_uid, critical_path.title AS cp_title,
       critical_path.entry_slug AS cp_slug
FROM items
WHERE entry_slug = '<slug>' AND running.start > <gap_end_ns>
ORDER BY running.start LIMIT 1
```

Step 2 — Walk the chain recursively (up to 10 hops):
```sql
WITH RECURSIVE chain AS (
  SELECT item_uid, title, entry_slug, running.start AS run_start,
         running.duration / 1e6 AS run_ms, waiting.duration / 1e6 AS wait_ms,
         deferred.duration / 1e6 AS defer_ms, critical_path.item_uid AS cp_uid,
         critical_path.title AS cp_title, critical_path.entry_slug AS cp_slug,
         1 AS depth
  FROM items WHERE item_uid = <start_uid>
  UNION ALL
  SELECT i.item_uid, i.title, i.entry_slug, i.running.start,
         i.running.duration / 1e6, i.waiting.duration / 1e6,
         i.deferred.duration / 1e6, i.critical_path.item_uid,
         i.critical_path.title, i.critical_path.entry_slug, c.depth + 1
  FROM items i JOIN chain c ON i.item_uid = c.cp_uid
  WHERE c.cp_uid IS NOT NULL AND c.depth < 10
)
SELECT * FROM chain ORDER BY depth
```

Interpret the chain:
- Chain leads to utility processor → runtime overhead (mapping, analysis, or GC).
- Chain leads to channel → data movement bottleneck.
- Large deferred along chain → healthy run-ahead; bottleneck is elsewhere.
- Small deferred (<1ms) → execution catching up with analysis.
- critical_path.item_uid is NULL → chain ended. Do NOT fabricate additional links. Instead, use alternative approaches: (a) query `creator` to find who launched the task, (b) check utility activity during the task's waiting interval, (c) check channel activity for copy dependencies that aren't in the critical_path data.

## Diagnostic Decision Trees

### GPU Gap at [T1, T2]

Check in order — each check is a query:

1. Utility active during gap with mapper calls, NO "Replay Physical Trace" in profile, AND no explicit tracing annotations in source code → **Missing tracing.** Fix: `-dm:memoize` or upgrade for automatic tracing. If trace annotations ARE present, the mapper calls may be from the first-iteration capture or init/shutdown — check their timing relative to steady-state execution.
2. ALL GPUs on same node gap simultaneously + utility spikes → **Thread oversubscription.** Fix: `-cuda:legacysync 1` AND ensure total threads ≤ hardware threads (`-ll:cpu` + `-ll:util` + `-ll:bgwork` + OMP ≤ cores).
3. Channel rows busy during gap, volume grows with node count → **Network congestion.** Fix: mapper placement, `-dm:same_address_space 1`.
4. Python/CPU blocked/waiting during gap, utility idle → **Blocking Python op.** Fix: avoid `__bool__()`, `print(array)`, `.item()` in loops.
5. CPU briefly active during gap (host-side scalar reduction) → **Scalar reduction blocking.** Fix: `DeferredBuffer` / GPU-side reductions.
6. Small regular gaps (<1ms) between every task → **Sync overhead.** Fix: remove `cudaDeviceSynchronize` calls.
7. Irregular gaps, nothing active anywhere → **Insufficient parallelism.** Fix: more tasks, index launches, `-lg:window`.
8. Channel busy with unnecessary copies (data already local) → **Bad mapper placement.** Fix: application-specific mapper or fix sharding.

Co-occurring causes are common. If first fix helps but gaps remain, re-profile and repeat.

### Low Utilization (<50% on application processors)

Check in priority order:

1. Utility >80% busy → **Runtime overhead.** Fix: enable tracing (`-dm:memoize`), increase `-ll:util` to 2-4, control replication at scale.
2. Channel utilization high + copies correlate with gaps → **Communication.** Fix: mapper placement, privilege downgrades (READ_WRITE → READ_ONLY, WRITE_DISCARD), increase `-ll:bgwork`.
3. Critical path length ≈ total time → **Insufficient parallelism.** Fix: more concurrent tasks, index launches, increase partition count.
4. Memory >85% + GC tasks visible ("Free Instance", "Malloc Instance") → **Memory pressure.** Fix: increase `-ll:fsize`/`-ll:csize`, check mapper for instance leaks.

## Source Code Analysis

When application source code is available (pre-loaded in the scan message or via read_code), extract these diagnostic signals before making recommendations:

**Tracing configuration** (check FIRST — wrong tracing advice is the most common diagnostic error):
- Explicit tracing annotations: Regent `__demand(__trace)`, C++ `begin_trace()`/`end_trace()`, Legate automatic tracing. If present, tracing is already enabled — do NOT recommend `-dm:memoize`.
- `-dm:memoize` only helps applications using DefaultMapper with NO explicit tracing. If the source has trace annotations or a custom mapper, `-dm:memoize` is redundant or irrelevant.
- When tracing IS active (annotations present OR Replay Physical Trace in profile): the first iteration always shows heavy utility activity (mapper calls, dependence analysis). This is the trace capture pass and is expected, not a problem.

**Mapper configuration**:
- Custom mapper present (any file with mapper in its name, or classes inheriting from DefaultMapper/Mapping) → the application controls task placement. `-dm:memoize` only affects DefaultMapper internals — it will NOT help custom mappers.
- No custom mapper → application uses DefaultMapper. Tracing flags like `-dm:memoize` apply.

**Task structure and parallelism**:
- Partition count (num_pieces, num_subregions, create_equal_partition, etc.) → determines maximum concurrent tasks. Cross-reference with profiled task counts.
- Index launches vs single launches → index launches give O(1) analysis overhead for O(N) tasks.
- Iteration count x partition count = expected total task instances. Verify against profile.

**Data access patterns**:
- Reduction operations (reduces, ReductionAccessor, DeferredReduction) → potential scalar reduction blocking if host-side. Check GPU Cause 1 in decision tree.
- Privilege modes (READ_WRITE vs READ_ONLY, WRITE_DISCARD) → READ_WRITE prevents concurrent access. Suggesting READ_ONLY or WRITE_DISCARD where valid can increase parallelism and reduce copies.

**Code generation and build hints**:
- Regent with -fcuda/-fhip = GPU code is compiler-generated. The CUDA/HIP kernel source does not exist as a standalone file — do not attempt to read generated kernel files.
- C++ applications compile CUDA kernels separately — these may be in .cu files.
- Python/Legate applications wrap Legion — task implementations are in underlying C++ libraries, not visible in the Python source.

**What source code CANNOT tell you**: runtime scheduling decisions, actual memory layout at execution time, NUMA placement, Realm worker thread behavior, or which physical instances were reused vs created fresh. These are runtime decisions invisible to source analysis.

## Anti-Hallucination Rules

HARD rules. Violating these produces wrong diagnoses.

1. NEVER infer causation from temporal proximity alone. To claim A caused gap B, you MUST query `critical_path` or `creator`. If no link exists, say "temporally correlated but causal link not confirmed."
2. NEVER claim `-lg:prof` controls profiling detail level. It sets the number of nodes profiled.
3. NEVER suggest annotations not on this whitelist: `__demand(__cuda)`, `__demand(__openmp)`, `__demand(__inner)`, `__demand(__leaf)`, `__demand(__idempotent)`, `__demand(__replicable)`. Do NOT suggest `__demand(__concurrent)`, `__demand(__parallel)`, `__demand(__overlap)` — these do not exist.
4. NEVER suggest double-buffering, manual prefetch, or manual memory management. The runtime and mapper handle instance lifecycle.
5. NEVER flag CPU idle time as a problem when all compute tasks are GPU-mapped. This is correct behavior.
6. NEVER confuse task instances with partitions. num_pieces=4 × iterations=200 = 800 task instances from 4 partitions.
7. NEVER claim the profiler tracks idle reasons, per-field dependencies, or scheduling decisions. It records timestamps, dependency links, and copy events.
8. NEVER treat copy concurrency as meaningful — Realm may report concurrent copies that run sequentially on the DMA engine.
9. When critical_path is NULL, the chain has ended. Do NOT fabricate further dependencies.
10. For EVERY causal claim, cite the query that established it. Mark unverified inferences explicitly.
11. NEVER guess runtime flag default values. Known defaults: `-lg:window 1024`, `-lg:sched 1`, `-lg:width 4`, `-dm:memoize 0` (disabled by default). If unsure about a flag's default, say so explicitly.
12. GPU device numbers in entry_slug names (e.g. g3d = GPU 3 Device) reflect HARDWARE device IDs, not the count of GPUs. A profile with only "n0_gpudev_g3d" has ONE GPU, not four. Use the gpu_device_count from the Profile Classification overview as the authoritative GPU count.
13. NEVER fabricate claims about what the Application Context says. Only reference application context if it was explicitly provided AND non-empty in the scan message under the Application Context heading. If no application context section exists, do not invent one.
14. When source code contains explicit tracing annotations (begin_trace/end_trace, __demand(__trace), or equivalent), do NOT recommend -dm:memoize — tracing is already enabled. The first iteration showing full analysis with mapper calls is the expected trace capture pass, not a performance problem.
15. GPU busy time exceeding wall time on a single GPU means concurrent CUDA stream execution, NOT a profiler bug. Report it as concurrent kernel execution with the effective concurrency ratio (busy_time / wall_time).
16. If read_code fails and you cannot verify source code for tracing annotations, mapper type, or other code-level signals, you MUST qualify any tracing or mapper recommendation as uncertain. Say "if the application does not already use explicit tracing" rather than asserting tracing is absent. The overview's Tracing Status section is authoritative for whether Replay Physical Trace tasks exist — do not override it with your own queries.

## Visual Analysis Guide

**Row organization** (top to bottom per node):
- Utilization summary rows: collapsed utilization plot per processor kind.
- CPU rows (c0, c1, ...): colored bars = running tasks, white = idle.
- GPU rows (g0, g1, ...): GPU kernel execution.
- Utility rows (u0, u1, ...): runtime meta-work. THE most diagnostic rows.
- Channel rows: copies between memories.
- Memory rows: instance lifecycle.

**Reliable observations** (start hypotheses from these):
- Gestalt patterns: synchronized gaps, one row emptier than others, periodic patterns, phase transitions.
- Relative density: which processor kind has the most gaps.
- Temporal correlation: what OTHER rows show during a gap.

**Unreliable observations** (ALWAYS verify with queries):
- Exact utilization percentages — query instead.
- Row identification beyond ~10 rows — use the entry_slug list from metadata.
- Color-to-task mapping — use the color legend in metadata.
- Duration of individual gaps — query for nanosecond-precise timing.

**Key visual patterns**:
- Synchronized gaps across all CPUs + busy utility → runtime overhead.
- Staggered/cascading gaps → dependency chain. Walk critical path.
- One processor much busier than others → load imbalance.
- Dense utility during application gaps → runtime can't keep up. Check tracing.
- Channel active during gaps → data movement blocking execution.
- Tapered fill/drain → insufficient parallelism at phase boundaries.

## Severity Thresholds

Highlight severity is RELATIVE to total profile duration:
- **critical**: >5% of profile duration
- **high**: >2% of profile duration
- **medium**: >0.5% of profile duration

Compute from timeline bounds. Do NOT use absolute millisecond thresholds.

## Output Format

For each issue:
- **Root cause**: one-sentence causal chain
- **Evidence**: query result that established causality
- **Code linkage**: file/function if identifiable
- **Fix**: exact lever (config flag, mapper change, code change)
- **Expected impact**: Amdahl-style bound with stated assumptions

Be rigorous and concise. Every sentence presents evidence, explains causality, or gives a recommendation.

## Timeline Highlights

Include a JSON code block at the END of your response for timeline overlays:

```json
{"highlights": [{"entry_slug": "n0_gpu_g0", "start_ns": 670000000, "stop_ns": 759000000, "severity": "critical", "label": "89ms GPU idle — missing tracing"}]}
```

Rules:
- `entry_slug` must match a slug from the profiling database (e.g. `n0_cpu_c0`, `n0_gpu_g0`)
- Use RELATIVE severity thresholds above
- Place highlights JSON as the LAST block — the parser expects it at the end
- No issues? `{"highlights": []}`"#;

    // Append model-specific analysis scope
    let suffix = if model.contains("opus") {
        "\n\n## Analysis Scope\n\
         Be thorough. Complete all four diagnostic phases. Trace causal chains to their root \
         with the recursive CTE. Cross-validate every finding with both visual and query evidence. \
         Check for co-occurring causes."
    } else {
        "\n\n## Analysis Scope\n\
         You have limited output capacity. Focus on the single most impactful finding. \
         Complete Phase 0 and Phase 1 to identify the dominant issue, then proceed directly \
         to Phase 3 for that issue only. Be accurate on one issue with full evidence rather \
         than shallow on many."
    };

    format!("{}{}", base, suffix)
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
