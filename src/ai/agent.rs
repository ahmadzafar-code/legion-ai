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
    /// Tool returned a result (summary = first ~100 chars, full_content = complete result).
    ToolResult { name: String, summary: String, full_content: String },
    /// Agent needs a screenshot from the UI thread.
    ScreenshotRequest { request_id: u64 },
    /// Agent needs the UI to zoom to a time range and return a screenshot.
    ZoomRequest {
        request_id: u64,
        start_ns: i64,
        stop_ns: i64,
    },
    /// Agent wants to pan left/right and get a screenshot.
    PanRequest {
        request_id: u64,
        direction: String,
        percent: f64,
    },
    /// Agent wants to scroll vertically to a processor and get a screenshot.
    ScrollToRequest {
        request_id: u64,
        entry_slug: String,
    },
    /// Agent wants to zoom + optionally scroll in one call.
    SetViewRequest {
        request_id: u64,
        start_ns: i64,
        stop_ns: i64,
        entry_slug: Option<String>,
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
    /// Diagnostic knowledge and case records for system prompt injection.
    #[allow(dead_code)] // kept for prompt rebuilds (Task 5)
    record_store: std::sync::Arc<super::records::RecordStore>,

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
        record_store: std::sync::Arc<super::records::RecordStore>,
    ) -> Self {
        let has_duckdb = cfg!(feature = "duckdb") && !duckdb_path.is_empty();
        let has_code = !code_path.is_empty();
        let tools = super::tools::tool_definitions(has_duckdb, has_code);

        let system_prompt = build_system_prompt(&model, &record_store);

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
            record_store,
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

        self.wait_for_screenshot(request_id)
    }

    /// Allocate a new request ID for navigation commands.
    fn alloc_request_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    /// Send a navigation event to the UI thread and wait for the resulting screenshot.
    ///
    /// The UI thread applies the navigation action, captures a screenshot, and
    /// responds with `UiCommand::ScreenshotData`. Caller embeds `request_id`
    /// (from `alloc_request_id()`) into the event before passing it here.
    fn request_navigation(&mut self, request_id: u64, event: AgentEvent) -> Result<String, String> {
        self.emit(event);
        self.wait_for_screenshot(request_id)
    }

    /// Block until the UI thread sends back `ScreenshotData` with the given `request_id`.
    fn wait_for_screenshot(&mut self, request_id: u64) -> Result<String, String> {
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
                        msg.push_str("## Application Source Code (pre-loaded)\n\n");
                        msg.push_str(&format!(
                            "The file `{filename}` is already included below — do NOT \
                             call `read_code` for this file.\n\n\
                             ### {filename}\n```\n{contents}\n```\n\n"
                        ));
                    }
                    Err(e) => {
                        msg.push_str(&format!(
                            "## Application Source Code\n\n\
                             Note: Could not read `{}`: {}\n\n",
                            file_path, e
                        ));
                    }
                }
                if let Ok(tree) = super::tools::recursive_file_tree(&self.code_path) {
                    msg.push_str("### Other files available via `list_files` / `read_code`:\n");
                    msg.push_str(&tree);
                    msg.push('\n');
                }
            } else {
                // ── User pointed at a directory — scan for source files ──
                let file_tree = super::tools::recursive_file_tree(&self.code_path).ok();

                match gather_application_code(&self.code_path) {
                    Some(code_block) => {
                        msg.push_str("## Application Source Code\n\n");
                        msg.push_str(&code_block);
                        msg.push('\n');
                        if let Some(tree) = &file_tree {
                            msg.push_str(
                                "### Additional files available via `list_files` / `read_code`:\n",
                            );
                            msg.push_str(tree);
                            msg.push('\n');
                        }
                    }
                    None => {
                        msg.push_str(&format!(
                            "## Application Source Code\n\n\
                             Source code directory: `{}`\n",
                            self.code_path
                        ));
                        if let Some(tree) = &file_tree {
                            msg.push_str(
                                "Available files (use `list_files` to browse, `read_code` to read):\n",
                            );
                            msg.push_str(tree);
                        } else {
                            msg.push_str(
                                "No source files found. Use `list_files` tool to browse.\n",
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
            "\nStart by reading the overview data above carefully — it contains ~24 pre-computed \
             diagnostic signals including utilization, tracing status, deferred health, utility \
             breakdown, channel direction, copy burden, and navigation anchors with nanosecond \
             timestamps. Then take a screenshot. Use the Navigation Anchors to zoom directly to \
             the suggested steady-state region rather than viewing the full timeline — Bauer's \
             method: 'Don't start at the beginning. Start in the middle.' Use set_view to zoom \
             and scroll simultaneously. Describe what you see in the screenshot: which rows are \
             dense vs sparse, where the gaps are, what's happening on utility/channel rows during \
             application processor gaps, whether gaps are synchronized or staggered. These gestalt \
             patterns are your most reliable visual observations. Now form your initial hypothesis \
             by combining the overview signals with the visual patterns. State your hypothesis \
             explicitly before proceeding. Then immediately identify what signal would FALSIFY \
             your hypothesis and check it. For example, if you suspect runtime overhead, verify \
             that utility processors are actually busy (>30% in the overview). If they are idle, \
             your hypothesis is wrong — revise it before proceeding. If all signals indicate a \
             healthy profile (application processor utilization >75%, tracing active, deferred \
             P10 >1ms, utility <50%), report that the profile is healthy. Do not search for \
             problems that don't exist. Then use run_query to quantify what you see and walk \
             causal chains with the recursive CTE. Call run_query multiple times per response to \
             batch independent queries.",
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
                        full_content: if content.starts_with("__IMAGE_BASE64__") {
                            String::new()
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

            "list_files" => {
                let path = input
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                super::tools::execute_list_files(&self.code_path, path)
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

            "pan" => {
                let direction = input
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .ok_or("pan requires direction (\"left\" or \"right\")")?;
                if direction != "left" && direction != "right" {
                    return Err("direction must be \"left\" or \"right\"".into());
                }
                let percent = input
                    .get("percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(25.0)
                    .clamp(1.0, 200.0);
                let request_id = self.alloc_request_id();
                self.request_navigation(request_id, AgentEvent::PanRequest {
                    request_id,
                    direction: direction.to_owned(),
                    percent,
                })
            }

            "scroll_to" => {
                let entry_slug = input
                    .get("entry_slug")
                    .and_then(|v| v.as_str())
                    .ok_or("scroll_to requires entry_slug (string)")?;
                let request_id = self.alloc_request_id();
                self.request_navigation(request_id, AgentEvent::ScrollToRequest {
                    request_id,
                    entry_slug: entry_slug.to_owned(),
                })
            }

            "set_view" => {
                let start_ns = input
                    .get("start_ns")
                    .and_then(|v| v.as_i64())
                    .ok_or("set_view requires start_ns (integer)")?;
                let stop_ns = input
                    .get("stop_ns")
                    .and_then(|v| v.as_i64())
                    .ok_or("set_view requires stop_ns (integer)")?;
                let entry_slug = input
                    .get("entry_slug")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned());
                let request_id = self.alloc_request_id();
                self.request_navigation(request_id, AgentEvent::SetViewRequest {
                    request_id,
                    start_ns,
                    stop_ns,
                    entry_slug,
                })
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

// ── Highlight extraction ─────────────────────────────────────────────────────

fn build_system_prompt(model: &str, record_store: &super::records::RecordStore) -> String {
    let n_cases = record_store.record_count();
    let base = format!(
        r##"You are a Legion Runtime performance diagnostician. You have:
- Complete Legion domain knowledge (how the runtime works)
- Profiler signal reference (what each measurement means)
- Profiler data guide (what data is available and what isn't)
- {} documented diagnostic cases (known patterns with fixes and gotchas)
- Expert diagnostic examples showing how Bauer debugs profiles
- A pre-computed profiling overview with ~24 diagnostic signals
- Navigation anchors (nanosecond timestamps for direct zoom_to/set_view)
- A DuckDB database for follow-up queries
- Screenshots of the timeline
- Application source code

Your job: determine whether this application has performance issues, and if so, identify what's preventing it from achieving maximum performance. If the profile is healthy — high utilization on application processors, active tracing, healthy deferred, no saturated utility — say so. Do not manufacture problems. A healthy profile is a valid diagnosis. Follow causal chains through the data. Match your findings against the known cases. Adapt the fix to the user's code.

Before each investigation step, state what you expect to learn and how it will narrow the diagnosis. After each tool result, state whether it confirmed or changed your hypothesis.

Report a root cause only when you have confirming evidence from at least two independent sources (e.g., a query result AND a visual observation, or two different queries confirming the same finding). If you cannot reach this threshold, state your finding as preliminary and identify what additional evidence would confirm it.

MANDATORY CROSS-VALIDATION: After forming a hypothesis, identify the single strongest signal that would FALSIFY it, and check that signal before committing to the diagnosis. Examples:
- If you hypothesize 'runtime overhead': check utility processor utilization. If utility is below 30%, runtime overhead is NOT the cause — the runtime is idle, not overloaded. Look for application-level blocking (convergence checks, futures, barriers).
- If you hypothesize 'missing tracing': check for Replay Physical Trace tasks in the Tracing Status section. If RPT count > 0, tracing IS active — do not recommend -dm:memoize.
- If you hypothesize 'mapper placement bug': check channel direction. If no SYS↔FB channels exist, placement is not the issue.
- If you hypothesize 'insufficient parallelism': check if utility is busy. If utility is saturated, the problem is runtime overhead, not lack of work.
State the falsification check and its result explicitly in your analysis before reporting each finding.

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

## Compact Reference

### Overview interpretation
- The pre-computed overview provides classification, utilization, tracing status, deferred health, utility breakdown, mapper analysis, task granularity, channel direction, copy burden, GC activity, scheduling overhead, processor balance, per-node utility balance, and navigation anchors.
- Sections showing "Not available in this profile" mean that column does not exist in this profile's DuckDB export. Do NOT attempt to query those columns yourself — the data genuinely isn't there.
- Read the full overview before taking any action. The overview often contains enough information to form your initial hypothesis.

### Navigation anchors
- The overview ends with Navigation Anchors containing nanosecond timestamps. Use these with zoom_to or set_view to navigate directly to the largest gap, the worst mapper call, or the suggested steady-state region.
- Prefer using anchors over generic full-timeline screenshots. Bauer's method: "Don't start at the beginning. Start in the middle."

### Chain-walking CTE template
Walk the critical path recursively (up to 10 hops):
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
Interpret: chain to utility = runtime overhead; chain to channel = data movement; large deferred = healthy run-ahead; NULL critical_path = chain ended (do NOT fabricate links — try `creator` or check utility/channel activity instead).

### Visual analysis
**Reliable observations** (start hypotheses from these):
- Gestalt patterns: synchronized gaps, one row emptier than others, periodic patterns, phase transitions.
- Relative density: which processor kind has the most gaps.
- Temporal correlation: what OTHER rows show during a gap.

**Unreliable observations** (ALWAYS verify with queries):
- Exact utilization percentages — query instead.
- Row identification beyond ~10 rows — use the entry_slug list from metadata.
- Color-to-task mapping — use the color legend in metadata.
- Duration of individual gaps — query for nanosecond-precise timing.

### Source code key points
- Check for tracing annotations (begin_trace/end_trace, __demand(__trace)) BEFORE recommending -dm:memoize
- Check for custom mapper (inherits from DefaultMapper/Mapping) — -dm:memoize only affects DefaultMapper
- Use `list_files` to discover files before `read_code` — do NOT guess filenames
- What source code CANNOT tell you: runtime scheduling, memory layout, NUMA placement, Realm worker behavior

### SQL error handling
- When a query fails, read the error message and HINT carefully
- Fix column names, types, or syntax before retrying
- Do not retry the same query unchanged
- Maximum 3 retries per query intent

### DuckDB syntax quick reference
- STRUCT field access: running.start, critical_path.item_uid (dot notation, not brackets)
- Conditional count: COUNT(*) FILTER (WHERE condition)
- Safe cast: TRY_CAST(x AS BIGINT) returns NULL on failure (use for size column)
- Percentiles: PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY col)
- Avoid division by zero: GREATEST(denominator, 1)
- GROUP BY ALL: groups by all non-aggregate SELECT columns automatically
- All timestamps are BIGINT nanoseconds. Divide by 1e6 for milliseconds.
- size column is VARCHAR in some profiles — always use TRY_CAST, never bare CAST
- The full schema is in the overview's Schema section. Check column names there before writing any query.

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
{{"highlights": [{{"entry_slug": "n0_gpu_g0", "start_ns": 670000000, "stop_ns": 759000000, "severity": "critical", "label": "89ms GPU idle — missing tracing"}}]}}
```

Rules:
- `entry_slug` must match a slug from the profiling database (e.g. `n0_cpu_c0`, `n0_gpu_g0`)
- Use RELATIVE severity thresholds above
- Place highlights JSON as the LAST block — the parser expects it at the end
- No issues? `{{"highlights": []}}`

## Navigation vs Analysis

Distinguish between navigation commands and analysis requests:

**Navigation-only commands** — the user wants you to move the view, NOT analyze. Just execute the navigation tool, briefly describe what is now visible, and stop. Do NOT run queries or provide analysis unless asked. Examples:
- "zoom into the utility processors" → use zoom_to/set_view to show utility rows, describe what you see visually, stop.
- "pan right" → use pan, describe what's now visible, stop.
- "scroll to the GPU rows" → use scroll_to, describe what's now visible, stop.
- "show me [time range]" → use zoom_to, describe what's now visible, stop.

**Analysis requests** — the user wants investigation. Run the full diagnostic protocol. Examples:
- "why is this gap here?" → analyze with queries + screenshots.
- "what's causing the overhead?" → full diagnostic protocol.
- "analyze the utility processor overhead" → queries + root cause.
- "find performance issues" → full scan.

When in doubt: if the user's message is a short imperative about viewing (zoom, pan, scroll, show), treat it as navigation-only. If it asks why/what/how or mentions analysis/diagnosis/issues, treat it as an analysis request."##,
        n_cases
    );

    // Append model-specific analysis scope
    let suffix = if model.contains("opus") {
        "\n\n## Analysis Scope\n\
         Be thorough. Trace causal chains to their root \
         with the recursive CTE. Cross-validate every finding with both visual and query evidence. \
         Check for co-occurring causes."
    } else {
        "\n\n## Analysis Scope\n\
         You have limited output capacity. Focus on the single most impactful finding. \
         Identify the dominant issue from the overview, then proceed directly \
         to root-cause analysis for that issue only. Be accurate on one issue with full evidence rather \
         than shallow on many."
    };

    // Inject knowledge from RecordStore (domain model, signal reference, cases, expert traces)
    let knowledge = record_store.system_context();
    if knowledge.is_empty() {
        format!("{}{}", base, suffix)
    } else {
        format!("{}{}\n\n{}", base, suffix, knowledge)
    }
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
