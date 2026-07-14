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
use tracing::{Span, info_span};

// ── Public response types ────────────────────────────────────────────────────

// ── API-call tuning ──────────────────────────────────────────────────────────

/// Response token budget per request. Opus gets headroom for extended thinking.
const MAX_TOKENS_OPUS: u32 = 16_000;
const MAX_TOKENS_SONNET: u32 = 8_000;
/// One API request may legitimately take minutes (large prompts, thinking).
const API_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// Exponential backoff on 429/529: base doubling per retry, bounded ceiling.
const API_MAX_ATTEMPTS: u32 = 5;
const API_RETRY_BASE_MS: u64 = 1_000;
const API_RETRY_CEILING_MS: u64 = 60_000;

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
    /// Compact token/cost line from the backend's terminal usage report (the
    /// Claude Code `result` event). The embedded loop leaves it `None`.
    /// Display-only — the panel appends it to the "Done." line.
    pub usage_note: Option<String>,
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
    ToolResult {
        name: String,
        summary: String,
        full_content: String,
    },
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
    ScrollToRequest { request_id: u64, entry_slug: String },
    /// Agent wants to zoom + optionally scroll/filter/expand in one call.
    SetViewRequest {
        request_id: u64,
        start_ns: i64,
        stop_ns: i64,
        entry_slug: Option<String>,
        /// Show only these processor kinds (empty/None = show all).
        filter_kinds: Option<Vec<String>>,
        /// Expand these processor kinds.
        expand_kinds: Option<Vec<String>>,
        /// Collapse these processor kinds.
        collapse_kinds: Option<Vec<String>>,
        /// Vertical row scale (0.25–4.0); None leaves it unchanged.
        vertical_scale: Option<f64>,
    },
    /// Agent wants to set the timeline search query (highlights matching tasks).
    SearchRequest { request_id: u64, query: String },
    /// Agent wants to reset zoom, vertical spacing, and kind filters.
    ResetViewRequest { request_id: u64 },
    /// Agent is asking the user a clarifying question (human-in-the-loop).
    /// Blocks the agent thread until the UI sends back `UiCommand::UserAnswer`.
    QuestionForUser {
        request_id: u64,
        question: String,
        options: Vec<String>,
    },
    /// Agent wants all timeline highlight overlays cleared.
    ClearHighlights,
    /// MCP-driven: apply a highlight overlay to the live timeline, then ACK
    /// via `UiCommand::Ack`. The embedded agent accumulates highlights in
    /// `run_highlights` and never emits this — only the in-viewer MCP bridge does.
    HighlightRequest {
        request_id: u64,
        entry_slug: String,
        start_ns: i64,
        stop_ns: i64,
        severity: String,
        label: String,
    },
    /// MCP-driven: clear all highlight overlays, then ACK via
    /// `UiCommand::Ack`. The embedded agent emits the reply-less `ClearHighlights`.
    ClearHighlightsRequest { request_id: u64 },
    /// MCP-driven: READ the human's current timeline selection and reply via
    /// `UiCommand::SelectionData`. A non-driving read — the bridge services it
    /// WITHOUT claiming the viewport token. The embedded agent reads its own
    /// selection state directly (`build_selection_preamble`) and never emits this.
    GetSelection { request_id: u64 },
    /// Claude Code backend: an INTERIM assistant text message from the streamed
    /// transcript — narration between tool calls, rendered progressively so the
    /// chat feels alive during long runs. The final turn text still arrives via
    /// `Complete` (deduplicated by the emitter). The native agent never emits this.
    InterimText { text: String },
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
    /// The user's answer to a `QuestionForUser`.
    UserAnswer { request_id: u64, answer: String },
    /// Acknowledgement for a non-screenshot bridge request (highlight / clear).
    /// `message` is the model-readable confirmation text. Used only by the
    /// in-viewer MCP bridge path.
    Ack { request_id: u64, message: String },
    /// The human's current timeline selection, in reply to `GetSelection`.
    /// `items` are selected task bars; `range` is the dragged region (entry label,
    /// start_ns, stop_ns), if any. Both empty/None ⇒ nothing selected.
    SelectionData {
        request_id: u64,
        items: Vec<SelectedItemInfo>,
        range: Option<(String, i64, i64)>,
    },
}

/// One selected task bar reported by `get_selection` over the bridge. A
/// self-contained mirror of the chat panel's `SelectedItem`, so the UI↔bridge
/// command layer carries no dependency on the chat-panel type. The UI drain maps
/// `SelectedItem` → this when building [`UiCommand::SelectionData`].
#[derive(Debug, Clone)]
pub struct SelectedItemInfo {
    pub item_uid: u64,
    pub entry_slug: Option<String>,
    pub title: String,
    pub start_ns: i64,
    pub stop_ns: i64,
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
    /// Path to application source code root directory (used by read_code/list_files).
    pub code_path: String,
    /// Path to the Legion wiki root (used by wiki_index/wiki_read/wiki_search).
    pub wiki_path: String,
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

    /// Shared viewport-ownership token. When set, the screenshot/navigation
    /// path claims it for each round-trip so the embedded agent and an external
    /// MCP driver are mutually exclusive. `None` (tests, or before the chat panel
    /// wires it) keeps the sole-driver behavior: no claim, always proceeds.
    viewport_token: Option<super::bridge::ViewportToken>,
    /// Consumer id used when claiming `viewport_token`.
    viewport_consumer_id: u64,

    /// Highlights emitted via the `highlight` tool during the current run,
    /// merged into the final `AgentResponse`.
    run_highlights: Vec<Highlight>,

    /// Durable conclusions the model records via `update_findings`; persisted
    /// across questions and re-injected at the top of each new user message.
    findings: Vec<String>,

