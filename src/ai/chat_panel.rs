//! Chat panel UI for AI-powered performance analysis.
//!
//! Provides a Cursor-inspired toggleable right-side panel where users can:
//! - Ask questions about their profile in a composer input
//! - Attach files/folders via `@` mentions (DuckDB databases, code roots, files)
//! - View progressive analysis results with markdown rendering
//! - Configure API key and other settings behind a gear icon
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
#[derive(Clone, Debug)]
pub enum ChatMessageKind {
    /// Gray italic — system status messages
    System,
    /// Right-aligned blue bubble — user input
    User,
    /// Left-aligned with markdown rendering — analysis results
    Analysis,
    /// Compact badge — selection context
    Context,
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

// ── @-mention context types ─────────────────────────────────────────────────

/// The kind of context attachment, auto-detected from the filesystem entry.
#[derive(Clone, Debug, PartialEq)]
pub enum AttachmentKind {
    /// A `.duckdb` file — used as the database for `run_query` tool.
    Database,
    /// A directory — used as the code root for `read_code` tool.
    Folder,
    /// A regular file — contents injected as inline context.
    File,
}

/// A context attachment selected via the `@` picker.
#[derive(Clone, Debug)]
pub struct ContextAttachment {
    /// Full absolute path on disk.
    pub path: String,
    /// Display name (last path component, e.g. "legion_prof.duckdb").
    pub display_name: String,
    /// Auto-detected kind.
    pub kind: AttachmentKind,
}

/// A filesystem entry shown in the `@` picker popup.
#[derive(Clone, Debug)]
struct FsEntry {
    /// Full path.
    path: String,
    /// Just the file/directory name.
    name: String,
    /// Whether this is a directory.
    is_dir: bool,
}

/// Transient state for the `@` mention picker popup.
#[derive(Clone, Debug)]
struct AtPickerState {
    /// Whether the picker popup is currently visible.
    active: bool,
    /// The full query string after '@' (e.g. "leg" or "/Users/a").
    filter: String,
    /// Character offset of '@' in `input_buffer`.
    at_char_offset: usize,
    /// Cached directory listing, refreshed on filter changes.
    entries: Vec<FsEntry>,
    /// The resolved base directory currently being listed.
    base_dir: String,
    /// Index of the keyboard-selected entry.
    selected_index: usize,
    /// After drilling into a directory, skip cursor-based detection for one frame.
    /// The TextEdit cursor hasn't caught up with the programmatic buffer edit.
    drill_pending: bool,
}

impl Default for AtPickerState {
    fn default() -> Self {
        Self {
            active: false,
            filter: String::new(),
            at_char_offset: 0,
            entries: Vec::new(),
            base_dir: String::new(),
            selected_index: 0,
            drill_pending: false,
        }
    }
}

// ── Path picker for tools popover ──────────────────────────────────────────

/// State for a filesystem path picker popup attached to a tools path field.
/// Same visual style as the `@` context picker but sets the path buffer directly.
#[derive(Clone, Debug)]
struct PathPicker {
    /// Whether the popup is visible.
    active: bool,
    /// Filesystem entries matching the current filter.
    entries: Vec<FsEntry>,
    /// The parent directory being listed.
    base_dir: String,
    /// Keyboard-selected entry index.
    selected_index: usize,
    /// Rect of the associated TextEdit (for popup positioning).
    edit_rect: Option<egui::Rect>,
    /// Last buffer value we computed entries for (skip redundant refreshes).
    last_query: String,
}

impl Default for PathPicker {
    fn default() -> Self {
        Self {
            active: false,
            entries: Vec::new(),
            base_dir: String::new(),
            selected_index: 0,
            edit_rect: None,
            last_query: String::new(),
        }
    }
}

// ── Workspace index ─────────────────────────────────────────────────────────

/// An entry in the workspace index used for fast @ picker search.
#[derive(Clone, Debug)]
struct IndexEntry {
    /// Relative path from workspace root (e.g. "src/ai/agent.rs").
    rel_path: String,
    /// Absolute path on disk.
    abs_path: String,
    /// Just the filename or directory name.
    name: String,
    /// Whether this is a directory.
    is_dir: bool,
}

/// A lazily-built index of workspace files for fast @ picker filtering.
#[derive(Clone, Debug)]
struct WorkspaceIndex {
    /// The workspace root directory.
    root: String,
    /// All indexed entries.
    entries: Vec<IndexEntry>,
    /// Whether the index has been built.
    ready: bool,
}

impl Default for WorkspaceIndex {
    fn default() -> Self {
        Self {
            root: String::new(),
            entries: Vec::new(),
            ready: false,
        }
    }
}

impl WorkspaceIndex {
    /// Build the index by walking the workspace directory tree.
    ///
    /// Detects the workspace root by walking up from `cwd` looking for `.git`.
    /// Falls back to `cwd` if no `.git` is found. Ignores heavy directories
    /// (`.git`, `target`, `node_modules`, etc.) and caps entries at 5000.
    fn build_from_cwd() -> Self {
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));

        // Walk up to find .git directory (workspace root)
        let mut root = cwd.clone();
        loop {
            if root.join(".git").exists() {
                break;
            }
            if !root.pop() {
                root = cwd.clone();
                break;
            }
        }

        let root_str = root.to_string_lossy().to_string();
        let mut entries = Vec::new();
        let max_entries = 5000;

        // Directories to skip
        let skip_dirs: &[&str] = &[
            ".git",
            "target",
            "node_modules",
            "prof_results",
            "profiles",
            "__pycache__",
            ".mypy_cache",
            "build",
            "dist",
        ];

        // BFS walk
        let mut queue = vec![root.clone()];
        while let Some(dir) = queue.pop() {
            if entries.len() >= max_entries {
                break;
            }
            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read_dir.flatten() {
                if entries.len() >= max_entries {
                    break;
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                // Skip hidden files (except at root level)
                if name.starts_with('.') {
                    continue;
                }

                let is_dir = path.is_dir();

                // Skip heavy directories
                if is_dir && skip_dirs.contains(&name.as_str()) {
                    continue;
                }

                let abs_path = path.to_string_lossy().to_string();
                let rel_path = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                entries.push(IndexEntry {
                    rel_path,
                    abs_path,
                    name,
                    is_dir,
                });

                if is_dir {
                    queue.push(path);
                }
            }
        }

        // Sort: directories first, then alphabetical by relative path
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.rel_path.cmp(&b.rel_path)));

        Self {
            root: root_str,
            entries,
            ready: true,
        }
    }

    /// Filter entries by a case-insensitive substring match on name or path.
    /// Returns up to `limit` matching entries.
    fn search(&self, query: &str, limit: usize) -> Vec<&IndexEntry> {
        if query.is_empty() {
            // Show top-level entries when no query
            return self
                .entries
                .iter()
                .filter(|e| !e.rel_path.contains('/') || e.rel_path.ends_with('/'))
                .take(limit)
                .collect();
        }

        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&query_lower)
                    || e.rel_path.to_lowercase().contains(&query_lower)
            })
            .take(limit)
            .collect()
    }
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
    scroll_to_bottom: bool,
    /// Whether the settings drawer is expanded.
    settings_open: bool,

    // ── @-mention context ─────────────────────────────────────────────────
    /// Context attachments selected via the `@` picker.
    attachments: Vec<ContextAttachment>,
    /// Transient state for the `@` mention picker.
    at_picker: AtPickerState,
    /// Rect of the composer TextEdit (for popup positioning).
    composer_rect: Option<egui::Rect>,
    /// Programmatic request to open the @ picker on the next frame.
    request_at_picker: bool,
    /// Lazily-built workspace file index for fast @ picker search.
    workspace_index: WorkspaceIndex,

    // ── Tools configuration ──────────────────────────────────────────────
    /// DuckDB database path — required for `run_query` tool.
    duckdb_path_buffer: String,
    /// Application code directory — required for `read_code` tool.
    code_path_buffer: String,
    /// Whether the tools setup popover is open.
    tools_popover_open: bool,
    /// Filesystem picker state for the DB path field.
    db_picker: PathPicker,
    /// Filesystem picker state for the Code path field.
    code_picker: PathPicker,

    // ── Agent state ────────────────────────────────────────────────────────
    /// API key (from UI field; falls back to ANTHROPIC_API_KEY env var).
    api_key_buffer: String,
    /// Model name: "claude-sonnet-4-20250514" or "claude-opus-4-20250514".
    model_selection: String,
    /// Free-text application context (e.g. goals, configuration, number of nodes/GPUs).
    app_context_buffer: String,
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
}

