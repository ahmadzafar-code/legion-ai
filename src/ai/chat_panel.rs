//! Chat panel UI for AI-powered performance analysis.
//!
//! Provides a Cursor-inspired toggleable right-side panel where users can:
//! - Ask questions about their profile in a composer input
//! - Add context via the ＋ menu (plain files attach inline; folders set the
//!   project root; `.duckdb` files set the database path)
//! - View progressive analysis results with markdown rendering
//! - Enter an API key in a popup when the built-in API engine is active (the
//!   engine itself is auto-detected: Claude Code when installed, else the API loop)
//!
//! When built with `--features ai`, the panel calls the native Rust agent
//! (`agent::AgentSession`) directly — no Python sidecar required.

use crate::ai::agent::{AgentEvent, AgentResponse, AgentSession, Highlight, UiCommand};
use crate::data::EntryID;
use crate::timestamp::Interval;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

/// Shared event channel type — receives progressive AgentEvents from the agent thread.
type EventChannel = Arc<Mutex<Option<mpsc::Receiver<AgentEvent>>>>;

// ── Public types ────────────────────────────────────────────────────────────

/// The kind of chat message, controlling rendering style.
/// Model the built-in API engine uses. The Claude Code backend deliberately
/// passes no model (the user's own `claude` configuration decides).
const NATIVE_MODEL: &str = "claude-sonnet-4-6";

#[derive(Clone, Debug)]
pub enum ChatMessageKind {
    /// Gray italic — system status messages
    System,
    /// Right-aligned blue bubble — user input
    User,
    /// Left-aligned with markdown rendering — analysis results
    Analysis,
}

/// Which backend serves the embedded chat.
///
/// - `Native`: the hand-rolled in-process agent (`AgentSession` over raw HTTP).
///   Needs an API key. Drives the viewport in-process as `EMBEDDED_CONSUMER_ID`.
/// - `ClaudeCode`: the user's own Claude Code, spawned as a persistent
///   stream-json subprocess connected to the in-viewer MCP server (needs the
///   `viewer-mcp` feature + a running server). No API key — rides on the user's
///   `claude` login. Drives the viewport over the MCP bridge (`MCP_CONSUMER_ID`).
///
/// CHANNEL-LIFETIME CONTRACT (load-bearing): the Native arm
/// keeps the existing PER-TURN `(event_tx, event_rx)` swap in
/// `trigger_diagnosis`. The ClaudeCode arm must create its channels ONCE at
/// child spawn and keep them for the child's lifetime — follow-up turns only
/// write to the existing stdin. Sharing the per-turn swap would disconnect the
/// subprocess reader on turn 2 (`poll_events` would see `Disconnected`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatBackendKind {
    /// Hand-rolled in-process agent (API key).
    Native,
    /// Spawned Claude Code subprocess over the in-viewer MCP server.
    ClaudeCode,
}

/// A single message in the chat panel.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub kind: ChatMessageKind,
    pub text: String,
    /// Highlights attached to this message (only for Analysis messages).
    pub highlights: Vec<Highlight>,
    /// Full tool result content (only for System messages from ToolResult events).
    pub expandable_content: Option<String>,
}

/// A user-initiated highlight action from a chip click.
#[derive(Clone, Debug)]
pub struct HighlightAction {
    pub highlight: Highlight,
    pub zoom_to: bool,
}

/// A pending navigation action from the agent, consumed by core.rs.
///
/// Each variant carries a `request_id` so the UI can send back the screenshot
/// response to the correct agent request.
#[derive(Clone, Debug)]
pub enum PendingNavigation {
    /// Plain screenshot capture.
    Screenshot { request_id: u64 },
    /// Zoom to a time range, then screenshot.
    Zoom { request_id: u64, start_ns: i64, stop_ns: i64 },
    /// Pan left/right by a percentage, then screenshot.
    Pan { request_id: u64, direction: String, percent: f64 },
    /// Scroll vertically to a processor row, then screenshot.
    ScrollTo { request_id: u64, entry_slug: String },
    /// Zoom + optional scroll/filter/expand, then screenshot.
    SetView {
        request_id: u64,
        start_ns: i64,
        stop_ns: i64,
        entry_slug: Option<String>,
        filter_kinds: Option<Vec<String>>,
        expand_kinds: Option<Vec<String>>,
        collapse_kinds: Option<Vec<String>>,
        vertical_scale: Option<f64>,
    },
    /// Set the timeline search query, then screenshot.
    Search { request_id: u64, query: String },
    /// Reset zoom/spacing/filters, then screenshot.
    ResetView { request_id: u64 },
}

/// A user's selection on the timeline (like selected code lines in Cursor).
#[derive(Clone, Debug)]
pub struct TimelineSelection {
    pub entry_id: EntryID,
    /// Human-readable label: "CPU Proc 2" or "n0_cpu_c2"
    pub entry_label: String,
    /// The selected gap/time range
    pub interval: Interval,
}

/// A task (bar) the user selected on the timeline, surfaced to the agent as
/// structured context so it can resolve "this task" to concrete identifiers.
#[derive(Clone, Debug)]
pub struct SelectedItem {
    pub item_uid: u64,
    pub entry_slug: Option<String>,
    pub title: String,
    pub start_ns: i64,
    pub stop_ns: i64,
}

// ── ＋-menu context types ───────────────────────────────────────────────────

/// The kind of context attachment, auto-detected from the filesystem entry.
/// A file added as inline context via the ＋ menu. Folders and .duckdb files
/// are not attachments: ＋ routes them to the project-root / DB-path settings
/// directly (they configure TOOLS; only plain files are injected as text).
#[derive(Clone, Debug)]
pub struct ContextAttachment {
    /// Full absolute path on disk.
    pub path: String,
    /// Display name (last path component, e.g. "circuit.cc").
    pub display_name: String,
}

// ── Tool status ──────────────────────────────────────────────────────────────

/// Auto-derived tool readiness status (no toggles — tools are enabled when
/// their prerequisites are satisfied).
#[derive(Clone, Debug, PartialEq)]
enum ToolStatus {
    /// Prerequisite not configured (e.g. no DB path set).
    Off,
    /// Prerequisite satisfied — tool is available.
    Ready,
    /// Prerequisite set but invalid (e.g. DB path doesn't exist).
    Error(String),
}

// ── ChatPanel ───────────────────────────────────────────────────────────────

/// The chat panel state and UI.
pub struct ChatPanel {
    pub visible: bool,
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub selection: Option<TimelineSelection>,
    /// Task (bar) selection surfaced to the agent as structured context.
    selected_items: Vec<SelectedItem>,
    /// The API-key entry popup (opened from the header warning or automatically
    /// when the API engine is selected without a key).
    api_key_popup_open: bool,
    /// Auto-resolved engine (cached): Claude Code when `claude` is installed,
    /// else the native API loop. There is NO user-facing backend choice — the
    /// user's outside setup (claude login / API key) IS the choice. Re-resolved
    /// on ↺ New session.
    resolved_backend: Option<ChatBackendKind>,

    // ── ＋-menu context ────────────────────────────────────────────────────
    /// Plain-file attachments added via the ＋ menu (inline context).
    attachments: Vec<ContextAttachment>,

    // ── Tools configuration ──────────────────────────────────────────────
    /// DuckDB database path — required for `run_query` tool.
    duckdb_path_buffer: String,
    /// Application code directory — required for `read_code` tool.
    code_path_buffer: String,
    /// Legion wiki root — required for `wiki_index`/`wiki_read`/`wiki_search`.
    /// Pre-filled from `--wiki` / auto-detection at startup (no settings widget).
    wiki_path_buffer: String,

    // ── Agent state ────────────────────────────────────────────────────────
    /// API key (from UI field; falls back to ANTHROPIC_API_KEY env var).
    api_key_buffer: String,
    /// Model name: "claude-sonnet-4-6" or "claude-opus-4-8".
    /// Persistent agent session (holds conversation history for follow-ups).
    agent_session: Arc<Mutex<Option<AgentSession>>>,
    /// Whether an agent request is currently in flight.
    pending_request: bool,
    /// Channel for receiving progressive AgentEvents from the background thread.
    event_rx: EventChannel,
    /// Sender for UiCommand messages back to the agent thread (screenshot data).
    ui_command_tx: Option<mpsc::Sender<UiCommand>>,
    /// Pending navigation action from the agent thread, consumed by core.rs.
    /// Set by ScreenshotRequest/ZoomRequest/PanRequest/etc events.
    pending_navigation: Option<PendingNavigation>,
    /// Pending question from the agent (human-in-the-loop): (request_id, question, options).
    /// Rendered in the composer; answered via `send_user_answer`.
    pending_question: Option<(u64, String, Vec<String>)>,
    /// Request to clear all timeline highlight overlays (from the Clear button or
    /// the agent's `clear_highlights` tool), consumed by core.rs.
    pending_clear_highlights: bool,
    /// Request to clear the current task/region selection (✕ in the composer).
    pending_clear_selection: bool,
    /// User-initiated highlight actions from chip clicks, consumed by core.rs.
    pending_highlight_actions: Vec<HighlightAction>,
    /// Shared viewport-ownership token, handed to each spawned
    /// `AgentSession` so the embedded agent and the in-viewer MCP driver are
    /// mutually exclusive. `None` until `core.rs` wires it from the `Context`.
    viewport_token: Option<crate::ai::bridge::ViewportToken>,
    /// The in-viewer MCP server endpoint — (ACTUAL bound port, per-session bearer
    /// token) — wired by `core.rs` after spawn. `None` = server not running yet
    /// (the Claude Code backend is unavailable). Its `--mcp-config` needs BOTH: the
    /// real port and an `Authorization: Bearer <token>` header (server hardening).
    #[cfg(feature = "viewer-mcp")]
    mcp_endpoint: Option<(u16, String)>,
    /// The persistent Claude Code subprocess. Arc-shared across
    /// panel clones (like `agent_session`); the kill/reap lives in
    /// `ClaudeCodeAgent::Drop`, which runs when the LAST Arc drops — a throwaway
    /// panel clone can never kill the shared child.
    #[cfg(feature = "viewer-mcp")]
    cc_agent: Arc<Mutex<Option<Arc<crate::ai::claude_code::ClaudeCodeAgent>>>>,
    /// The approval broker behind the MCP server's /approve route — the
    /// panel polls it each frame and renders the Deny/Allow/Always-allow dialog
    /// for hook-gated tool calls (Bash/Edit/Write/WebFetch/…). Wired by core.rs
    /// alongside `mcp_endpoint`; Arc-shared with the server thread.
    #[cfg(feature = "viewer-mcp")]
    approval_broker: Option<Arc<crate::ai::claude_code::ApprovalBroker>>,
    /// The LIVE project-root handle shared with the MCP server (created
    /// here, handed to `viewer_mcp::spawn` by core.rs). Synced each frame from
    /// the normalized `code_path_buffer`.
    #[cfg(feature = "viewer-mcp")]
    project_root: crate::ai::mcp_core::SharedCodeRoot,
    /// Last value written to `project_root` (change detection for the sync).
    #[cfg(feature = "viewer-mcp")]
    last_synced_root: Option<String>,
    /// The project root the CURRENT Claude Code child was spawned with
    /// (`--add-dir` is fixed per child) — when it differs from the live value,
    /// the settings row shows "takes effect on ↺ New session".
    #[cfg(feature = "viewer-mcp")]
    cc_spawn_root: Option<String>,
    /// Markdown render cache for analysis messages (egui_commonmark). Arc-shared
    /// across panel clones; `CommonMarkCache` is not `Clone`.
    md_cache: Arc<Mutex<egui_commonmark::CommonMarkCache>>,
}