    /// Stable identifier for the entire session, used as a discriminator on
    /// every emitted span so a JSONL file containing many sessions can be
    /// split with `jq --arg sid X 'select(.session_id == $sid)'`.
    pub session_id: String,
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
        wiki_path: String,
        event_tx: mpsc::Sender<AgentEvent>,
        command_rx: mpsc::Receiver<UiCommand>,
    ) -> Self {
        let has_duckdb = cfg!(feature = "duckdb") && !duckdb_path.is_empty();
        let has_code = !code_path.is_empty();
        let has_wiki = !wiki_path.is_empty();
        let tools = super::tools::tool_definitions(has_duckdb, has_code, has_wiki);

        let system_prompt = build_system_prompt(has_code);

        Self {
            messages: Vec::new(),
            api_key,
            model,
            duckdb_path,
            code_path,
            wiki_path,
            max_turns: 25,
            system_prompt,
            tools,
            event_tx,
            command_rx,
            next_request_id: 0,
            viewport_token: None,
            viewport_consumer_id: super::bridge::EMBEDDED_CONSUMER_ID,
            run_highlights: Vec::new(),
            findings: Vec::new(),
            session_id: super::trace::new_session_id(),
        }
    }

    /// Follow-up question. The full conversation history is preserved so Claude
    /// has context from the initial scan.
    pub fn ask(&mut self, question: &str) -> Result<AgentResponse, String> {
        let _run = info_span!(
            "agent.run",
            session_id = %self.session_id,
            kind = "ask",
            model = %self.model,
            max_turns = self.max_turns,
            total_turns = tracing::field::Empty,
            queries_executed = tracing::field::Empty,
            n_highlights = tracing::field::Empty,
        )
        .entered();
        let response = self.run_agent_loop(question.to_owned())?;
        let span = Span::current();
        span.record("total_turns", response.turns_used as u64);
        span.record("queries_executed", response.queries_executed as u64);
        span.record("n_highlights", response.highlights.len() as u64);
        Ok(response)
    }

    /// Record a finding (or replace all findings when `replace`). Multi-line
    /// notes become separate bullets, each stripped of leading markers and
    /// length-capped; the list is bounded to `MAX_FINDINGS` (oldest dropped).
    fn add_finding(&mut self, note: &str, replace: bool) {
        if replace {
            self.findings.clear();
        }
        for line in note.lines() {
            let cleaned = line.trim().trim_start_matches(['-', '*', '•', ' ']).trim();
            if cleaned.is_empty() {
                continue;
            }
            self.findings
                .push(truncate_on_boundary(cleaned, MAX_FINDING_CHARS).to_owned());
        }
        while self.findings.len() > MAX_FINDINGS {
            self.findings.remove(0);
        }
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

    /// Re-read the (user-editable) project folder on a REUSED session — without
    /// this, setting the path mid-conversation would silently do nothing until
    /// ↺ New session. When the value changes, the tool list and system prompt
    /// are rebuilt so `read_code`/`list_files` (and the code-pointer bullet)
    /// appear/disappear on the next request. Rebuilding the system prompt
    /// invalidates the prompt cache — fine for a rare, user-initiated change.
    pub fn refresh_code_path(&mut self, code_path: &str) {
        if self.code_path == code_path {
            return;
        }
        self.code_path = code_path.to_owned();
        let has_duckdb = cfg!(feature = "duckdb") && !self.duckdb_path.is_empty();
        let has_code = !self.code_path.is_empty();
        let has_wiki = !self.wiki_path.is_empty();
        self.tools = super::tools::tool_definitions(has_duckdb, has_code, has_wiki);
        self.system_prompt = build_system_prompt(has_code);
    }

    /// Wire the shared viewport token. After this, the screenshot/navigation
    /// path claims `token` for each round-trip under `consumer_id`, so the embedded
    /// agent and the in-viewer MCP driver are mutually exclusive (single outstanding
    /// screenshot across both). Idempotent. When never called, the agent stays the
    /// transparent sole driver (no claim).
    pub fn set_viewport(&mut self, token: super::bridge::ViewportToken, consumer_id: u64) {
        self.viewport_token = Some(token);
        self.viewport_consumer_id = consumer_id;
    }

    /// Send an event to the UI thread. Silently ignores send failures
    /// (which happen if the UI dropped its receiver).
    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Claim the shared viewport for a viewport round-trip. Returns an RAII
    /// guard the caller holds for the duration of emit+wait — releasing it on every
    /// exit path. `Err` with a model-readable "viewport busy" message if an external
    /// driver holds it. When no token is wired, returns `Ok(None)` and the request
    /// proceeds as the sole driver.
    fn claim_viewport(&self) -> Result<Option<super::bridge::ViewportGuard>, String> {
        match &self.viewport_token {
            Some(token) => token
                .try_claim(self.viewport_consumer_id)
                .map(Some)
                .map_err(|_| {
                    "viewport busy: an external driver (Claude Code via the in-viewer MCP \
                 server) is currently controlling the timeline. Retry shortly."
                        .to_string()
                }),
            None => Ok(None),
        }
    }

    /// Request a screenshot from the UI thread and wait for the response.
    ///
    /// Emits a `ScreenshotRequest` or `ZoomRequest` event, then blocks on
    /// `command_rx` until the UI sends back `ScreenshotData` with matching
    /// `request_id`. Returns the base64-encoded PNG string (prefixed with
    /// `__IMAGE_BASE64__` so the caller can build an image content block).
    fn request_screenshot(&mut self, zoom_range: Option<(i64, i64)>) -> Result<String, String> {
        // Hold the viewport for the whole round-trip (claim -> emit -> await PNG ->
        // release on drop). Single outstanding screenshot across embedded + MCP.
        let _viewport = self.claim_viewport()?;
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
        // Hold the viewport across the nav + screenshot round-trip (released on drop).
        let _viewport = self.claim_viewport()?;
        self.emit(event);
        self.wait_for_screenshot(request_id)
    }

    /// Block until the UI thread sends a `UiCommand` that `extract` accepts, or
    /// until `timeout` elapses. Stale/mismatched commands are discarded. Shared
    /// request/response primitive for both screenshots and `ask_user`.
    ///
    /// Safe because sub-agents (if any) run sequentially on this one thread, so
    /// there is only ever a single consumer of `command_rx`.
    fn wait_for_command<T>(
        &mut self,
        timeout: std::time::Duration,
        mut extract: impl FnMut(UiCommand) -> Option<Result<T, String>>,
    ) -> Result<T, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match self.command_rx.recv_timeout(remaining) {
                Ok(cmd) => {
                    if let Some(result) = extract(cmd) {
                        return result;
                    }
                    // Mismatched / stale command — keep waiting.
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err("Timed out waiting for the UI thread.".into());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("UI command channel disconnected.".into());
                }
            }
        }
    }

    /// Block until the UI thread sends back `ScreenshotData` with the given `request_id`.
    fn wait_for_screenshot(&mut self, request_id: u64) -> Result<String, String> {
        self.wait_for_command(std::time::Duration::from_secs(10), move |cmd| match cmd {
            UiCommand::ScreenshotData {
                request_id: rid,
                png_bytes,
                metadata,
            } if rid == request_id => {
                if png_bytes.is_empty() {
                    return Some(Err("Screenshot capture returned empty data.".into()));
                }
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                Some(Ok(format!(
                    "__IMAGE_BASE64__{encoded}\n__METADATA__{metadata}"
                )))
            }
            _ => None,
        })
    }

    /// Ask the user a clarifying question and block (up to 5 minutes) for an answer.
    fn ask_user(&mut self, question: &str, options: Vec<String>) -> Result<String, String> {
        let request_id = self.alloc_request_id();
        self.emit(AgentEvent::QuestionForUser {
            request_id,
            question: question.to_owned(),
            options,
        });
        self.wait_for_command(std::time::Duration::from_secs(300), move |cmd| match cmd {
            UiCommand::UserAnswer {
                request_id: rid,
                answer,
            } if rid == request_id => Some(Ok(answer)),
            _ => None,
        })
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    /// Core agentic loop. Appends `user_message` to history, then iterates:
    /// tool_use → execute tools → send results → repeat until end_turn.
    fn run_agent_loop(&mut self, user_message: String) -> Result<AgentResponse, String> {
        // Append the new user message, prepending the running findings (the
        // model's own notes) so they ride at the live edge of context.
        let user_content = if self.findings.is_empty() {
            user_message
        } else {
            format!("{}\n\n{}", render_findings(&self.findings), user_message)
        };
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": user_content
        }));
        // Reset per-run highlight accumulator (tool-emitted highlights).
        self.run_highlights.clear();

        let mut turns = 0usize;
        let mut queries_executed = 0usize;
        let mut force_summary_sent = false;

        loop {
            turns += 1;
            let _turn = info_span!(
                "agent.turn",
                turn_number = turns,
                stop_reason = tracing::field::Empty,
                n_tool_calls = tracing::field::Empty,
                wall_ms = tracing::field::Empty,
            )
            .entered();
            let turn_started = std::time::Instant::now();

            // Compact stale bulk (old screenshots, oversized query results) before
            // each model call to curb token growth and context rot.
            let (imgs, chars) = compact_messages(&mut self.messages);
            if imgs > 0 || chars > 0 {
                tracing::debug!(
                    images_stripped = imgs,
                    chars_saved = chars,
                    "compacted history"
                );
            }

            let response = self.call_claude()?;
            Span::current().record("stop_reason", response.stop_reason.as_str());

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
                Span::current().record("n_tool_calls", 0u64);
                Span::current().record("wall_ms", turn_started.elapsed().as_millis() as u64);
                let mut highlights = std::mem::take(&mut self.run_highlights);
                highlights.extend(parse_highlights_from_text(&response_text));
                return Ok(AgentResponse {
                    text: response_text,
                    highlights,
                    queries_executed,
                    turns_used: turns,
                    usage_note: None,
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
                Span::current().record("n_tool_calls", 0u64);
                Span::current().record("wall_ms", turn_started.elapsed().as_millis() as u64);
                let mut highlights = std::mem::take(&mut self.run_highlights);
                highlights.extend(parse_highlights_from_text(&response_text));
                return Ok(AgentResponse {
                    text: response_text,
                    highlights,
                    queries_executed,
                    turns_used: turns,
                    usage_note: None,
                });
            }
            Span::current().record("n_tool_calls", tool_use_blocks.len() as u64);

            // Execute all tool calls and collect results
            let tool_results: Vec<Value> = tool_use_blocks
                .iter()
                .map(|block| self.tool_result_block(block, &mut queries_executed))
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
                                now, marking any timeline regions with the `highlight` tool."
                }));
            }

            Span::current().record("wall_ms", turn_started.elapsed().as_millis() as u64);
        }
    }

    /// Execute one `tool_use` block and package the outcome as a `tool_result`
    /// content block for the next API request, emitting the progressive
    /// `ToolCall` / `ToolResult` status events along the way. Bumps
    /// `queries_executed` when the tool is `run_query`.
    ///
    /// This is the single decoder of the stringly-typed screenshot protocol:
    /// viewport-capturing tools (`screenshot`, `zoom_to`, and the navigation
    /// tools) return `Ok` strings of the form `__IMAGE_BASE64__<base64 PNG>`,
    /// optionally followed by `\n__METADATA__<viewport description>` (encoded
    /// by `wait_for_screenshot`). Such results become a `tool_result` whose
    /// content is an image block plus, when metadata is present, a text block,
    /// so Claude sees the pixels and the viewport description together. All
    /// other results (including errors) pass through as a plain string
    /// `content` with `is_error` set accordingly.
    fn tool_result_block(&mut self, block: &Value, queries_executed: &mut usize) -> Value {
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
            *queries_executed += 1;
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
            let (base64_data, metadata) = if let Some(meta_pos) = content.find("\n__METADATA__") {
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
    }

    /// Returns true if `slug` is a known `entry_slug` in the profile's `entries`
    /// table.
    ///
    /// The slug comes from untrusted model tool input, so it is NEVER
    /// interpolated into the SQL (which would make the validation query itself an
    /// injection vector — `execute_run_query_raw` builds SQL by `format!`, not
    /// parameters). Instead a CONSTANT query fetches the valid-slug set and
    /// membership is tested in Rust. The slugs are aggregated into a SINGLE row
    /// via `json_group_array` so the 50-row cap in `execute_run_query_raw` cannot
    /// truncate the set (the `entries` table can exceed 50 rows).
    #[cfg(feature = "duckdb")]
    fn slug_exists(&self, slug: &str) -> bool {
        super::tools::slug_exists(&self.duckdb_path, slug)
    }

    /// Dispatch a tool call to the appropriate tool function.
    ///
    /// Screenshot and zoom_to results are returned with a `__IMAGE_BASE64__`
    /// prefix so the caller can build an image content block for Claude's
    /// vision capability.
    fn execute_tool(&mut self, name: &str, input: &Value) -> Result<String, String> {
        let _t = info_span!(
            "agent.tool_call",
            tool_name = name,
            args_size = input.to_string().len() as u64,
            result_size = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
            error = tracing::field::Empty,
        )
        .entered();
        let tool_started = std::time::Instant::now();

        // IIFE so `?` inside arms returns to the closure, not skipping our
        // post-call span recording below.
        let mut inner = || -> Result<String, String> {
            match name {
                "run_query" => {
                    #[cfg(feature = "duckdb")]
                    {
                        let sql = req_str(input, "sql")?;
                        super::tools::execute_run_query(&self.duckdb_path, sql)
                    }
                    #[cfg(not(feature = "duckdb"))]
                    {
                        let _ = input;
                        Err(
                            "DuckDB support not compiled in. Rebuild with --features duckdb."
                                .into(),
                        )
                    }
                }

                "overview" => {
                    let _ = input;
                    #[cfg(feature = "duckdb")]
                    {
                        super::tools::gather_overview(&self.duckdb_path)
                    }
                    #[cfg(not(feature = "duckdb"))]
                    {
                        Err(
                            "DuckDB support not compiled in. Rebuild with --features duckdb."
                                .into(),
                        )
                    }
                }

                "list_files" => {
                    let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                    super::tools::execute_list_files(&self.code_path, path)
                }

                "read_code" => {
                    let path = req_str(input, "path")?;
                    super::tools::execute_read_code(&self.code_path, path)
                }

                "wiki_index" => {
                    let section = input.get("section").and_then(|v| v.as_str());
                    super::tools::wiki_index(&self.wiki_path, section)
                }

                "wiki_read" => {
                    let path = req_str(input, "path")?;
                    let section = input.get("section").and_then(|v| v.as_str());
                    let max_chars = input
                        .get("max_chars")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize);
                    super::tools::wiki_read(&self.wiki_path, path, section, max_chars)
                }

                "wiki_search" => {
                    let query = req_str(input, "query")?;
                    let section = input.get("section").and_then(|v| v.as_str());
                    let tag = input.get("tag").and_then(|v| v.as_str());
                    let limit = input
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(5);
                    super::tools::wiki_search(&self.wiki_path, query, section, tag, limit)
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
                    self.request_navigation(
                        request_id,
                        AgentEvent::PanRequest {
                            request_id,
                            direction: direction.to_owned(),
                            percent,
                        },
                    )
                }

                "scroll_to" => {
                    let entry_slug = input
                        .get("entry_slug")
                        .and_then(|v| v.as_str())
                        .ok_or("scroll_to requires entry_slug (string)")?;
                    let request_id = self.alloc_request_id();
                    self.request_navigation(
                        request_id,
                        AgentEvent::ScrollToRequest {
                            request_id,
                            entry_slug: entry_slug.to_owned(),
                        },
                    )
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
                    let str_array = |key: &str| -> Option<Vec<String>> {
                        input.get(key).and_then(|v| v.as_array()).map(|arr| {
                            arr.iter()
                                .filter_map(|x| x.as_str().map(str::to_owned))
                                .collect()
                        })
                    };
                    let vertical_scale = input.get("vertical_scale").and_then(|v| v.as_f64());
                    let request_id = self.alloc_request_id();
                    self.request_navigation(
                        request_id,
                        AgentEvent::SetViewRequest {
                            request_id,
                            start_ns,
                            stop_ns,
                            entry_slug,
                            filter_kinds: str_array("filter_kinds"),
                            expand_kinds: str_array("expand_kinds"),
                            collapse_kinds: str_array("collapse_kinds"),
                            vertical_scale,
                        },
                    )
                }

                "search" => {
                    let query = input
                        .get("query")
                        .and_then(|v| v.as_str())
                        .ok_or("search requires query (string)")?
                        .to_owned();
                    let request_id = self.alloc_request_id();
                    self.request_navigation(
                        request_id,
                        AgentEvent::SearchRequest { request_id, query },
                    )
                }

                "reset_view" => {
                    let request_id = self.alloc_request_id();
                    self.request_navigation(request_id, AgentEvent::ResetViewRequest { request_id })
                }

                "highlight" => {
                    let entry_slug = input
                        .get("entry_slug")
                        .and_then(|v| v.as_str())
                        .ok_or("highlight requires entry_slug (string)")?;
                    let start_ns = input
                        .get("start_ns")
                        .and_then(|v| v.as_i64())
                        .ok_or("highlight requires start_ns (integer)")?;
                    let stop_ns = input
                        .get("stop_ns")
                        .and_then(|v| v.as_i64())
                        .ok_or("highlight requires stop_ns (integer)")?;
                    let severity = input
                        .get("severity")
                        .and_then(|v| v.as_str())
                        .unwrap_or("medium")
                        .to_owned();
                    let label = input
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    // Reject unknown slugs explicitly so an invalid highlight does not
                    // silently no-op at render time while reporting success. Gated by a
                    // cfg ATTRIBUTE (not cfg!()) so the call to the duckdb-only
                    // `slug_exists` is textually absent under the {ai}-only combo.
                    #[cfg(feature = "duckdb")]
                    {
                        if !self.slug_exists(entry_slug) {
                            return Err(format!(
                                "highlight: unknown entry_slug '{entry_slug}'. \
                             Query `SELECT entry_slug FROM entries` for valid slugs."
                            ));
                        }
                    }
                    self.run_highlights.push(Highlight {
                        entry_slug: entry_slug.to_owned(),
                        start_ns,
                        stop_ns,
                        severity,
                        label,
                    });
                    Ok(format!(
                        "Highlight added on {entry_slug} [{start_ns}, {stop_ns}]."
                    ))
                }

                "ask_user" => {
                    let question = input
                        .get("question")
                        .and_then(|v| v.as_str())
                        .ok_or("ask_user requires question (string)")?;
                    let options = input
                        .get("options")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|o| o.as_str().map(str::to_owned))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    self.ask_user(question, options)
                }

                "clear_highlights" => {
                    // Report what actually happened instead of always claiming success.
                    // Clear the run accumulator too, so the count is truthful and the
                    // cleared highlights do not still appear in the final response.
                    let n = self.run_highlights.len();
                    if n == 0 {
                        Ok("No highlights to clear.".to_owned())
                    } else {
                        self.emit(AgentEvent::ClearHighlights);
                        self.run_highlights.clear();
                        Ok(format!("Cleared {n} highlight(s)."))
                    }
                }

                "update_findings" => {
                    let note = input
                        .get("note")
                        .and_then(|v| v.as_str())
                        .ok_or("update_findings requires note (string)")?;
                    let replace = input
                        .get("replace")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    self.add_finding(note, replace);
                    Ok(format!(
                        "Noted. Tracking {} finding(s).",
                        self.findings.len()
                    ))
                }

                _ => Err(format!("Unknown tool: {name}")),
            }
        };
        let result = inner();

        let span = Span::current();
        span.record("duration_ms", tool_started.elapsed().as_millis() as u64);
        match &result {
            Ok(s) => {
                span.record("result_size", s.len() as u64);
            }
            Err(e) => {
                span.record("error", e.as_str());
            }
        }
        result
    }

    /// POST the current messages to the Claude API with exponential backoff on 429/529.
    ///
    /// When using an Opus model, adaptive thinking (`"type": "adaptive"`) is
    /// enabled with `output_config.effort = "high"` and max_tokens is doubled to
    /// 16 000. `claude-opus-4-8` rejects a fixed thinking budget, so we must use
    /// adaptive thinking here. Thinking is NOT added on Sonnet (the fast path) —
    /// the guard is `model.contains("opus")`.
    fn call_claude(&self) -> Result<ApiResponse, String> {
        let use_opus = self.model.contains("opus");

        let _api = info_span!(
            "agent.claude_api",
            model = %self.model,
            n_messages = self.messages.len() as u64,
            n_tools = self.tools.len() as u64,
            extended_thinking = use_opus,
            request_size = tracing::field::Empty,
            response_size = tracing::field::Empty,
            latency_ms = tracing::field::Empty,
            retries = tracing::field::Empty,
            http_status = tracing::field::Empty,
            stop_reason = tracing::field::Empty,
            tokens_input = tracing::field::Empty,
            tokens_output = tracing::field::Empty,
            cache_read = tracing::field::Empty,
            cache_creation = tracing::field::Empty,
            error = tracing::field::Empty,
        )
        .entered();
        let api_started = std::time::Instant::now();

        // Opus benefits from a larger token budget for its thinking + response.
        let max_tokens: u32 = if use_opus {
            MAX_TOKENS_OPUS
        } else {
            MAX_TOKENS_SONNET
        };

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

        // Extended thinking — Opus only. `claude-opus-4-8` does not accept a fixed
        // thinking budget (`{"type":"enabled","budget_tokens":N}` returns a 400),
        // so we use adaptive thinking and steer depth via `output_config.effort`.
        // Sonnet keeps no thinking block.
        if use_opus {
            req_body["thinking"] = serde_json::json!({ "type": "adaptive" });
            req_body["output_config"] = serde_json::json!({ "effort": "high" });
        }

        let body_str = serde_json::to_string(&req_body)
            .map_err(|e| format!("Failed to serialize request: {e}"))?;
        Span::current().record("request_size", body_str.len() as u64);

        let mut retry_delay_ms = API_RETRY_BASE_MS;
        let mut retries_count: u32 = 0;

        // Helper to record terminal fields on every exit path.
        let finalize_err = |err: String, status: Option<u16>, retries: u32| -> String {
            let span = Span::current();
            span.record("latency_ms", api_started.elapsed().as_millis() as u64);
            span.record("retries", retries as u64);
            if let Some(s) = status {
                span.record("http_status", s as u64);
            }
            span.record("error", err.as_str());
            err
        };

        for attempt in 0..API_MAX_ATTEMPTS {
            let result = ureq::post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", &self.api_key)
                .set("anthropic-version", "2023-06-01")
                .set("Content-Type", "application/json")
                .timeout(API_REQUEST_TIMEOUT)
                .send_string(&body_str);

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.into_string().map_err(|e| {
                        finalize_err(
                            format!("Failed to read response body: {e}"),
                            Some(status),
                            retries_count,
                        )
                    })?;
                    let span = Span::current();
                    span.record("latency_ms", api_started.elapsed().as_millis() as u64);
                    span.record("retries", retries_count as u64);
                    span.record("http_status", status as u64);
                    span.record("response_size", text.len() as u64);
                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                        if let Some(usage) = json.get("usage") {
                            super::trace::record_usage(&span, usage);
                        }
                        if let Some(stop) = json.get("stop_reason").and_then(|v| v.as_str()) {
                            span.record("stop_reason", stop);
                        }
                    }
                    return serde_json::from_str::<ApiResponse>(&text).map_err(|e| {
                        let preview = &text[..text.len().min(500)];
                        finalize_err(
                            format!("Failed to parse Claude response: {e}\nBody: {preview}"),
                            Some(status),
                            retries_count,
                        )
                    });
                }

                Err(ureq::Error::Status(429 | 529, resp)) => {
                    let wait_ms = resp
                        .header("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|secs| secs * 1_000)
                        .unwrap_or(retry_delay_ms);

                    if attempt < 4 {
                        retries_count += 1;
                        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                        retry_delay_ms = (retry_delay_ms * 2).min(API_RETRY_CEILING_MS);
                        continue;
                    }
                    return Err(finalize_err(
                        format!("Rate limited after {} retries", attempt + 1),
                        Some(429),
                        retries_count,
                    ));
                }

                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    return Err(finalize_err(
                        format!("API error {code}: {}", &body[..body.len().min(500)]),
                        Some(code),
                        retries_count,
                    ));
                }

                Err(e) => {
                    return Err(finalize_err(
                        format!("Network error: {e}"),
                        None,
                        retries_count,
                    ));
                }
            }
        }

        Err(finalize_err(
            "Max retries exceeded".into(),
            None,
            retries_count,
        ))
    }
}