impl Clone for ChatPanel {
    fn clone(&self) -> Self {
        Self {
            visible: self.visible,
            messages: self.messages.clone(),
            input_buffer: self.input_buffer.clone(),
            selection: self.selection.clone(),
            selected_items: self.selected_items.clone(),
            scroll_to_bottom: self.scroll_to_bottom,
            settings_open: self.settings_open,
            attachments: self.attachments.clone(),
            at_picker: self.at_picker.clone(),
            composer_rect: self.composer_rect,
            request_at_picker: self.request_at_picker,
            workspace_index: self.workspace_index.clone(),
            duckdb_path_buffer: self.duckdb_path_buffer.clone(),
            code_path_buffer: self.code_path_buffer.clone(),
            tools_popover_open: self.tools_popover_open,
            db_picker: self.db_picker.clone(),
            code_picker: self.code_picker.clone(),
            api_key_buffer: self.api_key_buffer.clone(),
            model_selection: self.model_selection.clone(),
            app_context_buffer: self.app_context_buffer.clone(),
            agent_session: Arc::clone(&self.agent_session),
            pending_request: self.pending_request,
            event_rx: Arc::clone(&self.event_rx),
            ui_command_tx: self.ui_command_tx.clone(),
            pending_navigation: self.pending_navigation.clone(),
            pending_question: self.pending_question.clone(),
            pending_clear_highlights: self.pending_clear_highlights,
            pending_clear_selection: self.pending_clear_selection,
            pending_highlight_actions: self.pending_highlight_actions.clone(),
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
    /// Create a new chat panel with a welcome message.
    pub fn new() -> Self {
        Self {
            visible: false,
            messages: vec![ChatMessage {
                kind: ChatMessageKind::System,
                text: "Ask me anything about this profile, or try a suggestion below."
                    .into(),
                highlights: vec![],
                expandable_content: None,
            }],
            input_buffer: String::new(),
            selection: None,
            selected_items: Vec::new(),
            scroll_to_bottom: false,
            settings_open: true,
            attachments: Vec::new(),
            at_picker: AtPickerState::default(),
            composer_rect: None,
            request_at_picker: false,
            workspace_index: WorkspaceIndex::default(),
            duckdb_path_buffer: String::new(),
            code_path_buffer: String::new(),
            tools_popover_open: false,
            db_picker: PathPicker::default(),
            code_picker: PathPicker::default(),
            api_key_buffer: String::new(),
            model_selection: "claude-sonnet-4-20250514".into(),
            app_context_buffer: String::new(),
            agent_session: Arc::new(Mutex::new(None)),
            pending_request: false,
            event_rx: Arc::new(Mutex::new(None)),
            ui_command_tx: None,
            pending_navigation: None,
            pending_question: None,
            pending_clear_highlights: false,
            pending_clear_selection: false,
            pending_highlight_actions: Vec::new(),
        }
    }

    /// Toggle panel visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Pre-fill the tool paths (from CLI flags / auto-detection at startup).
    pub fn set_tool_paths(&mut self, duckdb: Option<String>, code: Option<String>) {
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
    }

    /// Add a message to the chat panel.
    pub fn add_message(&mut self, kind: ChatMessageKind, text: impl Into<String>) {
        self.messages.push(ChatMessage {
            kind,
            text: text.into(),
            highlights: vec![],
            expandable_content: None,
        });
        self.scroll_to_bottom = true;
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
            if ui.small_button("✕").on_hover_text("Clear selection").clicked() {
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
    /// in-viewer MCP server (V1.1) serves `run_query`/`overview`/`find_blockers`
    /// against.
    #[cfg(feature = "viewer-mcp")]
    pub fn duckdb_path(&self) -> Option<String> {
        let trimmed = self.duckdb_path_buffer.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
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

    /// Derive the Code tool status from `code_path_buffer`.
    ///
    /// Accepts a directory or a single source file (e.g. `.rg`).  When a file
    /// is given, the agent will use its parent directory as the code root.
    fn tool_status_code(&self) -> ToolStatus {
        let trimmed = self.code_path_buffer.trim();
        if trimmed.is_empty() {
            return ToolStatus::Off;
        }
        let path = std::path::Path::new(trimmed);
        if path.is_dir() {
            return ToolStatus::Ready;
        }
        if path.is_file() {
            return ToolStatus::Ready;
        }
        ToolStatus::Error("Path not found".into())
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

    /// Trigger an agent request in a background thread.
    ///
    /// `user_query = None` → initial "Find Performance Issues" scan.
    /// `user_query = Some(q)` → follow-up question (session persists).
    fn trigger_diagnosis(&mut self, user_query: String) {
        let Some(api_key) = self.get_api_key() else {
            self.add_message(
                ChatMessageKind::System,
                "⚠ API key not set. Open ⚙ Settings or set ANTHROPIC_API_KEY.",
            );
            self.settings_open = true;
            return;
        };

        if self.pending_request {
            self.add_message(
                ChatMessageKind::System,
                "A request is already in progress. Please wait.",
            );
            return;
        }

        // Tool paths from dedicated fields
        let duckdb_path = self.duckdb_path_buffer.trim().to_owned();
        let code_path = self.code_path_buffer.trim().to_owned();

        // Collect inline context from @ attachments (Part D)
        let mut context_section = String::new();
        let mut total_context_bytes: usize = 0;
        let max_total_context: usize = 80_000;
        let max_per_file: usize = 16_000;

        for att in &self.attachments {
            if total_context_bytes >= max_total_context {
                break;
            }
            match att.kind {
                AttachmentKind::File | AttachmentKind::Database => {
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
                AttachmentKind::Folder => {
                    // Shallow directory listing (depth 2)
                    context_section.push_str(&format!(
                        "## Attached folder: {}\n```\n",
                        att.display_name
                    ));
                    if let Ok(entries) = std::fs::read_dir(&att.path) {
                        let mut count = 0;
                        for entry in entries.flatten() {
                            if count >= 50 {
                                context_section.push_str("  …(more entries)\n");
                                break;
                            }
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with('.') {
                                continue;
                            }
                            let is_dir = entry.path().is_dir();
                            let suffix = if is_dir { "/" } else { "" };
                            context_section
                                .push_str(&format!("  {}{}\n", name, suffix));
                            // Depth 2: list children of subdirectories
                            if is_dir {
                                if let Ok(sub) = std::fs::read_dir(entry.path()) {
                                    for sub_entry in sub.flatten().take(20) {
                                        let sub_name = sub_entry
                                            .file_name()
                                            .to_string_lossy()
                                            .to_string();
                                        if sub_name.starts_with('.') {
                                            continue;
                                        }
                                        let sub_dir = sub_entry.path().is_dir();
                                        let s = if sub_dir { "/" } else { "" };
                                        context_section.push_str(&format!(
                                            "    {}{}\n",
                                            sub_name, s
                                        ));
                                    }
                                }
                            }
                            count += 1;
                        }
                    }
                    context_section.push_str("```\n\n");
                }
            }
        }

        self.add_message(ChatMessageKind::User, &user_query);
        let time_hint = if self.model_selection.contains("opus") {
            "Opus with extended thinking — allow 3–5 min"
        } else {
            "Sonnet — typically 30–90 s"
        };
        self.add_message(
            ChatMessageKind::System,
            format!("Working… ({time_hint})"),
        );
        self.pending_request = true;

        let app_context = self.app_context_buffer.clone();
        let model = self.model_selection.clone();
        let session_arc = Arc::clone(&self.agent_session);
        let selection_preamble = self.build_selection_preamble();
        let query_clone = user_query;

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
                    s.update_channels(event_tx.clone(), cmd_rx);
                    s
                }
                None => AgentSession::new(
                    api_key,
                    model,
                    duckdb_path,
                    code_path,
                    app_context,
                    event_tx.clone(),
                    cmd_rx,
                ),
            };

            // Run the agent — prepend selection context + @ attachments to the question.
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

    // ── @-mention picker logic ──────────────────────────────────────────────

    /// Detect whether the user has typed an active `@` trigger in the input buffer.
    fn detect_at_trigger(&mut self, output: &egui::text_edit::TextEditOutput) {
        if !output.response.has_focus() {
            // Don't close the picker on focus loss — the user may be clicking
            // entries in the popup (which steals focus from the TextEdit).
            // The picker closes via Escape, file acceptance, or buffer invalidation.
            return;
        }

        // After drilling into a directory, the TextEdit cursor hasn't caught up
        // with our programmatic buffer edit. Skip one detection cycle.
        if self.at_picker.drill_pending {
            self.at_picker.drill_pending = false;
            return;
        }

        // If picker is active, verify the buffer still contains our expected @filter
        // at the known offset. This avoids cursor-position-dependent re-detection,
        // which breaks after programmatic buffer edits (cursor may be mid-filter).
        if self.at_picker.active {
            let at_pos = self.at_picker.at_char_offset;
            let expected = format!("@{}", self.at_picker.filter);
            if self
                .input_buffer
                .get(at_pos..)
                .map_or(false, |s| s.starts_with(&expected))
            {
                // Buffer still matches — check if user typed more AFTER the filter
                if let Some(cursor_range) = &output.cursor_range {
                    let cursor_pos = cursor_range.primary.ccursor.index;
                    let filter_end = at_pos + 1 + self.at_picker.filter.len();
                    if cursor_pos > filter_end {
                        // User typed more characters after the known filter
                        let new_end = cursor_pos.min(self.input_buffer.len());
                        let new_filter =
                            self.input_buffer[at_pos + 1..new_end].to_owned();
                        if new_filter != self.at_picker.filter {
                            self.at_picker.selected_index = 0;
                            self.refresh_fs_entries(&new_filter);
                            self.at_picker.filter = new_filter;
                        }
                    }
                    // cursor <= filter_end: user's cursor is mid-filter, keep state
                }
                return;
            }
            // Buffer no longer matches — close picker, fall through to re-detect
            self.at_picker.active = false;
        }

        let Some(cursor_range) = &output.cursor_range else {
            return;
        };

        let cursor_pos = cursor_range.primary.ccursor.index;
        let text = &self.input_buffer;
        let end = cursor_pos.min(text.len());
        let before_cursor = &text[..end];

        // Scan backwards from cursor for '@'
        if let Some(at_pos) = before_cursor.rfind('@') {
            // Validate: '@' must be at start or preceded by whitespace
            let valid_trigger = at_pos == 0 || {
                let prev_char = text[..at_pos].chars().next_back();
                prev_char.map_or(true, |c| c.is_whitespace())
            };

            if valid_trigger {
                let filter = before_cursor[at_pos + 1..].to_owned();
                // New @ trigger — initialize picker
                self.at_picker.active = true;
                self.at_picker.at_char_offset = at_pos;
                self.at_picker.selected_index = 0;
                self.refresh_fs_entries(&filter);
                self.at_picker.filter = filter;
                return;
            }
        }

        // No valid @ found
        if self.at_picker.active {
            self.at_picker.active = false;
        }
    }

    /// Refresh the filesystem entry listing based on the current filter.
    ///
    /// Two modes:
    /// - **Workspace search** (no leading `/`): uses the in-memory workspace index
    ///   for fast substring filtering. The index is built lazily on first use.
    /// - **Absolute path browsing** (starts with `/`): uses live filesystem listing
    ///   for drilling into arbitrary directories.
    fn refresh_fs_entries(&mut self, filter: &str) {
        if filter.starts_with('/') || filter.starts_with('~') {
            // Absolute path browsing (existing behavior)
            let expanded = if filter.starts_with('~') {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
                filter.replacen('~', &home, 1)
            } else {
                filter.to_owned()
            };
            let (base_dir, name_filter) = if expanded.contains('/') {
                let last_slash = expanded.rfind('/').unwrap();
                (
                    expanded[..=last_slash].to_owned(),
                    &filter[filter.len() - (expanded.len() - last_slash - 1)..],
                )
            } else {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
                (home, filter)
            };
            self.at_picker.base_dir = base_dir.clone();
            self.at_picker.entries = list_directory(&base_dir, name_filter);
        } else {
            // Workspace index search — build lazily on first use
            if !self.workspace_index.ready {
                self.workspace_index = WorkspaceIndex::build_from_cwd();
            }

            self.at_picker.base_dir = self.workspace_index.root.clone();
            let results = self.workspace_index.search(filter, 20);
            self.at_picker.entries = results
                .into_iter()
                .map(|e| FsEntry {
                    path: e.abs_path.clone(),
                    name: if filter.is_empty() {
                        e.name.clone()
                    } else {
                        e.rel_path.clone()
                    },
                    is_dir: e.is_dir,
                })
                .collect();
        }
    }

    /// Handle keyboard navigation in the @ picker. Returns true if a key was consumed.
    fn handle_picker_keys(&mut self, ui: &egui::Ui) -> bool {
        if !self.at_picker.active || self.at_picker.entries.is_empty() {
            return false;
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.at_picker.active = false;
            return true;
        }

        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            let max = self.at_picker.entries.len().saturating_sub(1);
            if self.at_picker.selected_index < max {
                self.at_picker.selected_index += 1;
            }
            return true;
        }

        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.at_picker.selected_index = self.at_picker.selected_index.saturating_sub(1);
            return true;
        }

        if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
            let idx = self.at_picker.selected_index;
            self.accept_picker_entry(idx);
            return true;
        }

        false
    }

    /// Accept the picker entry at `index` — attach it or drill into a directory.
    fn accept_picker_entry(&mut self, index: usize) {
        let Some(entry) = self.at_picker.entries.get(index).cloned() else {
            return;
        };

        let at_offset = self.at_picker.at_char_offset;
        let old_trigger_len = 1 + self.at_picker.filter.len(); // '@' + filter text

        if entry.is_dir {
            // Drill into directory: replace @filter with @full_path/
            let new_filter = format!("{}/", entry.path);
            let before = &self.input_buffer[..at_offset];
            let after_start = (at_offset + old_trigger_len).min(self.input_buffer.len());
            let after = self.input_buffer[after_start..].trim_start_matches('\n');
            self.input_buffer = format!("{}@{}{}", before, new_filter, after);
            self.at_picker.filter = new_filter;
            let filter_clone = self.at_picker.filter.clone();
            self.at_picker.selected_index = 0;
            // Skip one detect_at_trigger cycle — cursor hasn't caught up with
            // the programmatic buffer edit yet.
            self.at_picker.drill_pending = true;
            self.refresh_fs_entries(&filter_clone);
            return;
        }

        // File: create attachment
        let path = std::path::Path::new(&entry.path);
        let kind = classify_path(path);
        let display_name = entry.name.clone();

        // Don't add duplicates
        if !self.attachments.iter().any(|a| a.path == entry.path) {
            self.attachments.push(ContextAttachment {
                path: entry.path.clone(),
                display_name,
                kind,
            });
        }

        // Remove @query from input buffer
        let before = &self.input_buffer[..at_offset];
        let after_start = (at_offset + old_trigger_len).min(self.input_buffer.len());
        let after = self.input_buffer[after_start..].trim_start_matches('\n');
        self.input_buffer = format!("{}{}", before, after);

        // Close picker
        self.at_picker = AtPickerState::default();
    }

    // ── Zone methods ────────────────────────────────────────────────────────

    /// Zone 1: Header bar with title, tool status chips, and action buttons.
    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Right-aligned controls (no title — it's redundant with the toolbar toggle)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Settings gear (toggles settings_open)
                let gear_text = if self.settings_open { "⚙ ▾" } else { "⚙" };
                if ui.button(gear_text).clicked() {
                    self.settings_open = !self.settings_open;
                }
                // New session button
                if ui
                    .add_enabled(!self.pending_request, egui::Button::new("↺"))
                    .on_hover_text("New session")
                    .clicked()
                {
                    *self.agent_session.lock().unwrap() = None;
                    self.add_message(ChatMessageKind::System, "Session cleared.");
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
                    "DB ●",
                    egui::Color32::from_rgb(34, 139, 34),
                    "Database connected".to_string(),
                ),
                ToolStatus::Error(msg) => (
                    "DB ✕",
                    egui::Color32::from_rgb(220, 60, 60),
                    format!("Error: {msg}"),
                ),
                ToolStatus::Off => (
                    "DB ○",
                    egui::Color32::from_rgb(160, 160, 160),
                    "Click to configure DB path".to_string(),
                ),
            };
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(db_label).small().color(db_color),
                    )
                    .frame(false),
                )
                .on_hover_text(&db_hover)
                .clicked()
            {
                self.tools_popover_open = !self.tools_popover_open;
            }

            // Code chip
            let (code_label, code_color, code_hover) = match &code_status {
                ToolStatus::Ready => (
                    "Code ●",
                    egui::Color32::from_rgb(34, 139, 34),
                    "Code root configured".to_string(),
                ),
                ToolStatus::Error(msg) => (
                    "Code ✕",
                    egui::Color32::from_rgb(220, 60, 60),
                    format!("Error: {msg}"),
                ),
                ToolStatus::Off => (
                    "Code ○",
                    egui::Color32::from_rgb(160, 160, 160),
                    "Optional — set code path for read_code".to_string(),
                ),
            };
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(code_label).small().color(code_color),
                    )
                    .frame(false),
                )
                .on_hover_text(&code_hover)
                .clicked()
            {
                self.tools_popover_open = !self.tools_popover_open;
            }

            // Visual chip
            let (vis_label, vis_color) = match &visual_status {
                ToolStatus::Ready => ("Visual ●", egui::Color32::from_rgb(34, 139, 34)),
                _ => ("Visual ○", egui::Color32::from_rgb(160, 160, 160)),
            };
            ui.label(egui::RichText::new(vis_label).small().color(vis_color))
                .on_hover_text("Screenshot + zoom always available");

            // Model + API key status
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let model_short = if self.model_selection.contains("opus") {
                    "Opus"
                } else {
                    "Sonnet"
                };
                if has_key {
                    ui.label(
                        egui::RichText::new(model_short)
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
                        self.settings_open = true;
                    }
                }
            });
        });
    }

    /// Zone 2: Collapsible settings drawer (API key, app context).
    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(245, 245, 245))
            .rounding(4.0)
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui: &mut egui::Ui| {
                ui.label("API Key:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.api_key_buffer)
                        .password(true)
                        .hint_text("sk-ant-… (or set ANTHROPIC_API_KEY)")
                        .desired_width(ui.available_width()),
                );

                ui.add_space(4.0);
                ui.label("App context:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.app_context_buffer)
                        .hint_text("e.g. goals, configuration, number of nodes/GPUs")
                        .desired_width(ui.available_width())
                        .desired_rows(2),
                );
                ui.label(
                    egui::RichText::new("Helps the AI tailor analysis to your goals")
                        .small()
                        .italics()
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                );
            });
    }

    /// Zone 3: Message transcript scroll area.
    fn ui_transcript(&mut self, ui: &mut egui::Ui) {
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
                                    ChatMessageKind::Context => "[context]",
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
                // Swap messages out to avoid borrowing self.messages
                // immutably while self.pending_highlight_actions is
                // borrowed mutably by render_message().
                let messages = std::mem::take(&mut self.messages);
                for msg in &messages {
                    render_message(ui, msg, &mut self.pending_highlight_actions);
                    ui.add_space(4.0);
                }
                self.messages = messages;
            });
    }

    /// Render context attachment chips above the composer input.
    fn ui_attachment_chips(&mut self, ui: &mut egui::Ui) {
        if self.attachments.is_empty() {
            return;
        }

        ui.horizontal_wrapped(|ui| {
            let mut to_remove = None;
            for (i, att) in self.attachments.iter().enumerate() {
                let (icon, bg_color) = match att.kind {
                    AttachmentKind::Database => {
                        ("🗄", egui::Color32::from_rgb(219, 234, 254))
                    }
                    AttachmentKind::Folder => {
                        ("📁", egui::Color32::from_rgb(220, 252, 231))
                    }
                    AttachmentKind::File => {
                        ("📄", egui::Color32::from_rgb(243, 244, 246))
                    }
                };

                egui::Frame::none()
                    .fill(bg_color)
                    .rounding(12.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                    .show(ui, |ui: &mut egui::Ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(egui::RichText::new(icon).size(11.0));
                            ui.label(
                                egui::RichText::new(&att.display_name)
                                    .size(11.5)
                                    .color(egui::Color32::from_rgb(30, 30, 30)),
                            );
                            if ui
                                .small_button("✕")
                                .on_hover_text(&att.path)
                                .clicked()
                            {
                                to_remove = Some(i);
                            }
                        });
                    });
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

        // Handle programmatic @ picker open
        if self.request_at_picker {
            self.request_at_picker = false;
            // Insert @ at end of buffer and activate picker
            self.input_buffer.push('@');
            let at_pos = self.input_buffer.len() - 1;
            self.at_picker.active = true;
            self.at_picker.at_char_offset = at_pos;
            self.at_picker.filter.clear();
            self.at_picker.selected_index = 0;
            self.refresh_fs_entries("");
        }

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

                // Attachment chips
                self.ui_attachment_chips(ui);

                // Text input (multiline, 2 rows) — use .show() for cursor access
                let enabled = !self.pending_request || self.pending_question.is_some();
                let output = egui::TextEdit::multiline(&mut self.input_buffer)
                    .hint_text("Ask about this profile… (@ to attach context)")
                    .desired_width(ui.available_width())
                    .desired_rows(2)
                    .frame(false)
                    .interactive(enabled)
                    .show(ui);

                // Store rect for popup positioning
                self.composer_rect = Some(output.response.rect);

                // Detect @ trigger and update picker state
                self.detect_at_trigger(&output);

                // Handle keyboard navigation in picker (consumes ↑/↓/Tab/Esc)
                let picker_consumed = self.handle_picker_keys(ui);

                // Enter key handling: accept picker entry OR submit message
                let enter_pressed = output.response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !ui.input(|i| i.modifiers.shift);

                if enter_pressed && self.at_picker.active && !self.at_picker.entries.is_empty() {
                    // Enter accepts the selected picker entry
                    let idx = self.at_picker.selected_index;
                    self.accept_picker_entry(idx);
                } else if enter_pressed && !picker_consumed && !self.at_picker.active {
                    // Normal submit (or answer a pending ask_user question)
                    let can_submit = !self.pending_request || self.pending_question.is_some();
                    if !self.input_buffer.trim().is_empty() && can_submit {
                        let text = self.input_buffer.trim().to_string();
                        self.input_buffer.clear();
                        self.submit_input(text);
                    }
                }

                // Bottom row: @ Context | model pill | 🔧 Tools | ⏎ Send
                ui.horizontal(|ui| {
                    // @ Context button (with count badge)
                    let ctx_label = if self.attachments.is_empty() {
                        "@ Context".to_string()
                    } else {
                        format!("@ Context ({})", self.attachments.len())
                    };
                    if ui
                        .add_enabled(enabled, egui::Button::new(
                            egui::RichText::new(&ctx_label).size(11.5),
                        ))
                        .on_hover_text("Attach files or folders as context")
                        .clicked()
                    {
                        self.request_at_picker = true;
                    }

                    // Model selector as a compact ComboBox
                    egui::ComboBox::from_id_salt("model_pill")
                        .selected_text(if self.model_selection.contains("opus") {
                            "Opus"
                        } else {
                            "Sonnet"
                        })
                        .width(72.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model_selection,
                                "claude-sonnet-4-20250514".into(),
                                "Sonnet — fast",
                            );
                            ui.selectable_value(
                                &mut self.model_selection,
                                "claude-opus-4-20250514".into(),
                                "Opus — deep",
                            );
                        });

                    // Right-aligned: Tools + Send
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            // Send button
                            if ui
                                .add_enabled(
                                    enabled && !self.input_buffer.trim().is_empty(),
                                    egui::Button::new("⏎")
                                        .rounding(20.0)
                                        .min_size(egui::vec2(32.0, 32.0)),
                                )
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

                            // Tools button
                            let tools_text = if self.tools_popover_open {
                                "🔧 ▾"
                            } else {
                                "🔧"
                            };
                            if ui
                                .button(egui::RichText::new(tools_text).size(11.5))
                                .on_hover_text("Tools Setup — configure DB & code paths")
                                .clicked()
                            {
                                self.tools_popover_open = !self.tools_popover_open;
                            }
                        },
                    );
                });
            });
    }

    /// Render the `@` mention picker popup above the composer.
    fn show_at_picker(&mut self, ctx: &egui::Context, anchor_rect: egui::Rect) {
        if !self.at_picker.active || self.at_picker.entries.is_empty() {
            return;
        }

        let popup_id = egui::Id::new("at_mention_picker");
        let pos = egui::pos2(anchor_rect.left(), anchor_rect.top() - 4.0);

        egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui: &mut egui::Ui| {
                        ui.set_max_width(anchor_rect.width().max(280.0));
                        ui.set_max_height(250.0);

                        // Breadcrumb: current directory
                        ui.label(
                            egui::RichText::new(&self.at_picker.base_dir)
                                .small()
                                .color(egui::Color32::from_rgb(120, 120, 120)),
                        );
                        ui.separator();

                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for (i, entry) in self.at_picker.entries.iter().enumerate() {
                                    let is_selected = i == self.at_picker.selected_index;
                                    let icon = if entry.is_dir {
                                        "📁"
                                    } else if entry.name.ends_with(".duckdb") {
                                        "🗄"
                                    } else {
                                        "📄"
                                    };
                                    let suffix = if entry.is_dir { "/" } else { "" };
                                    let label = format!("{} {}{}", icon, entry.name, suffix);

                                    let response = ui.selectable_label(is_selected, &label);

                                    if response.clicked() {
                                        self.accept_picker_entry(i);
                                        return; // exit early — entries may have changed
                                    }
                                }
                            });
                    });
            });
    }

    /// Render the Tools Setup popover (DB path, Code path, capabilities).
    fn show_tools_popover(&mut self, ctx: &egui::Context, anchor_rect: egui::Rect) {
        let popup_id = egui::Id::new("tools_setup_popover");
        let pos = egui::pos2(anchor_rect.left(), anchor_rect.top() - 4.0);

        egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui: &mut egui::Ui| {
                        ui.set_max_width(anchor_rect.width().max(300.0));

                        // Force dark text
                        ui.visuals_mut().override_text_color =
                            Some(egui::Color32::from_rgb(30, 30, 30));

                        ui.label(
                            egui::RichText::new("Tools Setup")
                                .strong()
                                .size(13.0),
                        );
                        ui.separator();

                        // ── Quick Setup ──────────────────────────────────
                        ui.label(egui::RichText::new("Quick Setup").strong().size(12.0));
                        ui.add_space(2.0);

                        // DB path with picker
                        ui.label("Database path:");
                        path_field_with_picker(
                            ui,
                            egui::Id::new("tools_db_path"),
                            &mut self.duckdb_path_buffer,
                            &mut self.db_picker,
                            "/path/to/legion_prof.duckdb",
                        );
                        let db_status = self.tool_status_db();
                        let (db_icon, db_msg, db_color) = match &db_status {
                            ToolStatus::Ready => (
                                "●",
                                "Ready".to_string(),
                                egui::Color32::from_rgb(34, 139, 34),
                            ),
                            ToolStatus::Off => (
                                "○",
                                "Not set".to_string(),
                                egui::Color32::from_rgb(160, 160, 160),
                            ),
                            ToolStatus::Error(e) => (
                                "✕",
                                e.clone(),
                                egui::Color32::from_rgb(220, 60, 60),
                            ),
                        };
                        ui.label(
                            egui::RichText::new(format!("{db_icon} {db_msg}"))
                                .small()
                                .color(db_color),
                        );

                        ui.add_space(4.0);

                        // Code path with picker
                        ui.label("Code path:");
                        path_field_with_picker(
                            ui,
                            egui::Id::new("tools_code_path"),
                            &mut self.code_path_buffer,
                            &mut self.code_picker,
                            "/path/to/app/src (optional)",
                        );
                        let code_status = self.tool_status_code();
                        let (code_icon, code_msg, code_color) = match &code_status {
                            ToolStatus::Ready => (
                                "●",
                                "Ready".to_string(),
                                egui::Color32::from_rgb(34, 139, 34),
                            ),
                            ToolStatus::Off => (
                                "○",
                                "Optional".to_string(),
                                egui::Color32::from_rgb(160, 160, 160),
                            ),
                            ToolStatus::Error(e) => (
                                "✕",
                                e.clone(),
                                egui::Color32::from_rgb(220, 60, 60),
                            ),
                        };
                        ui.label(
                            egui::RichText::new(format!("{code_icon} {code_msg}"))
                                .small()
                                .color(code_color),
                        );

                        ui.add_space(8.0);
                        ui.separator();

                        // ── Capabilities ─────────────────────────────────
                        ui.label(egui::RichText::new("Capabilities").strong().size(12.0));
                        ui.add_space(2.0);

                        let cap_muted = egui::Color32::from_rgb(100, 100, 100);

                        // run_query
                        ui.horizontal(|ui| {
                            let (icon, color) = if db_status == ToolStatus::Ready {
                                ("●", egui::Color32::from_rgb(34, 139, 34))
                            } else {
                                ("○", egui::Color32::from_rgb(160, 160, 160))
                            };
                            ui.label(egui::RichText::new(icon).color(color));
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("Query profile database").size(12.0));
                                let desc = if db_status == ToolStatus::Ready {
                                    "Ready — run_query"
                                } else {
                                    "Needs DB path"
                                };
                                ui.label(egui::RichText::new(desc).small().color(cap_muted));
                            });
                        });

                        // read_code
                        ui.horizontal(|ui| {
                            let (icon, color) = if code_status == ToolStatus::Ready {
                                ("●", egui::Color32::from_rgb(34, 139, 34))
                            } else {
                                ("○", egui::Color32::from_rgb(160, 160, 160))
                            };
                            ui.label(egui::RichText::new(icon).color(color));
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Read application code").size(12.0),
                                );
                                let desc = if code_status == ToolStatus::Ready {
                                    "Ready — read_code"
                                } else {
                                    "Optional — set code path"
                                };
                                ui.label(egui::RichText::new(desc).small().color(cap_muted));
                            });
                        });

                        // screenshot / zoom_to
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("●")
                                    .color(egui::Color32::from_rgb(34, 139, 34)),
                            );
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Visual inspection").size(12.0),
                                );
                                ui.label(
                                    egui::RichText::new("Ready — screenshot, zoom_to")
                                        .small()
                                        .color(cap_muted),
                                );
                            });
                        });
                    });
            });
    }

    /// Render the chat panel. Must be called BEFORE CentralPanel in the layout.
    pub fn show(&mut self, ctx: &egui::Context) {
        self.poll_events();

        egui::SidePanel::right("ai_chat_panel")
            .resizable(true)
            .default_width(420.0)
            .min_width(300.0)
            .frame(
                egui::Frame::side_top_panel(ctx.style().as_ref())
                    .fill(egui::Color32::from_rgb(250, 250, 250)),
            )
            .show_animated(ctx, self.visible, |ui| {
                // Force dark text throughout this panel
                ui.visuals_mut().override_text_color =
                    Some(egui::Color32::from_rgb(30, 30, 30));
                // Slightly larger, more readable text throughout the chat panel.
                for font_id in ui.style_mut().text_styles.values_mut() {
                    font_id.size *= 1.1;
                }

                // Zone 1: Header bar
                self.ui_header(ui);
                ui.separator();

                // Zone 2: Settings (collapsible, only visible when settings_open)
                if self.settings_open {
                    self.ui_settings(ui);
                    ui.separator();
                }

                // Zone 4: Composer pinned to the bottom. A bottom panel auto-sizes
                // to the composer's real height (input + buttons + selection pill +
                // pending question), so it can never be pushed off-screen as the
                // transcript grows.
                egui::TopBottomPanel::bottom("ai_chat_composer")
                    .resizable(false)
                    .frame(egui::Frame::none())
                    .show_inside(ui, |ui| {
                        self.ui_composer(ui);
                    });

                // Zone 3: Transcript fills the remaining space and scrolls internally.
                egui::CentralPanel::default()
                    .frame(egui::Frame::none())
                    .show_inside(ui, |ui| {
                        self.ui_transcript(ui);
                    });
            });

        // Popups (rendered AFTER the panel so they overlay correctly)
        if self.visible && self.at_picker.active {
            if let Some(rect) = self.composer_rect {
                self.show_at_picker(ctx, rect);
            }
        }
        if self.visible && self.tools_popover_open {
            if let Some(rect) = self.composer_rect {
                self.show_tools_popover(ctx, rect);
            }
        }

        // Path picker popups (rendered AFTER tools popover so they overlay on top)
        if self.visible && self.tools_popover_open && self.db_picker.active {
            show_path_picker_popup(
                ctx,
                &mut self.db_picker,
                &mut self.duckdb_path_buffer,
                "db_path_picker",
                egui::Id::new("tools_db_path"),
            );
        }
        if self.visible && self.tools_popover_open && self.code_picker.active {
            show_path_picker_popup(
                ctx,
                &mut self.code_picker,
                &mut self.code_path_buffer,
                "code_path_picker",
                egui::Id::new("tools_code_path"),
            );
        }

        // Keep repainting while waiting so poll_events() fires promptly
        if self.pending_request {
            ctx.request_repaint();
        }
    }
}