impl Clone for ChatPanel {
    fn clone(&self) -> Self {
        Self {
            visible: self.visible,
            messages: self.messages.clone(),
            input_buffer: self.input_buffer.clone(),
            selection: self.selection.clone(),
            selected_items: self.selected_items.clone(),
            api_key_popup_open: self.api_key_popup_open,
            resolved_backend: self.resolved_backend,
            attachments: self.attachments.clone(),
            duckdb_path_buffer: self.duckdb_path_buffer.clone(),
            code_path_buffer: self.code_path_buffer.clone(),
            wiki_path_buffer: self.wiki_path_buffer.clone(),
            api_key_buffer: self.api_key_buffer.clone(),
            agent_session: Arc::clone(&self.agent_session),
            pending_request: self.pending_request,
            event_rx: Arc::clone(&self.event_rx),
            ui_command_tx: self.ui_command_tx.clone(),
            pending_navigation: self.pending_navigation.clone(),
            pending_question: self.pending_question.clone(),
            pending_clear_highlights: self.pending_clear_highlights,
            pending_clear_selection: self.pending_clear_selection,
            pending_highlight_actions: self.pending_highlight_actions.clone(),
            viewport_token: self.viewport_token.clone(),
            #[cfg(feature = "viewer-mcp")]
            mcp_endpoint: self.mcp_endpoint.clone(),
            #[cfg(feature = "viewer-mcp")]
            cc_agent: Arc::clone(&self.cc_agent),
            #[cfg(feature = "viewer-mcp")]
            approval_broker: self.approval_broker.clone(),
            #[cfg(feature = "viewer-mcp")]
            project_root: Arc::clone(&self.project_root),
            #[cfg(feature = "viewer-mcp")]
            last_synced_root: self.last_synced_root.clone(),
            #[cfg(feature = "viewer-mcp")]
            cc_spawn_root: self.cc_spawn_root.clone(),
            md_cache: Arc::clone(&self.md_cache),
        }
    }
}