// ── Context compaction ──────────────────────────────────────────────────────

/// Maximum characters kept for a single non-image tool_result before truncation.
const MAX_TOOLRESULT_CHARS: usize = 3000;
/// How many leading characters to keep when truncating an oversized tool_result.
const TRUNCATE_KEEP_CHARS: usize = 2000;

/// Truncate `s` to at most `max` bytes, snapping down to a UTF-8 char boundary.
/// Fetch a required string parameter from a tool's JSON `input`, with the
/// standard model-readable error when it is absent or not a string.
fn req_str<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing '{key}' parameter"))
}

fn truncate_on_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Shrink the conversation history in place before each model call to curb token
/// growth and context rot. Two transforms, both sparing the most recent turn
/// (the agent is still reasoning over it):
///
/// 1. Strip the base64 image block from every screenshot tool_result except the
///    most recent one, leaving its text metadata (visible range + entry_slugs).
/// 2. Truncate oversized text tool_results (big run_query / overview dumps) to a
///    head slice plus a marker.
///
/// Returns `(images_stripped, chars_saved)` for logging. Cache-safe: the message
/// history is not part of the prompt-cached prefix (only system + tools are).
fn compact_messages(messages: &mut [serde_json::Value]) -> (usize, usize) {
    let mut images_stripped = 0usize;
    let mut chars_saved = 0usize;

    // --- keep images only in the most recent image-bearing message ---
    let mut kept_latest = false;
    for msg in messages.iter_mut().rev() {
        let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        let has_image = content.iter().any(|block| {
            block
                .get("content")
                .and_then(|c| c.as_array())
                .is_some_and(|tr| {
                    tr.iter()
                        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("image"))
                })
        });
        if !has_image {
            continue;
        }
        if !kept_latest {
            kept_latest = true; // most recent screenshot stays full
            continue;
        }
        // Older screenshot: drop image blocks, keep the metadata text block.
        for block in content.iter_mut() {
            if let Some(tr) = block.get_mut("content").and_then(|c| c.as_array_mut()) {
                let before = tr.len();
                tr.retain(|b| b.get("type").and_then(|t| t.as_str()) != Some("image"));
                images_stripped += before - tr.len();
                if tr.is_empty() {
                    tr.push(serde_json::json!({
                        "type": "text",
                        "text": "[earlier screenshot omitted to save context]"
                    }));
                }
            }
        }
    }

    // --- truncate oversized text tool_results, except in the last message ---
    let last_idx = messages.len().saturating_sub(1);
    for (i, msg) in messages.iter_mut().enumerate() {
        if i == last_idx {
            continue; // keep the current turn's results intact
        }
        let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for block in content.iter_mut() {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                continue;
            }
            let new_text = {
                let Some(s) = block.get("content").and_then(|c| c.as_str()) else {
                    continue;
                };
                if s.len() <= MAX_TOOLRESULT_CHARS {
                    continue;
                }
                let original_len = s.len();
                let head = truncate_on_boundary(s, TRUNCATE_KEEP_CHARS).to_owned();
                let nt = format!(
                    "{head}…\n[truncated {} of {} chars to save context — re-run with \
                     tighter filters/LIMIT if you need the rest]",
                    original_len - head.len(),
                    original_len
                );
                chars_saved += original_len.saturating_sub(nt.len());
                nt
            };
            block["content"] = serde_json::Value::String(new_text);
        }
    }

    (images_stripped, chars_saved)
}