// ── Message rendering ────────────────────────────────────────────────────────

fn render_message(ui: &mut egui::Ui, msg: &ChatMessage, actions: &mut Vec<HighlightAction>) {
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
            render_analysis_markdown(ui, &msg.text);

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
                if msg.highlights.len() > 1 {
                    if ui.small_button("Zoom to all \u{25b8}").clicked() {
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
        ChatMessageKind::Context => {
            ui.label(
                egui::RichText::new(&msg.text)
                    .small()
                    .color(egui::Color32::from_rgb(37, 99, 235)),
            );
        }
    }
}

/// Render analysis text with basic markdown formatting.
///
/// Supports: `## headings`, `**bold**`, `- bullets`, `| tables` (monospace),
/// blank-line paragraph breaks, and numbered lists.
fn render_analysis_markdown(ui: &mut egui::Ui, text: &str) {
    let dark = egui::Color32::from_rgb(30, 30, 30);
    let heading_color = egui::Color32::from_rgb(10, 10, 10);
    let muted = egui::Color32::from_rgb(80, 80, 80);

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }

        // # Heading levels
        if let Some(h) = trimmed.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(h)
                    .strong()
                    .size(16.0)
                    .color(heading_color),
            );
            ui.add_space(1.0);
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(h)
                    .strong()
                    .size(17.0)
                    .color(heading_color),
            );
            ui.add_space(2.0);
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("# ") {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(h)
                    .strong()
                    .size(18.0)
                    .color(heading_color),
            );
            ui.add_space(3.0);
            continue;
        }

        // | table row — monospace, skip separators
        if trimmed.starts_with('|') {
            if trimmed.contains("---") {
                continue;
            }
            ui.label(
                egui::RichText::new(trimmed)
                    .monospace()
                    .size(12.0)
                    .color(dark),
            );
            continue;
        }

        // - / * bullet
        if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("  •").color(muted));
                render_inline_markdown(ui, rest, dark);
            });
            continue;
        }

        // Numbered list: "1. item"
        if trimmed.len() > 2
            && trimmed.as_bytes()[0].is_ascii_digit()
            && trimmed.contains(". ")
        {
            if let Some(pos) = trimmed.find(". ") {
                let number = &trimmed[..pos + 1];
                let rest = &trimmed[pos + 2..];
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("  {number}")).color(muted));
                    render_inline_markdown(ui, rest, dark);
                });
                continue;
            }
        }

        // Regular paragraph
        ui.horizontal_wrapped(|ui| {
            render_inline_markdown(ui, trimmed, dark);
        });
    }
}