impl std::fmt::Debug for ChatPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatPanel")
            .field("visible", &self.visible)
            .field("messages", &self.messages.len())
            .field("pending_request", &self.pending_request)
            .field("pending_highlight_actions", &self.pending_highlight_actions.len())
            .finish()
    }
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatPanel {
    /// Create a new chat panel. The transcript starts EMPTY — the centered
    /// empty-state screen (headline + suggestion cards) is the welcome.
    pub fn new() -> Self {
        Self {
            visible: false,
            messages: Vec::new(),
            input_buffer: String::new(),
            selection: None,
            selected_items: Vec::new(),
            api_key_popup_open: false,
            resolved_backend: None,
            attachments: Vec::new(),
            duckdb_path_buffer: String::new(),
            code_path_buffer: String::new(),
            wiki_path_buffer: String::new(),
            api_key_buffer: String::new(),
            agent_session: Arc::new(Mutex::new(None)),
            pending_request: false,
            event_rx: Arc::new(Mutex::new(None)),
            ui_command_tx: None,
            pending_navigation: None,
            pending_question: None,
            pending_clear_highlights: false,
            pending_clear_selection: false,
            pending_highlight_actions: Vec::new(),
            viewport_token: None,
            #[cfg(feature = "viewer-mcp")]
            mcp_endpoint: None,
            #[cfg(feature = "viewer-mcp")]
            cc_agent: Arc::new(Mutex::new(None)),
            #[cfg(feature = "viewer-mcp")]
            approval_broker: None,
            #[cfg(feature = "viewer-mcp")]
            project_root: Default::default(),
            #[cfg(feature = "viewer-mcp")]
            last_synced_root: None,
            #[cfg(feature = "viewer-mcp")]
            cc_spawn_root: None,
            md_cache: Arc::new(Mutex::new(egui_commonmark::CommonMarkCache::default())),
        }
    }

    /// Toggle panel visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Wire the shared viewport token. Idempotent; called by `core.rs` once
    /// the `Context` is available so each spawned `AgentSession` claims the same
    /// token the in-viewer MCP driver does.
    pub fn ensure_viewport_token(&mut self, token: crate::ai::bridge::ViewportToken) {
        if self.viewport_token.is_none() {
            self.viewport_token = Some(token);
        }
    }

    /// Pre-fill the tool paths (from CLI flags / auto-detection at startup).
    pub fn set_tool_paths(
        &mut self,
        duckdb: Option<String>,
        code: Option<String>,
        wiki: Option<String>,
    ) {
        if let Some(d) = duckdb {
            if !d.is_empty() {
                self.duckdb_path_buffer = d;
            }
        }
        if let Some(c) = code {
            if !c.is_empty() {
                self.code_path_buffer = c;
            }
        }
        if let Some(w) = wiki {
            if !w.is_empty() {
                self.wiki_path_buffer = w;
            }
        }
    }

    /// Add a message to the chat panel.
    pub fn add_message(&mut self, kind: ChatMessageKind, text: impl Into<String>) {
        self.messages.push(ChatMessage {
            kind,
            text: text.into(),
            highlights: vec![],
            expandable_content: None,
        });
    }

    /// Update the timeline (region) selection. Shown live in the composer pill.
    pub fn set_selection(&mut self, sel: TimelineSelection) {
        self.selection = Some(sel);
    }

    /// Clear the current timeline selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Replace the selected-items set (task bars). Shown live in the composer pill.
    pub fn set_item_selection(&mut self, items: Vec<SelectedItem>) {
        self.selected_items = items;
    }

    /// Clear the task (bar) selection.
    pub fn clear_item_selection(&mut self) {
        self.selected_items.clear();
    }

    /// Render the current task/region selection as a live pill above the input.
    /// Reflects select AND deselect (no stale transcript badges).
    fn ui_selection_pill(&mut self, ui: &mut egui::Ui) {
        if self.selected_items.is_empty() && self.selection.is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Selection:").size(11.0).weak());
            for it in &self.selected_items {
                let name = if it.title.is_empty() {
                    format!("uid {}", it.item_uid)
                } else {
                    it.title.chars().take(28).collect::<String>()
                };
                let chip = match &it.entry_slug {
                    Some(s) => format!("📍 {name} [{s}]"),
                    None => format!("📍 {name}"),
                };
                ui.label(egui::RichText::new(chip).size(11.0));
            }
            if let Some(sel) = &self.selection {
                ui.label(
                    egui::RichText::new(format!(
                        "▭ {} ({})",
                        sel.entry_label,
                        format_duration_ns(sel.interval.duration_ns())
                    ))
                    .size(11.0),
                );
            }
            if ui.small_button("×").on_hover_text("Clear selection").clicked() {
                self.selected_items.clear();
                self.selection = None;
                self.pending_clear_selection = true;
            }
        });
        ui.add_space(4.0);
    }

    /// Build a structured `## Current selection` preamble for the agent message,
    /// or an empty string if nothing is selected.
    fn build_selection_preamble(&self) -> String {
        if self.selected_items.is_empty() && self.selection.is_none() {
            return String::new();
        }
        let mut s = String::from("## Current selection\n");
        for it in &self.selected_items {
            s.push_str(&format!(
                "- item_uid={} title={:?} entry_slug={} interval=[{}, {}]\n",
                it.item_uid,
                it.title,
                it.entry_slug.as_deref().unwrap_or("?"),
                it.start_ns,
                it.stop_ns,
            ));
        }
        if let Some(sel) = &self.selection {
            s.push_str(&format!(
                "- region entry={} interval=[{}, {}]\n",
                sel.entry_label, sel.interval.start.0, sel.interval.stop.0,
            ));
        }
        s.push_str(
            "(The user is asking about the selection above. Resolve \"this\"/\"that task\" \
             to these item_uid(s)/entry_slug(s).)\n\n",
        );
        s
    }

    /// Structured snapshot of the current selection for the in-viewer MCP
    /// `get_selection` tool. Reads the SAME state the embedded
    /// `build_selection_preamble` reads (selected task bars + dragged region), so
    /// the MCP read and the embedded preamble report identical information. Returns
    /// `(items, range)`; `range` is `(entry_label, start_ns, stop_ns)`. Both
    /// empty/None ⇒ nothing selected.
    pub fn selection_snapshot(
        &self,
    ) -> (Vec<crate::ai::SelectedItemInfo>, Option<(String, i64, i64)>) {
        let items = self
            .selected_items
            .iter()
            .map(|it| crate::ai::SelectedItemInfo {
                item_uid: it.item_uid,
                entry_slug: it.entry_slug.clone(),
                title: it.title.clone(),
                start_ns: it.start_ns,
                stop_ns: it.stop_ns,
            })
            .collect();
        let range = self
            .selection
            .as_ref()
            .map(|s| (s.entry_label.clone(), s.interval.start.0, s.interval.stop.0));
        (items, range)
    }

    /// Take pending highlight actions (user clicked a chip) for core.rs to resolve.
    pub fn take_pending_highlight_actions(&mut self) -> Vec<HighlightAction> {
        std::mem::take(&mut self.pending_highlight_actions)
    }

    /// Take the pending "clear all highlights" request, if any.
    pub fn take_clear_highlights(&mut self) -> bool {
        std::mem::take(&mut self.pending_clear_highlights)
    }

    /// Take the pending "clear selection" request (✕ in the composer), if any.
    pub fn take_clear_selection(&mut self) -> bool {
        std::mem::take(&mut self.pending_clear_selection)
    }

    /// Take the pending navigation action, if any.
    ///
    /// Called once per frame from `ProfApp::update()` in core.rs.
    /// Core.rs applies the navigation action, captures a screenshot, and sends
    /// the result back to the agent thread.
    pub fn take_pending_navigation(&mut self) -> Option<PendingNavigation> {
        self.pending_navigation.take()
    }

    /// Send screenshot data back to the agent thread.
    ///
    /// Called by core.rs after capturing the screenshot PNG bytes.
    pub fn send_screenshot(&self, request_id: u64, png_bytes: Vec<u8>, metadata: String) {
        if let Some(tx) = &self.ui_command_tx {
            let _ = tx.send(UiCommand::ScreenshotData {
                request_id,
                png_bytes,
                metadata,
            });
        }
    }

    /// Send the user's answer to a pending `ask_user` question back to the agent.
    fn send_user_answer(&self, request_id: u64, answer: String) {
        if let Some(tx) = &self.ui_command_tx {
            let _ = tx.send(UiCommand::UserAnswer { request_id, answer });
        }
    }

    /// Submit composer text: answer a pending `ask_user` question if one is open,
    /// otherwise start a new agent request.
    fn submit_input(&mut self, text: String) {
        if let Some((request_id, _, _)) = self.pending_question.take() {
            self.send_user_answer(request_id, text.clone());
            self.add_message(ChatMessageKind::User, text);
        } else {
            self.trigger_diagnosis(text);
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    /// Get the API key: UI field first, then ANTHROPIC_API_KEY env var.
    /// Trims whitespace/newlines that can sneak in via paste.
    fn get_api_key(&self) -> Option<String> {
        let trimmed = self.api_key_buffer.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
        std::env::var("ANTHROPIC_API_KEY").ok().map(|s| s.trim().to_owned())
    }

    // ── Tool status helpers ────────────────────────────────────────────────

    /// The configured DuckDB path (trimmed, non-empty), if any — the database the
    /// in-viewer MCP server serves `run_query`/`overview`/`find_blockers`
    /// against.
    #[cfg(feature = "viewer-mcp")]
    pub fn duckdb_path(&self) -> Option<String> {
        let trimmed = self.duckdb_path_buffer.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    /// The configured wiki root, if any — handed to the in-viewer MCP server so it
    /// advertises + routes the `wiki_*` tools (mirrors [`Self::duckdb_path`]).
    #[cfg(feature = "viewer-mcp")]
    pub fn wiki_path(&self) -> Option<String> {
        let trimmed = self.wiki_path_buffer.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    /// The EFFECTIVE project root, if any (file paths normalize to their
    /// parent directory) — used for the Claude Code child's `--add-dir` and the
    /// native agent's per-turn refresh.
    #[cfg(feature = "viewer-mcp")]
    pub fn code_path(&self) -> Option<String> {
        effective_project_root(&self.code_path_buffer)
    }

    /// The LIVE project-root handle shared with the in-viewer MCP server:
    /// the server reads it per request, so a folder set in the panel at ANY time
    /// reaches read_code/list_files/instructions (never a snapshot at spawn).
    /// The panel keeps it in sync with the (normalized) path buffer each frame.
    #[cfg(feature = "viewer-mcp")]
    pub fn project_root_handle(&self) -> crate::ai::mcp_core::SharedCodeRoot {
        Arc::clone(&self.project_root)
    }

    /// Push the effective project root into the shared handle when it changed
    /// (called once per frame; one fs stat per frame — same cost class as the
    /// status chips, which already stat per frame).
    #[cfg(feature = "viewer-mcp")]
    fn sync_project_root(&mut self) {
        let effective = effective_project_root(&self.code_path_buffer);
        if effective != self.last_synced_root {
            *self.project_root.write().unwrap() = effective.clone();
            self.last_synced_root = effective;
        }
    }

    /// Wire the in-viewer MCP server endpoint — (ACTUAL bound port, per-session
    /// bearer token) — called by `core.rs` right after the server spawns; `None`
    /// = bind failed. The ClaudeCode backend refuses to start until this is
    /// `Some`, and builds its `--mcp-config` (URL + `Authorization: Bearer`
    /// header) from exactly this pair.
    #[cfg(feature = "viewer-mcp")]
    pub fn set_mcp_endpoint(&mut self, endpoint: Option<(u16, String)>) {
        self.mcp_endpoint = endpoint;
    }

    /// Wire the approval broker — called by `core.rs` alongside
    /// [`Self::set_mcp_endpoint`]. The panel polls it each frame and renders the
    /// Deny/Allow/Always-allow dialog for the child's hook-gated tool calls.
    #[cfg(feature = "viewer-mcp")]
    pub fn set_approval_broker(
        &mut self,
        broker: Option<Arc<crate::ai::claude_code::ApprovalBroker>>,
    ) {
        self.approval_broker = broker;
    }

    /// Persistence: snapshot the user's AI settings for eframe storage
    /// (called by `ProfApp::save`). The API key is deliberately EXCLUDED —
    /// eframe storage is plaintext on disk; use ANTHROPIC_API_KEY instead.
    pub fn export_persisted(&self) -> crate::app::PersistedAiSettings {
        crate::app::PersistedAiSettings {
            project_root: self.code_path_buffer.trim().to_owned(),
            duckdb_path: self.duckdb_path_buffer.trim().to_owned(),
            wiki_path: self.wiki_path_buffer.trim().to_owned(),
        }
    }

    /// Persistence: restore a previous session's AI settings (called by
    /// `ProfApp::new` BEFORE `set_tool_paths`, so explicit CLI flags win).
    pub fn apply_persisted(&mut self, saved: &crate::app::PersistedAiSettings) {
        if !saved.project_root.is_empty() {
            self.code_path_buffer = saved.project_root.clone();
        }
        if !saved.duckdb_path.is_empty() {
            self.duckdb_path_buffer = saved.duckdb_path.clone();
        }
        if !saved.wiki_path.is_empty() {
            self.wiki_path_buffer = saved.wiki_path.clone();
        }
    }

    /// End-of-turn receiver cleanup — CHANNEL-LIFETIME critical (see
    /// [`ChatBackendKind`]): Native uses per-turn channels, so dropping the
    /// receiver after Complete/Error is correct. The Claude Code backend's channel
    /// is once-at-spawn and must OUTLIVE turns — dropping it would orphan the
    /// persistent child's event stream (turn 2 would never render). Keep the
    /// receiver installed while a ClaudeCode child is alive.
    fn end_of_turn_channel_cleanup(&mut self) {
        #[cfg(feature = "viewer-mcp")]
        if self.cc_agent.lock().unwrap().is_some() {
            return;
        }
        *self.event_rx.lock().unwrap() = None;
    }

    /// Derive the DB tool status from `duckdb_path_buffer`.
    ///
    /// Accepts any existing non-directory file. DuckDB files may have various
    /// naming conventions (`.duckdb`, `_duckdb`, etc.). The DuckDB crate will
    /// produce a clear error if it can't open the file.
    fn tool_status_db(&self) -> ToolStatus {
        let trimmed = self.duckdb_path_buffer.trim();
        if trimmed.is_empty() {
            return ToolStatus::Off;
        }
        let path = std::path::Path::new(trimmed);
        if !path.exists() {
            return ToolStatus::Error("File not found".into());
        }
        if path.is_dir() {
            return ToolStatus::Error("Path is a directory, not a file".into());
        }
        ToolStatus::Ready
    }

    /// Derive the Code tool status from the EFFECTIVE project root: green
    /// only when the effective root is a real directory — the value every
    /// consumer actually uses, so the chip never shows Ready for a
    /// configuration that would fail (a FILE path is accepted because
    /// [`effective_project_root`] really does normalize it to its parent
    /// directory).
    fn tool_status_code(&self) -> ToolStatus {
        if self.code_path_buffer.trim().is_empty() {
            return ToolStatus::Off;
        }
        match effective_project_root(&self.code_path_buffer) {
            Some(root) if std::path::Path::new(&root).is_dir() => ToolStatus::Ready,
            _ => ToolStatus::Error("Folder not found".into()),
        }
    }

    /// Visual tool status — always Ready (screenshot/zoom are built-in).
    fn tool_status_visual(&self) -> ToolStatus {
        ToolStatus::Ready
    }

    /// Poll for progressive agent events (non-blocking).
    ///
    /// Two-phase approach: drain all available events under the lock into a
    /// local Vec, then process them after dropping the lock (avoids borrow
    /// conflict between the Mutex guard and `&mut self`).
    fn poll_events(&mut self) {
        // Phase 1: Drain all available events under the lock
        let (events, disconnected) = {
            let guard = self.event_rx.lock().unwrap();
            if let Some(rx) = guard.as_ref() {
                let mut events = Vec::new();
                let mut disconnected = false;
                loop {
                    match rx.try_recv() {
                        Ok(event) => events.push(event),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
                (events, disconnected)
            } else {
                return;
            }
            // guard dropped here
        };

        // Phase 2: Process events (no lock held) through the SHARED bridge handler
        // (`apply_agent_event`), so the embedded agent and any future consumer run
        // the identical AgentEvent→UI logic. The embedded screenshot reply is
        // delivered by core.rs via `ui_command_tx`, so `reply_tx` is unused by this
        // sink; a dummy channel only stands in if no agent channel is wired yet.
        let (dummy_tx, _dummy_rx) = mpsc::channel::<UiCommand>();
        let reply_tx = self.ui_command_tx.clone().unwrap_or(dummy_tx);
        for event in events {
            crate::ai::bridge::apply_agent_event(self, event, &reply_tx);
        }

        // Handle disconnected channel (agent thread crashed without sending Complete/Error)
        if disconnected && self.pending_request {
            self.add_message(
                ChatMessageKind::System,
                "Agent thread disconnected unexpectedly.",
            );
            self.pending_request = false;
            *self.event_rx.lock().unwrap() = None;
        }
    }

    /// Trigger an agent request on the SELECTED backend.
    ///
    /// `Native` runs the in-process `AgentSession` (needs an API key);
    /// `ClaudeCode` drives the user's own Claude Code over the in-viewer MCP
    /// server (no key). Exactly one backend runs a request at a time —
    /// `pending_request` is the shared guard.
    /// Inline context from ＋-menu file attachments, rendered as fenced blocks
    /// (folders/.duckdb never become attachments — they configure the project
    /// root / DB path instead). Capped per file and in total so a large attach
    /// cannot blow the request. Used by BOTH engines: the native agent prepends
    /// it to the request, the Claude Code backend appends it to the turn text.
    fn build_attachment_context(&self) -> String {
        let mut context_section = String::new();
        let mut total_context_bytes: usize = 0;
        let max_total_context: usize = 80_000;
        let max_per_file: usize = 16_000;

        for att in &self.attachments {
            if total_context_bytes >= max_total_context {
                break;
            }
            match std::fs::read_to_string(&att.path) {
                Ok(content) => {
                    let truncated = if content.len() > max_per_file {
                        format!(
                            "{}…\n(truncated at {} bytes)",
                            &content[..max_per_file],
                            max_per_file
                        )
                    } else {
                        content
                    };
                    context_section.push_str(&format!(
                        "## Attached file: {}\n```\n{}\n```\n\n",
                        att.display_name, truncated
                    ));
                    total_context_bytes += truncated.len();
                }
                Err(e) => {
                    context_section.push_str(&format!(
                        "## Attached file: {} (could not read: {})\n\n",
                        att.display_name, e
                    ));
                }
            }
        }
        context_section
    }

    /// Resolve which engine serves this session (cached until ↺): the user's
    /// own Claude Code when installed — their login/API key/model choices all
    /// live there — else the built-in API loop (key from popup or env).
    fn resolve_backend(&mut self) -> ChatBackendKind {
        if let Some(b) = self.resolved_backend {
            return b;
        }
        #[cfg(feature = "viewer-mcp")]
        let b = if crate::ai::claude_code::preflight_claude().is_ok() {
            ChatBackendKind::ClaudeCode
        } else {
            ChatBackendKind::Native
        };
        #[cfg(not(feature = "viewer-mcp"))]
        let b = ChatBackendKind::Native;
        self.resolved_backend = Some(b);
        b
    }

    fn trigger_diagnosis(&mut self, user_query: String) {
        if self.pending_request {
            self.add_message(
                ChatMessageKind::System,
                "A request is already in progress. Please wait.",
            );
            return;
        }
        match self.resolve_backend() {
            ChatBackendKind::Native => self.trigger_native(user_query),
            ChatBackendKind::ClaudeCode => self.trigger_claude_code(user_query),
        }
    }

    /// The Claude Code backend: the user's own Claude Code as a persistent
    /// stream-json subprocess over the in-viewer MCP server. Spawned lazily on
    /// the first turn; follow-up turns write to the SAME live stdin (verified
    /// empirically against claude 2.1.x).
    ///
    /// CHANNEL-LIFETIME CONTRACT (vs. Native's per-turn swap): the
    /// `(event_tx, event_rx)` pair is created ONCE at spawn, `event_rx` stays
    /// installed for the child's lifetime, and the parser events flow through
    /// the same `poll_events`/`apply_agent_event` path Native uses.
    fn trigger_claude_code(&mut self, user_query: String) {
        // Echo the user's turn — the composer clears the buffer before submit, so
        // without this the typed question would silently vanish from the transcript.
        self.add_message(ChatMessageKind::User, &user_query);
        #[cfg(feature = "viewer-mcp")]
        {
            let Some((port, token)) = self.mcp_endpoint.clone() else {
                self.add_message(
                    ChatMessageKind::System,
                    "⚠ Claude Code backend needs the in-viewer MCP server. Load a \
                     profile (connect the profile DuckDB via the + menu) so the server \
                     starts, then try again. (If the server failed to bind, the \
                     terminal log has the reason — a restart is needed in that case.)",
                );
                return;
            };

            // Lazy once-per-session spawn (once-at-spawn channels), with a
            // preflight so "claude isn't installed" is a friendly message, not a
            // spawn error. Auth is deliberately NOT probed (it would cost a model
            // call) — a missing `claude login` surfaces on the first turn as the
            // parser's actionable 401 message.
            if self.cc_agent.lock().unwrap().is_none() {
                let version = match crate::ai::claude_code::preflight_claude() {
                    Ok(v) => v,
                    Err(e) => {
                        self.add_message(ChatMessageKind::System, format!("⚠ {e}"));
                        return;
                    }
                };
                let (event_tx, event_rx) = mpsc::channel::<AgentEvent>();
                // Grant the harness's own Read/Glob/Grep access to the profiled app's
                // source (via --add-dir), so the Claude Code backend can read code
                // with the full harness rather than only the MCP read_code tool.
                let code_root = self.code_path();
                // No --model: the child uses the user's own Claude Code default
                // (their install, their model choice).
                match crate::ai::claude_code::ClaudeCodeAgent::spawn(
                    port,
                    &token,
                    "",
                    code_root.as_deref(),
                    event_tx,
                ) {
                    Ok(agent) => {
                        *self.event_rx.lock().unwrap() = Some(event_rx);
                        *self.cc_agent.lock().unwrap() = Some(agent);
                        // --add-dir is fixed per child: remember what this one got
                        // so the settings row can flag a later change.
                        self.cc_spawn_root = code_root.clone();
                        self.add_message(
                            ChatMessageKind::System,
                            format!(
                                "Started your Claude Code ({version}) against the \
                                 profiler's MCP server (port {port}, bearer-token \
                                 protected). One-time setup if the first turn fails \
                                 to authenticate: run `claude login` in a terminal."
                            ),
                        );
                    }
                    Err(e) => {
                        self.add_message(ChatMessageKind::System, format!("⚠ {e}"));
                        return;
                    }
                }
            }

            // Attachment context + timeline-selection preamble travel WITH the
            // turn text — without this, chips render but their content silently
            // never reaches the model on this engine.
            let mut turn_text = String::new();
            let preamble = self.build_selection_preamble();
            if !preamble.is_empty() {
                turn_text.push_str(&preamble);
                turn_text.push('\n');
            }
            let attachment_context = self.build_attachment_context();
            if !attachment_context.is_empty() {
                turn_text.push_str(&attachment_context);
            }
            turn_text.push_str(&user_query);

            let send_result = self
                .cc_agent
                .lock()
                .unwrap()
                .as_ref()
                .map(|agent| agent.send_turn(&turn_text));
            match send_result {
                Some(Ok(())) => {
                    self.add_message(
                        ChatMessageKind::System,
                        "Working… (your Claude Code is driving the profiler over MCP)",
                    );
                    self.pending_request = true;
                }
                Some(Err(e)) => {
                    self.add_message(ChatMessageKind::System, format!("⚠ {e}"));
                    // The child is unusable (dead stdin) — drop it so the next
                    // turn re-spawns fresh.
                    *self.cc_agent.lock().unwrap() = None;
                }
                None => {}
            }
        }
        // Without viewer-mcp, resolve_backend() always yields Native, so this
        // path is unreachable in those builds.
        #[cfg(not(feature = "viewer-mcp"))]
        unreachable!("ClaudeCode backend dispatched without the viewer-mcp feature");
    }

    /// The native backend: the in-process agent (`AgentSession` over raw HTTP).
    /// Runs in a background thread; the session persists across follow-ups.
    /// NOTE: the per-turn `(event_tx, event_rx)` swap below is Native-ONLY —
    /// see [`ChatBackendKind`] for the channel-lifetime contract.
    fn trigger_native(&mut self, user_query: String) {
        let Some(api_key) = self.get_api_key() else {
            self.add_message(
                ChatMessageKind::System,
                "⚠ API key not set. Enter one (or set ANTHROPIC_API_KEY).",
            );
            self.api_key_popup_open = true;
            return;
        };

        // Tool paths from dedicated fields. The project root is the EFFECTIVE
        // value (a file normalizes to its parent directory).
        let duckdb_path = self.duckdb_path_buffer.trim().to_owned();
        let code_path = effective_project_root(&self.code_path_buffer).unwrap_or_default();
        let wiki_path = self.wiki_path_buffer.trim().to_owned();

        let context_section = self.build_attachment_context();

        self.add_message(ChatMessageKind::User, &user_query);
        let time_hint = "typically 30–90 s";
        self.add_message(
            ChatMessageKind::System,
            format!("Working… ({time_hint})"),
        );
        self.pending_request = true;

        let model = NATIVE_MODEL.to_owned();
        let session_arc = Arc::clone(&self.agent_session);
        let selection_preamble = self.build_selection_preamble();
        let query_clone = user_query;
        let viewport_token = self.viewport_token.clone();

        // Create bidirectional channels for this request
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<UiCommand>();
        *self.event_rx.lock().unwrap() = Some(event_rx);
        self.ui_command_tx = Some(cmd_tx);

        std::thread::spawn(move || {
            // Take the session out (or create a fresh one) — lock released immediately
            let existing = {
                let mut guard = session_arc.lock().unwrap();
                guard.take()
                // guard drops here
            };

            let mut session = match existing {
                Some(mut s) => {
                    // Reused session: update channels (old ones are disconnected)
                    // and re-read the project root (the user can set/change it
                    // mid-conversation).
                    s.update_channels(event_tx.clone(), cmd_rx);
                    s.refresh_code_path(&code_path);
                    s
                }
                None => AgentSession::new(
                    api_key,
                    model,
                    duckdb_path,
                    code_path,
                    wiki_path,
                    event_tx.clone(),
                    cmd_rx,
                ),
            };

            // Claim the shared viewport for each screenshot/nav round-trip so
            // the embedded agent and the in-viewer MCP driver never have two
            // screenshots in flight at once. No token wired => unchanged sole driver.
            if let Some(token) = viewport_token {
                session.set_viewport(token, crate::ai::bridge::EMBEDDED_CONSUMER_ID);
            }

            // Run the agent — prepend selection context + file attachments to the question.
            let enriched = format!("{selection_preamble}{context_section}{query_clone}");
            let result = session.ask(&enriched);

            // Put the session back (with updated conversation history)
            {
                let mut guard = session_arc.lock().unwrap();
                *guard = Some(session);
            }

            // Send completion/error event to the UI thread
            match result {
                Ok(response) => {
                    let _ = event_tx.send(AgentEvent::Complete(response));
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::Error(e));
                }
            }
        });
    }

    // ── Zone methods ────────────────────────────────────────────────────────

    /// Zone 1: Header bar with title, tool status chips, and action buttons.
    fn ui_header(&mut self, ui: &mut egui::Ui) {
        self.resolve_backend();
        ui.horizontal(|ui| {
            // Right-aligned controls (no title — it's redundant with the toolbar toggle)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // New session button — the guaranteed cancel.
                // ↺ is the guaranteed cancel for the Claude Code backend, so it
                // stays clickable MID-turn while a Claude Code child exists
                // (hard_stop kills the child; the reader sees EOF). Native keeps
                // disabled-while-pending behavior (its thread can't be stopped).
                #[cfg(feature = "viewer-mcp")]
                let can_reset =
                    !self.pending_request || self.cc_agent.lock().unwrap().is_some();
                #[cfg(not(feature = "viewer-mcp"))]
                let can_reset = !self.pending_request;
                if ui
                    .add_enabled(can_reset, egui::Button::new("↺"))
                    .on_hover_text("New session (force-stops a running Claude Code turn)")
                    .clicked()
                {
                    *self.agent_session.lock().unwrap() = None;
                    // Hard-stop + drop the persistent Claude Code child
                    // (Drop reaps + joins). Its once-at-spawn receiver dies with it.
                    #[cfg(feature = "viewer-mcp")]
                    if let Some(agent) = self.cc_agent.lock().unwrap().take() {
                        agent.hard_stop();
                        *self.event_rx.lock().unwrap() = None;
                        self.pending_request = false;
                    }
                    // Deny any in-flight approval and clear the session's
                    // always-allow rules — they must not outlive the child.
                    #[cfg(feature = "viewer-mcp")]
                    if let Some(broker) = &self.approval_broker {
                        broker.reset();
                    }
                    // Back to the welcome screen (empty transcript = empty state)
                    // and re-detect the engine (claude may have been (un)installed).
                    self.messages.clear();
                    self.resolved_backend = None;
                }
            });
        });

        // Tool status chips row
        let db_status = self.tool_status_db();
        let code_status = self.tool_status_code();
        let visual_status = self.tool_status_visual();
        let has_key = self.get_api_key().is_some();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            // DB chip
            let (db_label, db_color, db_hover) = match &db_status {
                ToolStatus::Ready => (
                    "DB •",
                    egui::Color32::from_rgb(34, 139, 34),
                    "Database connected".to_string(),
                ),
                ToolStatus::Error(msg) => (
                    "DB ×",
                    egui::Color32::from_rgb(220, 60, 60),
                    format!("Error: {msg}"),
                ),
                ToolStatus::Off => (
                    "DB ○",
                    egui::Color32::from_rgb(160, 160, 160),
                    "Add a DuckDB via the ＋ menu".to_string(),
                ),
            };
            ui.label(egui::RichText::new(db_label).size(13.5).color(db_color))
                .on_hover_text(&db_hover);

            // Code chip
            let (code_label, code_color, code_hover) = match &code_status {
                ToolStatus::Ready => (
                    "Code •",
                    egui::Color32::from_rgb(34, 139, 34),
                    "Code root configured".to_string(),
                ),
                ToolStatus::Error(msg) => (
                    "Code ×",
                    egui::Color32::from_rgb(220, 60, 60),
                    format!("Error: {msg}"),
                ),
                ToolStatus::Off => (
                    "Code ○",
                    egui::Color32::from_rgb(160, 160, 160),
                    "Optional — add a code repo via the ＋ menu".to_string(),
                ),
            };
            ui.label(egui::RichText::new(code_label).size(13.5).color(code_color))
                .on_hover_text(&code_hover);

            // Visual chip
            let (vis_label, vis_color) = match &visual_status {
                ToolStatus::Ready => ("Visual •", egui::Color32::from_rgb(34, 139, 34)),
                _ => ("Visual ○", egui::Color32::from_rgb(160, 160, 160)),
            };
            ui.label(egui::RichText::new(vis_label).size(13.5).color(vis_color))
                .on_hover_text("Screenshot + zoom always available");

            // Engine + API key status. The Claude Code backend needs no key, so
            // the key warning only nags on the API engine.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let engine = self.resolved_backend.unwrap_or(ChatBackendKind::Native);
                let engine_label = match engine {
                    ChatBackendKind::ClaudeCode => "Claude Code",
                    ChatBackendKind::Native => "API",
                };
                if has_key || engine == ChatBackendKind::ClaudeCode {
                    ui.label(
                        egui::RichText::new(engine_label)
                            .small()
                            .color(egui::Color32::from_rgb(100, 100, 100)),
                    );
                } else {
                    let key_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("⚠ API key")
                                .small()
                                .color(egui::Color32::from_rgb(200, 120, 20)),
                        )
                        .frame(false),
                    );
                    if key_btn.clicked() {
                        self.api_key_popup_open = true;
                    }
                }
            });
        });
    }

    /// The API-key entry popup (opened from the backend pill or the header's
    /// ⚠ API key warning). Used by the API backend only; ANTHROPIC_API_KEY in
    /// the environment works without it. Never persisted to disk.
    fn ui_api_key_popup(&mut self, ctx: &egui::Context) {
        if !self.api_key_popup_open {
            return;
        }
        let mut open = true;
        egui::Window::new("API key")
            .id(egui::Id::new("api_key_popup"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color =
                    Some(egui::Color32::from_rgb(30, 30, 30));
                ui.set_min_width(320.0);
                ui.label(
                    egui::RichText::new(
                        "Used by the API backend. Alternatively set ANTHROPIC_API_KEY \
                         in the environment. Kept in memory only — never written to disk.",
                    )
                    .size(12.0)
                    .color(egui::Color32::from_rgb(110, 110, 110)),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.api_key_buffer)
                        .password(true)
                        .hint_text("sk-ant-…")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                if ui.button("Done").clicked() {
                    self.api_key_popup_open = false;
                }
            });
        if !open {
            self.api_key_popup_open = false;
        }
    }

    /// The empty-state screen (Claude-Code-style): centered headline + two
    /// suggestion cards. Shown instead of the transcript until the first
    /// message exists; clicking a card submits its prompt.
    fn ui_empty_state(&mut self, ui: &mut egui::Ui) {
        let mut submit: Option<&str> = None;
        // Push the headline into the upper-middle of the free space.
        ui.add_space((ui.available_height() * 0.28).max(24.0));
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("How can I help?")
                    .size(26.0)
                    .color(egui::Color32::from_rgb(40, 40, 40)),
            );
        });
        ui.add_space(28.0);

        let card = |ui: &mut egui::Ui, title: &str| -> bool {
            let resp = egui::Frame::none()
                .fill(egui::Color32::WHITE)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(225, 225, 225)))
                .rounding(12.0)
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui: &mut egui::Ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(64.0);
                    ui.label(
                        egui::RichText::new(title)
                            .size(14.5)
                            .strong()
                            .color(egui::Color32::from_rgb(40, 40, 40)),
                    );
                })
                .response;
            let resp = resp.interact(egui::Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            resp.clicked()
        };

        // Two side-by-side cards. `ui.columns` does the equal-width split and
        // spacing math itself — the previous manual width math overflowed the
        // panel edge and cut card 2 off.
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(8.0, 0.0))
            .show(ui, |ui: &mut egui::Ui| {
                ui.columns(2, |cols| {
                    if card(&mut cols[0], "Give me an overview of this profile") {
                        submit = Some(
                            "Give me an overview of this profile — what ran, where the \
                             time went, and anything unusual.",
                        );
                    }
                    if card(
                        &mut cols[1],
                        "Highlight idle gaps and find what's preventing them from \
                         starting earlier",
                    ) {
                        submit = Some(
                            "Highlight the largest idle gaps on the timeline and find \
                             what's preventing that work from starting earlier.",
                        );
                    }
                });
            });

        if let Some(prompt) = submit {
            self.submit_input(prompt.to_owned());
        }
    }

    /// Zone 3: Message transcript scroll area.
    fn ui_transcript(&mut self, ui: &mut egui::Ui) {
        // Empty session -> the welcome screen, not an empty scroll area.
        if self.messages.is_empty() && !self.pending_request {
            self.ui_empty_state(ui);
            return;
        }
        // Spinner while waiting
        if self.pending_request {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Analyzing…")
                        .color(egui::Color32::from_rgb(100, 100, 100)),
                );
            });
            ui.add_space(4.0);
        }

        // Copy transcript button
        if !self.messages.is_empty() && !self.pending_request {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Copy transcript")
                        .on_hover_text("Copy full conversation including tool results")
                        .clicked()
                    {
                        let transcript: String = self
                            .messages
                            .iter()
                            .map(|m| {
                                let prefix = match &m.kind {
                                    ChatMessageKind::System => "[system]",
                                    ChatMessageKind::User => "[user]",
                                    ChatMessageKind::Analysis => "[analysis]",
                                };
                                let base = format!("{} {}", prefix, m.text);
                                if let Some(ref content) = m.expandable_content {
                                    format!("{}\n{}\n", base, content)
                                } else {
                                    format!("{}\n", base)
                                }
                            })
                            .collect();
                        ui.output_mut(|o| o.copied_text = transcript);
                    }
                });
            });
            ui.add_space(2.0);
        }

        // Message scroll area — fills the remaining (central-panel) space.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Fixed chat text size (~16.5px body on the panel's 1.2 base) —
                // the Claude-app-standard reading size; no user slider.
                for font_id in ui.style_mut().text_styles.values_mut() {
                    font_id.size *= 1.1;
                }
                // Swap messages out to avoid borrowing self.messages
                // immutably while self.pending_highlight_actions is
                // borrowed mutably by render_message().
                let messages = std::mem::take(&mut self.messages);
                let mut md_cache = self.md_cache.lock().unwrap();
                for msg in &messages {
                    render_message(ui, msg, &mut self.pending_highlight_actions, &mut md_cache);
                    ui.add_space(4.0);
                }
                drop(md_cache);
                self.messages = messages;
            });
    }

    /// Context chips above the composer input (Claude-Desktop style): the
    /// active DuckDB, the project repo, and any attached files — each with an ×.
    /// The DB/repo chips mirror the SETTINGS buffers (however they were set —
    /// the "+" menu, a CLI flag, or persistence), so what the agent can touch is
    /// always visible right where you type; × genuinely unconfigures the tool.
    /// Text-only (no glyph icons — emojis are tofu in egui's default fonts); the
    /// chip BACKGROUND color carries the kind (blue=DB, green=repo, gray=file).
    fn ui_context_chips(&mut self, ui: &mut egui::Ui) {
        let db_set = !self.duckdb_path_buffer.trim().is_empty();
        let repo = effective_project_root(&self.code_path_buffer);
        if !db_set && repo.is_none() && self.attachments.is_empty() {
            return;
        }

        let chip = |ui: &mut egui::Ui, name: &str, hover: &str, bg: egui::Color32| -> bool {
            let mut remove = false;
            egui::Frame::none()
                .fill(bg)
                .rounding(12.0)
                .inner_margin(egui::Margin::symmetric(9.0, 4.0))
                .show(ui, |ui: &mut egui::Ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.label(
                            egui::RichText::new(name)
                                .size(14.5)
                                .color(egui::Color32::from_rgb(30, 30, 30)),
                        )
                        .on_hover_text(hover);
                        if ui
                            .small_button(egui::RichText::new("×").size(14.5))
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            remove = true;
                        }
                    });
                });
            remove
        };

        ui.horizontal_wrapped(|ui| {
            if db_set {
                let path = self.duckdb_path_buffer.trim().to_owned();
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                if chip(ui, &name, &path, egui::Color32::from_rgb(219, 234, 254)) {
                    self.duckdb_path_buffer.clear();
                }
            }
            if let Some(root) = repo {
                let name = std::path::Path::new(&root)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.clone());
                #[cfg(feature = "viewer-mcp")]
                let hover = if self.cc_agent.lock().unwrap().is_some()
                    && self.cc_spawn_root.as_deref() != Some(root.as_str())
                {
                    format!("{root}\n(running Claude Code keeps its original folder until ↺)")
                } else {
                    root.clone()
                };
                #[cfg(not(feature = "viewer-mcp"))]
                let hover = root.clone();
                if chip(ui, &name, &hover, egui::Color32::from_rgb(220, 252, 231)) {
                    self.code_path_buffer.clear();
                }
            }
            let mut to_remove = None;
            for (i, att) in self.attachments.iter().enumerate() {
                if chip(
                    ui,
                    &att.display_name,
                    &att.path,
                    egui::Color32::from_rgb(243, 244, 246),
                ) {
                    to_remove = Some(i);
                }
            }
            if let Some(i) = to_remove {
                self.attachments.remove(i);
            }
        });
        ui.add_space(4.0);
    }

    /// Zone 4: Composer card with attachment chips, multiline input, model selector, send button.
    fn ui_composer(&mut self, ui: &mut egui::Ui) {
        ui.separator();

        // Composer card — rounded frame like Cursor
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(245, 245, 245))
            .rounding(10.0)
            .inner_margin(egui::Margin::same(10.0))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(220, 220, 220)))
            .show(ui, |ui: &mut egui::Ui| {
                // Human-in-the-loop: a pending question from the agent.
                if let Some((request_id, question, options)) = self.pending_question.clone() {
                    ui.label(egui::RichText::new(format!("❓ {question}")).strong());
                    let mut answer: Option<String> = None;
                    if !options.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for opt in &options {
                                if ui.button(opt).clicked() {
                                    answer = Some(opt.clone());
                                }
                            }
                        });
                    }
                    ui.label(
                        egui::RichText::new("…or type your own answer below.")
                            .size(11.0)
                            .weak(),
                    );
                    if let Some(ans) = answer {
                        self.pending_question = None;
                        self.send_user_answer(request_id, ans.clone());
                        self.add_message(ChatMessageKind::User, ans);
                    }
                    ui.add_space(6.0);
                }

                // Current selection pill (live; updates on select/deselect)
                self.ui_selection_pill(ui);

                // Context chips (DB / repo / attached files)
                self.ui_context_chips(ui);

                // Text input (multiline, 2 rows) — use .show() for cursor access
                let enabled = !self.pending_request || self.pending_question.is_some();
                let output = egui::TextEdit::multiline(&mut self.input_buffer)
                    .hint_text("Ask about this profile…")
                    .desired_width(ui.available_width())
                    .desired_rows(2)
                    .frame(false)
                    .interactive(enabled)
                    .show(ui);

                // Enter submits (Shift+Enter = newline)
                let enter_pressed = output.response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !ui.input(|i| i.modifiers.shift);
                if enter_pressed {
                    // Normal submit (or answer a pending ask_user question)
                    let can_submit = !self.pending_request || self.pending_question.is_some();
                    if !self.input_buffer.trim().is_empty() && can_submit {
                        let text = self.input_buffer.trim().to_string();
                        self.input_buffer.clear();
                        self.submit_input(text);
                    }
                }

                // Bottom row: + add-context | model pill | ⏎ Send
                ui.horizontal(|ui| {
                    // + menu (Claude-Desktop style): the ONE place to add context.
                    // Folders/.duckdb configure tools; plain files attach as text.
                    // Plain ASCII "+" (the fullwidth ＋ and the file-kind emojis are
                    // not in egui's default fonts — they render as tofu boxes), and
                    // the popup is forced to open UPWARD like Claude Desktop's
                    // (menu_button drops down, straight out of a bottom bar).
                    let plus_resp = ui
                        .button(
                            egui::RichText::new("+")
                                .size(20.0)
                                .strong()
                                .color(egui::Color32::from_rgb(30, 30, 30)),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Connect the profile DB or code repo, or attach a file");
                    let plus_menu_id = ui.make_persistent_id("plus_context_menu");
                    if plus_resp.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(plus_menu_id));
                    }
                    egui::popup::popup_above_or_below_widget(
                        ui,
                        plus_menu_id,
                        &plus_resp,
                        egui::AboveOrBelow::Above,
                        egui::PopupCloseBehavior::CloseOnClick,
                        |ui| {
                            ui.set_min_width(190.0);
                            menu_row_visuals(ui);
                            let item = |ui: &mut egui::Ui, label: &str| {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(label)
                                            .size(14.5)
                                            .color(egui::Color32::from_rgb(30, 30, 30)),
                                    )
                                    .rounding(6.0)
                                    .min_size(egui::vec2(ui.available_width(), 28.0)),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            };
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                if item(ui, "Connect DuckDB…") {
                                    if let Some(f) = rfd::FileDialog::new()
                                        .set_title("Choose the profile DuckDB")
                                        .add_filter("DuckDB", &["duckdb"])
                                        .pick_file()
                                    {
                                        self.duckdb_path_buffer =
                                            f.to_string_lossy().into_owned();
                                    }
                                }
                                if item(ui, "Connect code repo…") {
                                    if let Some(d) = rfd::FileDialog::new()
                                        .set_title(
                                            "Choose the profiled application's source folder",
                                        )
                                        .pick_folder()
                                    {
                                        self.code_path_buffer =
                                            d.to_string_lossy().into_owned();
                                    }
                                }
                                if item(ui, "Add file…") {
                                    if let Some(f) = rfd::FileDialog::new()
                                        .set_title("Attach a file as context")
                                        .pick_file()
                                    {
                                        let path = f.to_string_lossy().into_owned();
                                        // A .duckdb picked here is a DB, not a text
                                        // attachment (binary would inject garbage).
                                        if f.extension().is_some_and(|e| e == "duckdb") {
                                            self.duckdb_path_buffer = path;
                                        } else if !self
                                            .attachments
                                            .iter()
                                            .any(|a| a.path == path)
                                        {
                                            let display_name = f
                                                .file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| path.clone());
                                            self.attachments
                                                .push(ContextAttachment { path, display_name });
                                        }
                                    }
                                }
                            }
                            #[cfg(target_arch = "wasm32")]
                            ui.label("File dialogs are unavailable in the browser");
                        },
                    );

                    // Right-aligned: Send
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add_enabled(
                                    enabled && !self.input_buffer.trim().is_empty(),
                                    egui::Button::new(
                                        // ↵ lives in Hack (the monospace font) —
                                        // the proportional font lacks all the
                                        // arrow glyphs (they render as boxes).
                                        egui::RichText::new("↵")
                                            .monospace()
                                            .size(19.0)
                                            .color(egui::Color32::WHITE),
                                    )
                                    .fill(egui::Color32::from_rgb(50, 50, 50))
                                    .rounding(18.0)
                                    .min_size(egui::vec2(36.0, 36.0)),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                let can_submit =
                                    !self.pending_request || self.pending_question.is_some();
                                if !self.input_buffer.trim().is_empty() && can_submit {
                                    let text = self.input_buffer.trim().to_string();
                                    self.input_buffer.clear();
                                    self.submit_input(text);
                                }
                            }

                        },
                    );
                });
            });
    }

    /// Render the chat panel. Must be called BEFORE CentralPanel in the layout.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll_events();
        // Keep the MCP server's live project-root handle in sync with the
        // (normalized) path buffer — the server reads it per request.
        #[cfg(feature = "viewer-mcp")]
        self.sync_project_root();

        // These run even while the panel is hidden — a Claude Code turn may be
        // live: the approval dialog must stay answerable (egui Windows float
        // independently of panels) and events must keep draining.
        #[cfg(feature = "viewer-mcp")]
        self.ui_approval_dialog(ctx);
        self.ui_api_key_popup(ctx);
        if self.pending_request {
            ctx.request_repaint();
        }

        // No open/close animation (`show_animated` slid the panel in from the
        // right, shoving the timeline leftward over several frames) — appear
        // instantly instead.
        if !self.visible {
            return;
        }
        egui::SidePanel::right("ai_chat_panel")
            .resizable(true)
            .default_width(420.0)
            .min_width(300.0)
            .frame(
                egui::Frame::side_top_panel(ctx.style().as_ref())
                    .fill(egui::Color32::from_rgb(250, 250, 250)),
            )
            .show(ctx, |ui| {
                // Force dark text throughout this panel
                ui.visuals_mut().override_text_color =
                    Some(egui::Color32::from_rgb(30, 30, 30));
                // Larger, more readable text throughout the chat panel.
                for font_id in ui.style_mut().text_styles.values_mut() {
                    font_id.size *= 1.2;
                }

                // Zone 1: Header bar
                self.ui_header(ui);
                ui.separator();

                // Zone 4: Composer pinned to the bottom. A bottom panel auto-sizes
                // to the composer's real height (input + buttons + selection pill +
                // pending question), so it can never be pushed off-screen as the
                // transcript grows.
                egui::TopBottomPanel::bottom("ai_chat_composer")
                    .resizable(false)
                    .frame(egui::Frame::none().inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 0.0,
                        bottom: 12.0, // float off the panel's bottom edge
                    }))
                    .show_inside(ui, |ui| {
                        self.ui_composer(ui);
                    });

                // Zone 3: Transcript fills the remaining space and scrolls internally.
                // Inner margin keeps message text off the panel walls (the other
                // zones have their own margins: settings 8px, composer 10px).
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
                    )
                    .show_inside(ui, |ui| {
                        self.ui_transcript(ui);
                    });
            });
    }

    /// The Deny / Allow / Always-allow dialog for the Claude Code child's
    /// hook-gated tool calls (Bash/Edit/Write/NotebookEdit/WebFetch/WebSearch).
    /// The /approve handler thread is BLOCKED on this verdict; the modal shows the
    /// FULL command/path/URL (never the 120-char transcript preview — an approval
    /// must be judgeable) plus a severity badge so a shell prompt can never look
    /// like a routine one.
    #[cfg(feature = "viewer-mcp")]
    fn ui_approval_dialog(&mut self, ctx: &egui::Context) {
        use crate::ai::claude_code::{bash_rule_prefix, ApprovalDecision};

        let Some(broker) = self.approval_broker.clone() else { return };
        let Some((id, tool_name, tool_input)) = broker.front() else { return };
        // A verdict can arrive from a background thread at any time — keep frames
        // coming while the dialog is up (cheap: only while a request is pending).
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // Severity tier: the one visual cue that fights rubber-stamp fatigue.
        let (badge, badge_color) = match tool_name.as_str() {
            "Bash" => ("SHELL", egui::Color32::from_rgb(220, 20, 20)),
            "Edit" | "Write" | "NotebookEdit" => ("WRITE", egui::Color32::from_rgb(220, 100, 20)),
            "WebFetch" | "WebSearch" => ("NETWORK", egui::Color32::from_rgb(30, 100, 220)),
            _ => ("TOOL", egui::Color32::from_rgb(120, 120, 120)),
        };
        // What an informed verdict needs to see, per tool.
        let detail: String = match tool_name.as_str() {
            "Bash" => tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("<no command>")
                .to_owned(),
            "Edit" | "Write" | "NotebookEdit" => {
                let path = tool_input
                    .get("file_path")
                    .or_else(|| tool_input.get("notebook_path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("<no path>");
                let body = tool_input
                    .get("content")
                    .or_else(|| tool_input.get("new_string"))
                    .or_else(|| tool_input.get("new_source"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if body.is_empty() { path.to_owned() } else { format!("{path}\n\n{body}") }
            }
            "WebFetch" => tool_input
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("<no url>")
                .to_owned(),
            "WebSearch" => tool_input
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("<no query>")
                .to_owned(),
            _ => tool_input.to_string(),
        };
        // "Always allow" scope: per-tool for non-Bash; per-command-prefix for Bash
        // (and not offered at all for metachar-laden commands).
        let always_label: Option<String> = if tool_name == "Bash" {
            tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .and_then(bash_rule_prefix)
                .map(|p| format!("Always allow `{p} …`"))
        } else {
            Some(format!("Always allow {tool_name}"))
        };

        let mut verdict: Option<(ApprovalDecision, bool)> = None;
        egui::Window::new("Claude Code asks permission")
            .id(egui::Id::new("cc_approval_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(30, 30, 30));
                ui.set_max_width(520.0);
                ui.horizontal(|ui| {
                    egui::Frame::none()
                        .fill(badge_color)
                        .rounding(4.0)
                        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                        .show(ui, |ui: &mut egui::Ui| {
                            ui.label(
                                egui::RichText::new(badge)
                                    .strong()
                                    .size(11.0)
                                    .color(egui::Color32::WHITE),
                            );
                        });
                    ui.label(egui::RichText::new(&tool_name).strong().size(15.0));
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&detail).monospace().size(12.5))
                            .wrap(),
                    );
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("Deny").color(egui::Color32::from_rgb(180, 30, 30)))
                        .clicked()
                    {
                        verdict = Some((ApprovalDecision::Deny, false));
                    }
                    if ui.button("Allow").clicked() {
                        verdict = Some((ApprovalDecision::Allow, false));
                    }
                    if let Some(label) = &always_label {
                        if ui
                            .button(label)
                            .on_hover_text(
                                "Auto-approve matching calls for THIS session only \
                                 (cleared by ↺ New session / app restart)",
                            )
                            .clicked()
                        {
                            verdict = Some((ApprovalDecision::Allow, true));
                        }
                    }
                });
            });
        if let Some((decision, always)) = verdict {
            broker.resolve(id, decision, always);
        }
    }
}