// ── Findings scratchpad ─────────────────────────────────────────────────────

/// Maximum number of findings retained (oldest dropped beyond this).
const MAX_FINDINGS: usize = 20;
/// Maximum characters kept per individual finding.
const MAX_FINDING_CHARS: usize = 500;

/// Render the running findings as a compact block re-injected at the top of each
/// new user message — the "Write" lever for cross-question memory.
fn render_findings(findings: &[String]) -> String {
    let mut out = String::from(
        "## Findings so far (your own notes from earlier in this session — treat as \
         established context; keep them current with the update_findings tool)\n",
    );
    for f in findings {
        out.push_str("- ");
        out.push_str(f);
        out.push('\n');
    }
    out
}

// ── Highlight extraction ─────────────────────────────────────────────────────

fn build_system_prompt(has_code: bool) -> String {
    // The source-read pointer is shown only when a code root is configured (the
    // read_code/list_files tools are gated the same way). {{CODE_POINTER}} is the
    // seam in the Tools list below.
    let code_pointer = if has_code {
        "- `list_files` / `read_code`: the profiled application's source — read a task's source before explaining what it computes.\n"
    } else {
        ""
    };
    r#"You are an interactive assistant embedded in the Legion Prof timeline viewer. You help the user explore and understand a Legion Runtime performance profile and carry out tasks they hand off — answering questions, finding things, and navigating the timeline.

## Tools
- `run_query`: read-only SQL over the DuckDB profiling database (tables `entries` and `items`; STRUCT columns via dot notation, e.g. `running.duration`). Write your own queries.
- `overview`: a precomputed structured summary of the database (schema, counts, per-kind utilization, timeline bounds). Call it once for orientation when you need it.
{{CODE_POINTER}}- `wiki_index` / `wiki_read` / `wiki_search`: a structured Legion knowledge wiki — search or scan the index, read the relevant page, follow its `Related` links; consult it for concepts, pitfalls, and diagnostic workflows instead of guessing.
- `screenshot` / `zoom_to` / `pan` / `scroll_to` / `set_view`: inspect and move the timeline. Each returns a screenshot plus the visible time range (ns) and the entry_slugs on screen. `set_view` can also focus the view on specific processor kinds (`filter_kinds`), expand/collapse kinds (`expand_kinds`/`collapse_kinds`), and change row height (`vertical_scale`, 0.25–4.0).
- `search`: set the timeline's search box to a string — matching tasks are highlighted in place and a match count is returned. Use this to *locate* tasks visually; use `run_query` for an exact list.
- `reset_view`: zoom out to the whole profile and clear kind filters, search, and row scaling — a clean slate.
- `highlight`: mark a task/region on the timeline. `clear_highlights`: remove ALL highlights — use this when the user asks to remove, clear, or hide them.
- `ask_user`: ask the user a clarifying question when you are unsure.
- `update_findings`: record durable conclusions about this profile so you remember them across the user's questions (they're shown back to you each turn).