/// Render a single text line with inline `**bold**` spans.
fn render_inline_markdown(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    let mut remaining = text;
    while let Some(start) = remaining.find("**") {
        let before = &remaining[..start];
        if !before.is_empty() {
            ui.label(egui::RichText::new(before).color(color));
        }
        remaining = &remaining[start + 2..];
        if let Some(end) = remaining.find("**") {
            let bold_text = &remaining[..end];
            ui.label(egui::RichText::new(bold_text).strong().color(color));
            remaining = &remaining[end + 2..];
        } else {
            ui.label(egui::RichText::new(format!("**{remaining}")).color(color));
            return;
        }
    }
    if !remaining.is_empty() {
        ui.label(egui::RichText::new(remaining).color(color));
    }
}

// ── Path field with picker ──────────────────────────────────────────────────

/// Render a path text field with an associated filesystem picker.
///
/// Handles Tab-completion, keyboard navigation (↑/↓/Enter/Escape), and
/// picker state updates. The actual popup is rendered separately via
/// `show_path_picker_popup()` — this function only manages state and the TextEdit.
fn path_field_with_picker(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    buffer: &mut String,
    picker: &mut PathPicker,
    hint: &str,
) {
    let had_focus = ui.ctx().memory(|m| m.has_focus(edit_id));

    // Consume keys BEFORE the TextEdit renders to prevent default focus behavior.
    let tab = had_focus
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
    let enter = had_focus
        && picker.active
        && !picker.entries.is_empty()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let up = had_focus
        && picker.active
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
    let down = had_focus
        && picker.active
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
    let esc = had_focus
        && picker.active
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let resp = ui.add(
        egui::TextEdit::singleline(buffer)
            .id(edit_id)
            .hint_text(hint)
            .desired_width(ui.available_width()),
    );

    // Tab-completion (inline, modifies buffer)
    if tab {
        tab_complete_path(buffer);
    }

    // Picker keyboard navigation
    if esc {
        picker.active = false;
    } else if up {
        picker.selected_index = picker.selected_index.saturating_sub(1);
    } else if down {
        let max = picker.entries.len().saturating_sub(1);
        if picker.selected_index < max {
            picker.selected_index += 1;
        }
    } else if enter {
        let idx = picker.selected_index;
        accept_path_picker_entry(picker, buffer, idx);
    }

    // Update picker state and store rect for popup positioning
    picker.edit_rect = Some(resp.rect);
    update_path_picker(picker, buffer, resp.has_focus());
}