// ── Message rendering ────────────────────────────────────────────────────────

fn render_message(
    ui: &mut egui::Ui,
    msg: &ChatMessage,
    actions: &mut Vec<HighlightAction>,
    md_cache: &mut egui_commonmark::CommonMarkCache,
) {
    match &msg.kind {
        ChatMessageKind::System => {
            if let Some(ref content) = msg.expandable_content {
                let id = ui.make_persistent_id(ui.next_auto_id());
                egui::CollapsingHeader::new(
                    egui::RichText::new(&msg.text)
                        .italics()
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                )
                .id_salt(id)
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("Copy result").clicked() {
                            ui.output_mut(|o| o.copied_text = content.clone());
                        }
                    });
                    ui.label(
                        egui::RichText::new(content)
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::from_rgb(60, 60, 60)),
                    );
                });
            } else {
                ui.label(
                    egui::RichText::new(&msg.text)
                        .italics()
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                );
            }
        }
        ChatMessageKind::User => {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(228, 228, 228))
                    .rounding(8.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&msg.text)
                                .color(egui::Color32::from_rgb(30, 30, 30)),
                        );
                    });
            });
        }
        ChatMessageKind::Analysis => {
            // Copy button — right-aligned at the top of each analysis message
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("📋 Copy")
                        .on_hover_text("Copy full response to clipboard")
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = msg.text.clone());
                    }
                });
            });
            // Proper markdown (headings, lists, fenced code blocks, tables,
            // inline bold/italic/code) — Claude-Desktop-like rendering.
            egui_commonmark::CommonMarkViewer::new().show(ui, md_cache, &msg.text);

            // Highlight chips (user-controlled)
            if !msg.highlights.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Issues found:")
                        .strong()
                        .color(egui::Color32::from_rgb(30, 30, 30)),
                );
                for hl in &msg.highlights {
                    ui.horizontal_wrapped(|ui| {
                        let severity_color = match hl.severity.as_str() {
                            "critical" => egui::Color32::from_rgb(220, 20, 20),
                            "high" => egui::Color32::from_rgb(220, 100, 20),
                            _ => egui::Color32::from_rgb(180, 150, 20),
                        };
                        ui.label(
                            egui::RichText::new(format!("● {}", hl.severity))
                                .color(severity_color)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(&hl.label)
                                .color(egui::Color32::from_rgb(30, 30, 30)),
                        );
                        if ui.small_button("Show \u{25b8}").clicked() {
                            actions.push(HighlightAction {
                                highlight: hl.clone(),
                                zoom_to: true,
                            });
                        }
                    });
                }
                if msg.highlights.len() > 1
                    && ui.small_button("Zoom to all \u{25b8}").clicked()
                {
                    for hl in &msg.highlights {
                        actions.push(HighlightAction {
                            highlight: hl.clone(),
                            zoom_to: true,
                        });
                    }
                }
            }
        }
    }
}