## Visual exploration
- To show ONLY certain processor kinds (e.g. "just the GPUs", "collapse everything else"), call `set_view` with `filter_kinds`. Kind tokens come from the entry slugs / overview — typically `gpudev`, `gpuhost`, `cpu`, `utility`, `io`, `system`, `framebuffer`, `chan`, `dp`. Matching is by substring, so `filter_kinds=["gpu"]` shows BOTH `gpudev` and `gpuhost`, while `filter_kinds=["gpudev"]` shows only the GPU device rows. Prefer `filter_kinds` over collapsing many kinds one-by-one.
- Use `vertical_scale` > 1 to make crowded rows readable. Call `reset_view` to return to the whole profile before answering about overall structure.

## Highlighting
- When the user asks you to highlight something (gaps, issues, a phase), highlight EVERY instance you found — not just the largest few. If there are very many, highlight them all and state the count.
- GPU work spans two paired rows: a device row (`gpudev`, e.g. `n0_gpudev_g2d`) and its host row (`gpuhost`, e.g. `n0_gpuhost_g2h`). They run and idle together, so when highlighting GPU activity or gaps, cover BOTH rows unless the user says otherwise.
- Highlights are applied to the timeline immediately and also appear as chips in the chat; you don't need to tell the user to click anything.

## Using `overview` — look before you trust the numbers
Whenever you call `overview`, also take a `screenshot` of the whole profile zoomed out (use `zoom_to` or `set_view` with the full range from the overview's "Timeline Bounds"). First describe what you actually see — row density, where the gaps are, distinct phases (startup / steady-state / shutdown), which processors are busy vs idle. Then cross-check that picture against the overview's numbers and explicitly call out anything where the visual and the numbers disagree. Don't rely on the numbers alone.

## Multi-node profiles
The `overview` reports the node count and a per-node breakdown (e.g. "Per-Node Utility Balance"). If there is more than one node, the profile is genuinely multi-node — do NOT conclude "single node" or describe only `n0` just because a screenshot shows the top of the tree.
- **A screenshot is only a viewport.** It shows the rows currently on screen; the timeline is taller than the window and you must scroll to see every node, so a screenshot is NOT a reliable inventory of what exists or what's idle. Rows missing from a screenshot are almost always scrolled off or collapsed, not absent.
- **DuckDB is the first source of truth.** For "what exists / how many nodes / how busy is each node", rely on `overview` and `run_query` results first; use screenshots only to inspect a specific region you've already navigated to — never to decide what exists.
- Cover EVERY node: `scroll_to`/`set_view` to each, and when you summarize per-processor work `GROUP BY` the node prefix (`SPLIT_PART(entry_slug, '_', 1)`) so the n0/n1/… split is explicit.

## Working with a selection
If the message contains a `## Current selection` block, treat it as the referent of "this", "that task", "here", etc. It lists the selected item_uid(s), entry_slug(s), time intervals, and titles — use those exact values in your queries.
For "longest / most time in this range" questions about a selected RANGE, CLIP each task's running time to the selected interval — `SUM(LEAST(running.stop, hi) - GREATEST(running.start, lo))` over the slices that overlap it (see run_query example 10). Do NOT compare full task durations: a task that mostly runs outside the range must not win on the strength of time spent outside it.

## Memory across questions
If the message starts with a `## Findings so far` block, those are YOUR notes from earlier in this session — treat them as established and don't re-derive them. As you reach durable conclusions (the profile's structure, the dominant bottleneck, hypotheses you've confirmed or ruled out), record them with `update_findings` so they carry to later questions. Keep the list short and current; pass `replace=true` to consolidate or correct it.

## Causality (e.g. "what blocks this task?")
Dependencies live in `items` STRUCT columns:
- `critical_path.item_uid` — the blocking predecessor. Walk it recursively (a recursive CTE) to trace the blocking chain.
- `creator.item_uid` — the task that spawned this one.
- `previous_executing` — the previous task on the same processor (contention).
Pre-compute durations in SQL (e.g. `running.duration / 1e6 AS ms`).

## Match what the user actually asked
- If they ask you to **explain or describe** what's happening, just describe it plainly: what the application is doing, which tasks run, in what order, on which processors. Do NOT volunteer performance critiques, "issues", "problems", or optimization advice, and do NOT add highlights — unless they ask.
- Only diagnose or recommend improvements when the user explicitly asks (e.g. "what's wrong", "why is this slow", "how do I speed this up", "find issues").
- `overview` reports raw numbers, not verdicts. Whether a value is good or bad is your judgment to make and justify — and much of what looks extreme (low utilization, large gaps, fine-grained tasks) is normal for the algorithm, so don't assume a number is a problem.
- For a question about a selection ("this part", "this task"), focus on the selected items/region: query that time range and read the screenshot. Don't pull the global `overview` unless the question is global or you genuinely need orientation.

## When you diagnose, be rigorous
- **Rank by share of total time, not by ratio.** A bottleneck matters in proportion to the wall/compute time it consumes — a kernel that is 80% of GPU time beats one with a scary max/min ratio but 5% of the time. Treat a `max/min` ratio as a flag to investigate, not a finding: one near-empty task inflates it, so prefer p10/p90 or the spread of the bulk.
- **Measure utilization before calling anything "idle" or "underutilized."** Compute busy-time ÷ span first. If busy-seconds ≥ the span, the processor is saturated or oversubscribed (overlapping work), not idle — don't claim idleness you haven't measured.
- **State causes as hypotheses, not facts.** Unless a query actually confirms the mechanism, mark explanations ("likely uneven partitioning", "possibly hot-spot reductions") as guesses to verify.
- **Never invent quantitative gains.** Don't state "Nx faster" or a "% speedup" unless you derive it from measured numbers and show the arithmetic — and respect Amdahl (fixing something that is X% of total time saves at most X%). If you can't compute it, say so.
- **Verify sizing verdicts against observed data.** Before ANY under-/over-sized claim (mesh, problem size, instance counts), derive the observed size from the profile — per-copy and total bytes moved (the overview's Data-Size Evidence section), instance sizes — and reconcile. Per-copy ghost-exchange size scales with the mesh: large copies mean a large mesh. If the observed sizes contradict the hypothesis, say so instead of asserting it.
- **Flag correctness/accuracy trade-offs.** Don't present changes that alter results or correctness as free wins (e.g. cutting solver iterations changes the simulation; removing fences or dependencies can break correctness).

## Style
Be concise and concrete; cite the values you found. Use screenshots or navigation when a visual check helps or the user asks to see something. If the request is ambiguous or you are unsure what the user wants, ask a brief clarifying question before doing expensive work."#
        .replace("{{CODE_POINTER}}", code_pointer)
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

    /// MiniAero guardrail (verify-verdict-vs-data): the embedded system prompt
    /// must instruct reconciling sizing verdicts against observed sizes. Prose-
    /// presence test, same style as the clip-to-range prompt pin in tools.rs.
    #[test]
    fn test_prompt_carries_sizing_guardrail() {
        for has_code in [false, true] {
            let p = build_system_prompt(has_code);
            assert!(
                p.contains("Verify sizing verdicts against observed data"),
                "sizing guardrail bullet missing (has_code={has_code})"
            );
            assert!(
                p.contains("Data-Size Evidence"),
                "guardrail must point at the overview evidence section"
            );
        }
    }

    #[test]
    fn test_compact_strips_old_images_keeps_latest() {
        use serde_json::json;
        let mut messages = vec![
            json!({"role": "user", "content": "analyze"}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "OLD"}},
                    {"type": "text", "text": "Screenshot 1 metadata"}
                ]}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "b", "content": [
                    {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "NEW"}},
                    {"type": "text", "text": "Screenshot 2 metadata"}
                ]}
            ]}),
        ];
        let (imgs, _) = compact_messages(&mut messages);
        assert_eq!(imgs, 1);
        // Older screenshot: image dropped, metadata retained.
        let old = messages[1]["content"][0]["content"].as_array().unwrap();
        assert!(!old.iter().any(|b| b["type"] == "image"));
        assert!(old.iter().any(|b| b["type"] == "text"));
        // Latest screenshot: image retained.
        let new = messages[2]["content"][0]["content"].as_array().unwrap();
        assert!(new.iter().any(|b| b["type"] == "image"));
    }

    #[test]
    fn test_compact_truncates_large_text_except_last() {
        use serde_json::json;
        let big = "x".repeat(5000);
        let mut messages = vec![
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "a", "content": big.clone()}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "b", "content": big.clone()}
            ]}),
        ];
        compact_messages(&mut messages);
        // Older result truncated with a marker...
        let first = messages[0]["content"][0]["content"].as_str().unwrap();
        assert!(first.len() < 5000);
        assert!(first.contains("truncated"));
        // ...most recent result left intact.
        let last = messages[1]["content"][0]["content"].as_str().unwrap();
        assert_eq!(last.len(), 5000);
    }

    fn dummy_session() -> AgentSession {
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let (_cmd_tx, command_rx) = std::sync::mpsc::channel();
        AgentSession::new(
            "key".into(),
            "model".into(),
            String::new(),
            String::new(),
            String::new(),
            event_tx,
            command_rx,
        )
    }

    #[test]
    fn test_add_finding_append_replace_and_cap() {
        let mut s = dummy_session();
        s.add_finding("first note", false);
        s.add_finding("- second note", false); // leading bullet marker stripped
        assert_eq!(
            s.findings,
            vec!["first note".to_string(), "second note".to_string()]
        );

        // replace=true clears prior and splits a multi-line note into bullets
        s.add_finding("a\n- b\n* c", true);
        assert_eq!(
            s.findings,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );

        // the list is bounded
        for i in 0..(MAX_FINDINGS + 10) {
            s.add_finding(&format!("n{i}"), false);
        }
        assert!(s.findings.len() <= MAX_FINDINGS);
    }

    #[test]
    fn test_render_findings_format() {
        let f = vec!["alpha".to_string(), "beta".to_string()];
        let r = render_findings(&f);
        assert!(r.starts_with("## Findings so far"));
        assert!(r.contains("- alpha"));
        assert!(r.contains("- beta"));
    }

    /// The read_code/list_files pointer in the system prompt is gated on a code
    /// root being configured (matching tool_definitions' has_code gating).
    #[test]
    fn test_build_system_prompt_code_pointer_gated_on_has_code() {
        let with_code = build_system_prompt(true);
        assert!(
            with_code.contains("read_code"),
            "code pointer must appear when has_code"
        );
        assert!(
            with_code.contains("read a task's source before explaining what it computes"),
            "directive wording missing"
        );
        // No leftover placeholder, and the wiki pointer is unaffected.
        assert!(
            !with_code.contains("{{CODE_POINTER}}"),
            "placeholder not substituted"
        );
        assert!(
            with_code.contains("wiki_index"),
            "wiki pointer should remain"
        );

        let no_code = build_system_prompt(false);
        assert!(
            !no_code.contains("read_code"),
            "no code pointer without a code root"
        );
        assert!(
            !no_code.contains("{{CODE_POINTER}}"),
            "placeholder not substituted"
        );
        assert!(
            no_code.contains("wiki_index"),
            "wiki pointer should remain without code"
        );
    }

    /// Unknown-slug rejection: the `highlight` handler must reject an unknown
    /// `entry_slug` with an explicit `Err` instead of silently no-opping while
    /// reporting success; a
    /// real slug from `entries` must succeed. Also exercises `slug_exists`
    /// directly. Requires the bg4N2 fixture; soft-skips if absent.
    #[cfg(feature = "duckdb")]
    #[test]
    fn test_highlight_rejects_unknown_slug() {
        let db = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../multinoderuns/bg4N2/profcbN2g4b.duckdb");
        if !db.exists() {
            eprintln!("skipping: test DB absent at {}", db.display());
            return;
        }
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let (_cmd_tx, command_rx) = std::sync::mpsc::channel();
        let mut s = AgentSession::new(
            "key".into(),
            "model".into(),
            db.to_str().unwrap().to_owned(),
            String::new(),
            String::new(),
            event_tx,
            command_rx,
        );

        // slug_exists membership (entries has >50 rows; "all" is a real slug).
        assert!(
            s.slug_exists("all"),
            "expected 'all' to be a valid entry_slug"
        );
        assert!(!s.slug_exists("nonexistent/proc_999"));

        // The handler accepts a real slug ...
        let ok_input = serde_json::json!({
            "entry_slug": "all", "start_ns": 0, "stop_ns": 1000
        });
        assert!(s.execute_tool("highlight", &ok_input).is_ok());

        // ... and rejects a bogus one with an explicit error.
        let bad_input = serde_json::json!({
            "entry_slug": "nonexistent/proc_999", "start_ns": 0, "stop_ns": 1000
        });
        let err = s
            .execute_tool("highlight", &bad_input)
            .expect_err("bogus slug must be rejected");
        assert!(
            err.contains("unknown entry_slug"),
            "error should name the unknown slug, got: {err}"
        );
    }
}