/// Update a path picker's entries based on the current buffer text.
///
/// Only refreshes when the TextEdit has focus and the buffer has changed.
/// Intentionally does NOT close the picker on focus loss — the user may be
/// clicking entries in the popup (which steals focus from the TextEdit).
fn update_path_picker(picker: &mut PathPicker, buffer: &str, has_focus: bool) {
    let trimmed = buffer.trim();

    // Close if buffer is empty
    if trimmed.is_empty() {
        picker.active = false;
        picker.entries.clear();
        picker.last_query.clear();
        return;
    }

    // Expand ~ to $HOME
    let expanded = if trimmed.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
        trimmed.replacen('~', &home, 1)
    } else {
        trimmed.to_owned()
    };

    // Only show for path-like text (starting with / or ~ which was expanded)
    if !expanded.starts_with('/') {
        picker.active = false;
        picker.entries.clear();
        return;
    }

    // Only refresh when focused; on focus loss, keep current state for popup clicks
    if !has_focus {
        return;
    }

    // Skip refresh if query hasn't changed
    if expanded == picker.last_query {
        return;
    }
    picker.last_query = expanded.clone();

    // Split into (parent_dir, partial_name)
    let (dir, partial) = if expanded.ends_with('/') {
        (expanded.as_str(), "")
    } else if let Some(pos) = expanded.rfind('/') {
        (&expanded[..=pos], &expanded[pos + 1..])
    } else {
        picker.active = false;
        return;
    };

    if !std::path::Path::new(dir).is_dir() {
        picker.entries.clear();
        picker.active = false;
        return;
    }

    picker.base_dir = dir.to_owned();
    picker.entries = list_directory(dir, partial);
    picker.selected_index = 0;
    picker.active = !picker.entries.is_empty();
}