/// Menu-row visuals (Claude-style): rows are invisible until hovered, then a
/// soft grey rounded box. Applied inside popup closures (scoped to that Ui).
fn menu_row_visuals(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(234, 234, 234);
    v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    v.widgets.active.weak_bg_fill = egui::Color32::from_rgb(222, 222, 222);
    v.widgets.active.bg_stroke = egui::Stroke::NONE;
}

/// The EFFECTIVE project root for a raw path-field value: trims, treats
/// empty as unset, and normalizes a FILE path to its parent directory.
/// Every consumer (chip, native agent, MCP handle, Claude Code `--add-dir`)
/// reads through this, so they can never disagree; the buffer itself stays
/// exactly as the user typed it (no cursor-fighting rewrites).
fn effective_project_root(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    if path.is_file() {
        return path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty());
    }
    Some(trimmed.to_owned())
}

/// Format a nanosecond duration into a human-readable string.
fn format_duration_ns(ns: i64) -> String {
    if ns < 1_000 {
        format!("{ns} ns")
    } else if ns < 1_000_000 {
        format!("{:.1} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.3} s", ns as f64 / 1_000_000_000.0)
    }
}

/// The embedded chat panel as an [`EventSink`] — `poll_events` delegates here
/// via `apply_agent_event`. The embedded screenshot reply is delivered by
/// core.rs through `ui_command_tx`, so this sink does not use `reply_tx`.
impl crate::ai::bridge::EventSink for ChatPanel {
    fn on_tool_call(&mut self, name: String, purpose: String) {
        self.add_message(ChatMessageKind::System, format!("  ↳ {name}: {purpose}"));
    }