/// Accept the selected entry in a path picker.
///
/// - **Directory**: updates buffer to `entry.path/` and refreshes entries (drill).
/// - **File**: sets buffer to `entry.path` and closes the picker.
fn accept_path_picker_entry(picker: &mut PathPicker, buffer: &mut String, index: usize) {
    let Some(entry) = picker.entries.get(index).cloned() else {
        return;
    };

    if entry.is_dir {
        // Drill into directory — update buffer and refresh entries inline
        let new_path = format!("{}/", entry.path);
        *buffer = new_path.clone();
        picker.base_dir = new_path.clone();
        picker.entries = list_directory(&new_path, "");
        picker.selected_index = 0;
        picker.last_query = new_path;
        picker.active = !picker.entries.is_empty();
        return;
    }

    // File selected — set buffer and close
    *buffer = entry.path.clone();
    picker.active = false;
    picker.last_query.clear();
}

/// Render a path picker popup below its associated TextEdit.
///
/// Same visual style as the `@` context picker (breadcrumb + scrollable entries).
/// `edit_id` is used to re-request focus on the TextEdit after directory drills.
fn show_path_picker_popup(
    ctx: &egui::Context,
    picker: &mut PathPicker,
    buffer: &mut String,
    popup_id_str: &str,
    edit_id: egui::Id,
) {
    let Some(rect) = picker.edit_rect else {
        return;
    };
    if picker.entries.is_empty() {
        return;
    }

    let popup_id = egui::Id::new(popup_id_str);
    let pos = egui::pos2(rect.left(), rect.bottom() + 2.0);

    egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui: &mut egui::Ui| {
                ui.set_max_width(rect.width().max(280.0));
                ui.set_max_height(200.0);

                // Breadcrumb: current directory
                ui.label(
                    egui::RichText::new(&picker.base_dir)
                        .small()
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                );
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(175.0)
                    .show(ui, |ui| {
                        for (i, entry) in picker.entries.iter().enumerate() {
                            let is_selected = i == picker.selected_index;
                            let icon = if entry.is_dir {
                                "📁"
                            } else if entry.name.ends_with(".duckdb")
                                || entry.name.contains("duckdb")
                            {
                                "🗄"
                            } else {
                                "📄"
                            };
                            let suffix = if entry.is_dir { "/" } else { "" };
                            let label = format!("{} {}{}", icon, entry.name, suffix);

                            if ui.selectable_label(is_selected, &label).clicked() {
                                accept_path_picker_entry(picker, buffer, i);
                                // Re-focus the TextEdit so the user can keep typing
                                ctx.memory_mut(|m| m.request_focus(edit_id));
                                return;
                            }
                        }
                    });
            });
        });
}