    fn on_tool_result(&mut self, name: String, summary: String, full_content: String) {
        let has_content = !full_content.is_empty() && full_content != summary;
        self.messages.push(ChatMessage {
            kind: ChatMessageKind::System,
            text: format!("  ✓ {name} ({summary})"),
            highlights: vec![],
            expandable_content: if has_content { Some(full_content) } else { None },
        });
    }

    fn on_navigation(&mut self, nav: PendingNavigation, _reply_tx: &mpsc::Sender<UiCommand>) {
        self.pending_navigation = Some(nav);
    }

    fn on_question(
        &mut self,
        request_id: u64,
        question: String,
        options: Vec<String>,
        _reply_tx: &mpsc::Sender<UiCommand>,
    ) {
        self.add_message(ChatMessageKind::System, format!("❓ {question}"));
        self.pending_question = Some((request_id, question, options));
        self.visible = true;
    }

    fn on_clear_highlights(&mut self) {
        self.pending_clear_highlights = true;
        self.add_message(ChatMessageKind::System, "Cleared timeline highlights.");
    }

    /// Claude Code backend: interim assistant narration streamed mid-turn — rendered
    /// immediately as an Analysis bubble so long runs feel alive. The emitter
    /// (claude_code::map_line) deduplicates the FINAL text against `Complete`.
    fn on_interim_text(&mut self, text: String) {
        if !text.trim().is_empty() {
            self.add_message(ChatMessageKind::Analysis, text);
        }
    }