// ── Filesystem helpers ──────────────────────────────────────────────────────

/// Classify a filesystem path into an attachment kind.
fn classify_path(path: &std::path::Path) -> AttachmentKind {
    if path.is_dir() {
        AttachmentKind::Folder
    } else if path.extension().map_or(false, |e| e == "duckdb") {
        AttachmentKind::Database
    } else {
        AttachmentKind::File
    }
}

/// List filesystem entries in `dir`, filtered by `name_filter` (case-insensitive prefix).
///
/// Returns up to 20 entries, directories first, then alphabetically.
/// Hidden files (starting with `.`) are skipped unless `name_filter` starts with `.`.
fn list_directory(dir: &str, name_filter: &str) -> Vec<FsEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let filter_lower = name_filter.to_lowercase();
    let mut results: Vec<FsEntry> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            // Skip hidden files unless the filter explicitly starts with '.'
            if name.starts_with('.') && !filter_lower.starts_with('.') {
                return None;
            }
            // Case-insensitive prefix match
            if !name_filter.is_empty() && !name.to_lowercase().starts_with(&filter_lower) {
                return None;
            }
            Some(FsEntry {
                path: path.to_string_lossy().to_string(),
                name,
                is_dir: path.is_dir(),
            })
        })
        .collect();

    // Sort: directories first, then alphabetical
    results.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

    // Limit to 20 entries for UI performance
    results.truncate(20);
    results
}