    fn on_complete(&mut self, response: AgentResponse) {
        let display = if response.text.len() > 10_000 {
            format!(
                "{}…\n\n*(truncated — full response was {} chars)*",
                &response.text[..10_000],
                response.text.len()
            )
        } else {
            response.text
        };

        let highlights = response.highlights;
        // Auto-apply the agent's highlights to the timeline so they appear
        // immediately (deduped in core) — matching the agent's "I've highlighted…"
        // narration. zoom_to:false because the agent has usually navigated already;
        // the "Zoom to all" chip re-fits the view on demand.
        for hl in &highlights {
            self.pending_highlight_actions.push(HighlightAction {
                highlight: hl.clone(),
                zoom_to: false,
            });
        }

        // Embed highlights in the Analysis message as clickable chips. The Claude
        // Code backend deduplicates the final text against its streamed interim
        // messages, so an empty text here means "already rendered" — skip the
        // empty bubble.
        if !display.trim().is_empty() || !highlights.is_empty() {
            self.messages.push(ChatMessage {
                kind: ChatMessageKind::Analysis,
                text: display,
                highlights,
                expandable_content: None,
            });
        }

        if response.queries_executed > 0 {
            self.add_message(
                ChatMessageKind::System,
                format!(
                    "Done. {} quer{} executed.",
                    response.queries_executed,
                    if response.queries_executed == 1 { "y" } else { "ies" }
                ),
            );
        } else {
            // Claude Code backend: queries run inside the MCP server, uncounted here.
            self.add_message(ChatMessageKind::System, "Done.");
        }
        self.pending_request = false;
        self.end_of_turn_channel_cleanup();
    }

    fn on_error(&mut self, error: String) {
        self.add_message(ChatMessageKind::System, format!("Error: {error}"));
        self.pending_request = false;
        self.end_of_turn_channel_cleanup();
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::data::EntryID;
    use crate::timestamp::{Interval, Timestamp};

    /// selection_snapshot mirrors the embedded `build_selection_preamble`
    /// state — the data `get_selection` returns over the MCP. Empty when nothing is
    /// selected; carries item_uid/entry_slug/title/interval + the dragged region.
    #[test]
    fn test_selection_snapshot_empty_and_seeded() {
        let mut p = ChatPanel::new();

        // Nothing selected -> empty snapshot.
        let (items, range) = p.selection_snapshot();
        assert!(items.is_empty() && range.is_none(), "empty selection -> empty snapshot");

        // Seed a selected task bar + a dragged region.
        p.set_item_selection(vec![SelectedItem {
            item_uid: 48,
            entry_slug: Some("n0_cpu_c1".into()),
            title: "top_level <6>".into(),
            start_ns: 100,
            stop_ns: 200,
        }]);
        p.set_selection(TimelineSelection {
            entry_id: EntryID::root(),
            entry_label: "CPU 1".into(),
            interval: Interval::new(Timestamp(50), Timestamp(300)),
        });

        let (items, range) = p.selection_snapshot();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_uid, 48);
        assert_eq!(items[0].entry_slug.as_deref(), Some("n0_cpu_c1"));
        assert_eq!(items[0].title, "top_level <6>");
        assert_eq!((items[0].start_ns, items[0].stop_ns), (100, 200));
        let (label, start, stop) = range.expect("dragged region present");
        assert_eq!(label, "CPU 1");
        assert_eq!((start, stop), (50, 300));
    }
}

#[cfg(test)]
mod project_root_tests {
    use super::*;

    /// The read-boundary normalization every consumer shares — trims,
    /// empty→None, FILE→parent directory.
    #[test]
    fn effective_project_root_normalizes() {
        assert_eq!(effective_project_root(""), None);
        assert_eq!(effective_project_root("   "), None);
        // A directory passes through (trimmed).
        let dir = std::env::temp_dir().join(format!("p3v2_dir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir_s = dir.to_string_lossy().into_owned();
        assert_eq!(effective_project_root(&format!("  {dir_s}  ")), Some(dir_s.clone()));
        // A FILE normalizes to its parent.
        let file = dir.join("kernel.cu");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(
            effective_project_root(&file.to_string_lossy()),
            Some(dir_s.clone()),
            "file path must normalize to its parent directory"
        );
        // A nonexistent path passes through as typed (status turns red; consumers
        // fail with a clear error rather than silently dropping the value).
        assert_eq!(
            effective_project_root("/no/such/dir"),
            Some("/no/such/dir".to_owned())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Persistence: export → apply round-trips the persistable settings,
    /// empty saved values never clobber, and the API key is never exported.
    #[test]
    fn persisted_settings_round_trip_and_precedence() {
        let mut a = ChatPanel::new();
        a.code_path_buffer = "/proj".into();
        a.duckdb_path_buffer = "/db.duckdb".into();
        a.api_key_buffer = "sk-ant-secret".into();
        let saved = a.export_persisted();
        assert_eq!(saved.project_root, "/proj");
        assert_eq!(saved.duckdb_path, "/db.duckdb");
        // The API key must not appear anywhere in the persisted form.
        let json = serde_json::to_string(&saved).unwrap();
        assert!(!json.contains("sk-ant-secret"), "API key must never persist");

        let mut b = ChatPanel::new();
        b.apply_persisted(&saved);
        assert_eq!(b.code_path_buffer, "/proj");
        assert_eq!(b.duckdb_path_buffer, "/db.duckdb");
        assert!(b.api_key_buffer.is_empty());

        // Empty saved values never clobber existing (CLI-set) ones.
        let mut c = ChatPanel::new();
        c.code_path_buffer = "/from-cli".into();
        c.apply_persisted(&crate::app::PersistedAiSettings::default());
        assert_eq!(c.code_path_buffer, "/from-cli");
    }

    /// The shared handle follows buffer edits (and normalizes) — this is
    /// what the MCP server reads per request.
    #[cfg(feature = "viewer-mcp")]
    #[test]
    fn project_root_handle_syncs_live() {
        let mut p = ChatPanel::new();
        let handle = p.project_root_handle();
        assert_eq!(*handle.read().unwrap(), None);
        p.code_path_buffer = "/app/src".into();
        p.sync_project_root();
        assert_eq!(*handle.read().unwrap(), Some("/app/src".to_owned()));
        p.code_path_buffer.clear();
        p.sync_project_root();
        assert_eq!(*handle.read().unwrap(), None, "clearing the field must clear the handle");
    }
}