/// Attempt terminal-style tab-completion on a path buffer.
///
/// Splits the text at the last `/` into (parent_dir, partial_name), lists the
/// parent directory, and:
/// - **Single match** → replaces buffer with full path (appends `/` for dirs).
/// - **Multiple matches** → extends buffer to the longest common prefix.
/// - **No matches** → leaves buffer unchanged.
///
/// Returns `true` if the buffer was modified.
fn tab_complete_path(buffer: &mut String) -> bool {
    let trimmed = buffer.trim_end();
    if trimmed.is_empty() {
        return false;
    }

    // Expand ~ to $HOME
    let expanded = if trimmed.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
        trimmed.replacen('~', &home, 1)
    } else {
        trimmed.to_owned()
    };

    let exp_path = std::path::Path::new(&expanded);

    // If the text exactly matches an existing directory, append /
    if exp_path.is_dir() && !expanded.ends_with('/') {
        *buffer = format!("{}/", expanded);
        return true;
    }

    // Split into (parent_dir, partial_name)
    let (dir, partial) = if expanded.ends_with('/') {
        // Trailing / → list that directory with no name filter
        (expanded.as_str(), "")
    } else if let Some(pos) = expanded.rfind('/') {
        (&expanded[..=pos], &expanded[pos + 1..])
    } else {
        // No slash — can't tab-complete a bare name here
        return false;
    };

    if !std::path::Path::new(dir).is_dir() {
        return false;
    }

    let entries = list_directory(dir, partial);
    if entries.is_empty() {
        return false;
    }

    if entries.len() == 1 {
        // Single match → complete to full path
        let entry = &entries[0];
        let suffix = if entry.is_dir { "/" } else { "" };
        *buffer = format!("{}{}", entry.path, suffix);
        return true;
    }

    // Multiple matches → extend to longest common prefix
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let common = longest_common_prefix(&names);
    if common.len() > partial.len() {
        *buffer = format!("{}{}", dir, common);
        return true;
    }

    false
}

/// Find the longest common prefix among a slice of strings.
fn longest_common_prefix(strings: &[&str]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = strings[0];
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        for (i, (a, b)) in first.bytes().zip(s.bytes()).enumerate() {
            if i >= len {
                break;
            }
            if a != b {
                len = i;
                break;
            }
        }
    }
    first[..len].to_string()
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

/// The embedded chat panel as an [`EventSink`]: each method is the verbatim body
/// of the corresponding former `poll_events` match arm, so behavior is unchanged —
/// `poll_events` now delegates here via `apply_agent_event`. The embedded
/// screenshot reply is delivered by core.rs through `ui_command_tx`, so this sink
/// does not use `reply_tx`.
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
        self.scroll_to_bottom = true;
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

        // Embed highlights in the Analysis message as clickable chips.
        self.messages.push(ChatMessage {
            kind: ChatMessageKind::Analysis,
            text: display,
            highlights,
            expandable_content: None,
        });
        self.scroll_to_bottom = true;

        self.add_message(
            ChatMessageKind::System,
            format!(
                "Done. {} quer{} executed.",
                response.queries_executed,
                if response.queries_executed == 1 { "y" } else { "ies" }
            ),
        );
        self.pending_request = false;
        *self.event_rx.lock().unwrap() = None;
    }

    fn on_error(&mut self, error: String) {
        self.add_message(ChatMessageKind::System, format!("Error: {error}"));
        self.pending_request = false;
        *self.event_rx.lock().unwrap() = None;
    }
}
