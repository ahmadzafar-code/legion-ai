//! Chat panel UI for AI-powered gap diagnosis.
//!
//! Provides a Cursor-style toggleable right-side panel where users can
//! select a timeline region and ask about idle gaps. The panel displays
//! messages (system, user, analysis, context) and a text input field.
//!
//! When built with `--features ai`, the panel connects to a Python sidecar
//! (FastAPI + Claude API + DuckDB tool) via HTTP for LLM-powered diagnosis.

use crate::data::EntryID;
use crate::timestamp::Interval;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// Shared async response channel type (avoids clippy::type_complexity).
type ResponseChannel = Arc<Mutex<Option<mpsc::Receiver<Result<String, String>>>>>;

/// The kind of chat message, controlling rendering style.
#[derive(Clone, Debug)]
pub enum ChatMessageKind {
    /// Gray italic — system status messages
    System,
    /// Right-aligned — user input
    User,
    /// Left-aligned monospace — diagnosis results
    Analysis,
    /// Compact badge — selection context
    Context,
}

/// A single message in the chat panel.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub kind: ChatMessageKind,
    pub text: String,
}

/// A user's selection on the timeline (like selected code lines in Cursor).
#[derive(Clone, Debug)]
pub struct TimelineSelection {
    pub entry_id: EntryID,
    /// Human-readable label: "CPU Proc 2" or "n0_cpu_c2"
    pub entry_label: String,
    /// The gap time range
    pub interval: Interval,
}

/// The chat panel state and UI.
pub struct ChatPanel {
    pub visible: bool,
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub selection: Option<TimelineSelection>,
    scroll_to_bottom: bool,

    // --- Sidecar connection state ---
    sidecar_url: String,
    sidecar_status: String,
    duckdb_path_buffer: String,
    pending_request: bool,
    /// Channel for receiving async HTTP responses from background thread.
    response_rx: ResponseChannel,
    /// Future: path to user's application code for deeper analysis.
    code_path_buffer: String,
}

impl Clone for ChatPanel {
    fn clone(&self) -> Self {
        Self {
            visible: self.visible,
            messages: self.messages.clone(),
            input_buffer: self.input_buffer.clone(),
            selection: self.selection.clone(),
            scroll_to_bottom: self.scroll_to_bottom,
            sidecar_url: self.sidecar_url.clone(),
            sidecar_status: self.sidecar_status.clone(),
            duckdb_path_buffer: self.duckdb_path_buffer.clone(),
            pending_request: self.pending_request,
            response_rx: Arc::clone(&self.response_rx),
            code_path_buffer: self.code_path_buffer.clone(),
        }
    }
}

impl std::fmt::Debug for ChatPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatPanel")
            .field("visible", &self.visible)
            .field("messages", &self.messages.len())
            .field("selection", &self.selection)
            .field("sidecar_url", &self.sidecar_url)
            .field("sidecar_status", &self.sidecar_status)
            .field("pending_request", &self.pending_request)
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
                text: "Select a region on the timeline (Shift+click a gap) and ask about idle gaps."
                    .into(),
            }],
            input_buffer: String::new(),
            selection: None,
            scroll_to_bottom: false,
            sidecar_url: "http://localhost:8420".into(),
            sidecar_status: "Not connected".into(),
            duckdb_path_buffer: String::new(),
            pending_request: false,
            response_rx: Arc::new(Mutex::new(None)),
            code_path_buffer: String::new(),
        }
    }

    /// Toggle panel visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Add a message to the chat panel.
    pub fn add_message(&mut self, kind: ChatMessageKind, text: impl Into<String>) {
        self.messages.push(ChatMessage {
            kind,
            text: text.into(),
        });
        self.scroll_to_bottom = true;
    }

    /// Update the timeline selection and add a context message.
    pub fn set_selection(&mut self, sel: TimelineSelection) {
        let context_msg = format!(
            "Selected: {} | {} → {} ({})",
            sel.entry_label,
            sel.interval.start,
            sel.interval.stop,
            format_duration_ns(sel.interval.duration_ns()),
        );
        self.selection = Some(sel);
        self.add_message(ChatMessageKind::Context, context_msg);
    }

    /// Clear the current timeline selection.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Poll for a completed sidecar response (non-blocking).
    ///
    /// Two-phase approach: first collect the result under the lock, then drop
    /// the lock before mutating `self` (adding messages, clearing state).
    fn poll_response(&mut self) {
        // Phase 1: try_recv under the lock, collect result into a local
        let result: Option<Result<String, String>> = {
            let guard = self.response_rx.lock().unwrap();
            if let Some(rx) = guard.as_ref() {
                match rx.try_recv() {
                    Ok(msg) => Some(msg),
                    Err(mpsc::TryRecvError::Empty) => None, // Still waiting
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Response channel disconnected.".into()))
                    }
                }
            } else {
                None
            }
            // guard dropped here
        };

        // Phase 2: act on the result (no lock held, free to call &mut self)
        if let Some(msg) = result {
            match msg {
                Ok(analysis) => {
                    let display = if analysis.len() > 10_000 {
                        format!(
                            "{}…\n\n(truncated — full response was {} chars)",
                            &analysis[..10_000],
                            analysis.len()
                        )
                    } else {
                        analysis
                    };
                    self.add_message(ChatMessageKind::Analysis, display);
                }
                Err(error) => {
                    self.add_message(
                        ChatMessageKind::System,
                        format!("Error: {}", error),
                    );
                }
            }
            self.pending_request = false;
            *self.response_rx.lock().unwrap() = None;
        }
    }

    /// Trigger an analysis request to the sidecar.
    fn trigger_diagnosis(&mut self, user_query: Option<String>) {
        if self.duckdb_path_buffer.is_empty() {
            self.add_message(
                ChatMessageKind::System,
                "Enter the path to the .duckdb file first.",
            );
            return;
        }

        if self.pending_request {
            self.add_message(
                ChatMessageKind::System,
                "A request is already in progress. Please wait.",
            );
            return;
        }

        let query_text = user_query.unwrap_or_else(|| "Find performance issues".to_string());
        self.add_message(ChatMessageKind::User, &query_text);
        self.add_message(ChatMessageKind::System, "Analyzing… (Claude is querying the database, this may take 30-90s)");
        self.pending_request = true;

        // Build request body
        let url = format!("{}/diagnose", self.sidecar_url);
        let mut body = serde_json::json!({
            "duckdb_path": self.duckdb_path_buffer,
            "query": query_text,
        });
        if !self.code_path_buffer.is_empty() {
            body["code_path"] = serde_json::Value::String(self.code_path_buffer.clone());
        }

        // Spawn background thread for HTTP request
        let (tx, rx) = mpsc::channel();
        *self.response_rx.lock().unwrap() = Some(rx);

        std::thread::spawn(move || {
            let result = ureq::post(&url)
                .timeout(std::time::Duration::from_secs(300))
                .set("Content-Type", "application/json")
                .send_string(&body.to_string());

            match result {
                Ok(resp) => {
                    let body_str = resp.into_string().unwrap_or_default();
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body_str) {
                        // Check for error field
                        if let Some(error) = parsed.get("error").and_then(|e| e.as_str()) {
                            if !error.is_empty() {
                                let _ = tx.send(Err(error.to_string()));
                                return;
                            }
                        }
                        if let Some(analysis) = parsed.get("analysis").and_then(|a| a.as_str()) {
                            let _ = tx.send(Ok(analysis.to_string()));
                        } else {
                            let _ = tx.send(Err("No 'analysis' field in response".into()));
                        }
                    } else {
                        let _ = tx.send(Err(format!("Invalid JSON response: {}", body_str)));
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(format!(
                        "Sidecar request failed: {}. Is the sidecar running?\n\
                         Start it with: cd sidecar && python server.py",
                        e
                    )));
                }
            }
        });
    }

    /// Render the chat panel. Must be called BEFORE CentralPanel in the layout.
    pub fn show(&mut self, ctx: &egui::Context) {
        // Poll for pending sidecar response
        self.poll_response();

        egui::SidePanel::right("ai_chat_panel")
            .resizable(true)
            .default_width(400.0)
            .min_width(300.0)
            .frame(egui::Frame::side_top_panel(ctx.style().as_ref())
                .fill(egui::Color32::from_rgb(250, 250, 250)))
            .show_animated(ctx, self.visible, |ui| {
                // Override text defaults to dark for this panel
                ui.visuals_mut().override_text_color = Some(egui::Color32::from_rgb(30, 30, 30));
                // 1. Header
                ui.horizontal(|ui| {
                    ui.heading("AI Chat");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            self.visible = false;
                        }
                    });
                });
                ui.separator();

                // 2. Sidecar connection UI
                ui.horizontal(|ui| {
                    ui.label("Sidecar:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sidecar_url)
                            .hint_text("http://localhost:8420")
                            .desired_width(160.0),
                    );
                    if ui.button("Check").clicked() {
                        let health_url = format!("{}/health", self.sidecar_url);
                        match ureq::get(&health_url)
                            .timeout(std::time::Duration::from_secs(5))
                            .call()
                        {
                            Ok(_resp) => {
                                self.sidecar_status = "✅ Connected".into();
                                self.add_message(
                                    ChatMessageKind::System,
                                    "Sidecar connected.",
                                );
                            }
                            Err(e) => {
                                self.sidecar_status = format!("❌ {}", e);
                            }
                        }
                    }
                });

                // DuckDB path
                ui.horizontal(|ui| {
                    ui.label("DB:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.duckdb_path_buffer)
                            .hint_text("/path/to/legion_prof.duckdb")
                            .desired_width(220.0),
                    );
                });

                ui.label(egui::RichText::new(&self.sidecar_status).small());
                ui.separator();

                // 3. Quick action button
                ui.horizontal(|ui| {
                    let btn = ui.add_enabled(
                        !self.pending_request,
                        egui::Button::new("🔍 Find Performance Issues"),
                    );
                    if btn.clicked() {
                        self.trigger_diagnosis(None);
                    }
                });
                ui.separator();

                // 4. Loading indicator
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

                // 5. Message scroll area
                let available_height = ui.available_height() - 40.0; // Reserve space for input
                egui::ScrollArea::vertical()
                    .max_height(available_height)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for msg in &self.messages {
                            match &msg.kind {
                                ChatMessageKind::System => {
                                    ui.label(
                                        egui::RichText::new(&msg.text)
                                            .italics()
                                            .color(egui::Color32::from_rgb(120, 120, 120)),
                                    );
                                }
                                ChatMessageKind::User => {
                                    // User message in a blue bubble
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::TOP),
                                        |ui| {
                                            egui::Frame::none()
                                                .fill(egui::Color32::from_rgb(59, 130, 246))
                                                .rounding(8.0)
                                                .inner_margin(egui::Margin::symmetric(10.0, 6.0))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        egui::RichText::new(&msg.text)
                                                            .color(egui::Color32::WHITE),
                                                    );
                                                });
                                        },
                                    );
                                }
                                ChatMessageKind::Analysis => {
                                    render_analysis_markdown(ui, &msg.text);
                                }
                                ChatMessageKind::Context => {
                                    ui.label(
                                        egui::RichText::new(&msg.text)
                                            .small()
                                            .color(egui::Color32::from_rgb(37, 99, 235)),
                                    );
                                }
                            }
                            ui.add_space(4.0);
                        }
                    });

                // 6. Input field at bottom
                ui.separator();
                let response = ui.horizontal(|ui| {
                    let enabled = !self.pending_request;
                    ui.add_enabled_ui(enabled, |ui| {
                        let r = ui.text_edit_singleline(&mut self.input_buffer);
                        ui.button("⏎").clicked()
                            || (r.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    })
                    .inner
                });

                if response.inner && !self.input_buffer.is_empty() && !self.pending_request {
                    let text = std::mem::take(&mut self.input_buffer);
                    self.trigger_diagnosis(Some(text));
                }

                // 7. Settings (collapsed by default)
                ui.add_space(8.0);
                ui.collapsing("Settings", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Code path:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.code_path_buffer)
                                .hint_text("/path/to/your/legion/app/src")
                                .desired_width(200.0),
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "Give the AI access to your application source code for deeper analysis",
                        )
                        .small()
                        .italics(),
                    );
                });
            });

        // Request repaint while waiting for response so we poll promptly
        if self.pending_request {
            ctx.request_repaint();
        }
    }
}

/// Render analysis text with basic markdown formatting.
///
/// Supports: `## headings`, `**bold**`, `- bullets`, `| tables` (monospace),
/// and blank-line paragraph breaks. Everything renders in dark text for readability.
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

        // ## Heading
        if let Some(heading) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(heading)
                    .strong()
                    .size(14.0)
                    .color(heading_color),
            );
            ui.add_space(2.0);
            continue;
        }

        // ### Sub-heading
        if let Some(heading) = trimmed.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(heading)
                    .strong()
                    .size(13.0)
                    .color(heading_color),
            );
            ui.add_space(1.0);
            continue;
        }

        // # Top-level heading
        if let Some(heading) = trimmed.strip_prefix("# ") {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(heading)
                    .strong()
                    .size(15.0)
                    .color(heading_color),
            );
            ui.add_space(3.0);
            continue;
        }

        // | table row — render monospace
        if trimmed.starts_with('|') {
            // Skip separator rows like |---|---|
            if trimmed.contains("---") {
                continue;
            }
            ui.label(
                egui::RichText::new(trimmed)
                    .monospace()
                    .size(11.0)
                    .color(dark),
            );
            continue;
        }

        // - bullet or * bullet
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let bullet_text = &trimmed[2..];
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("  •").color(muted));
                render_inline_markdown(ui, bullet_text, dark);
            });
            continue;
        }

        // Numbered list: 1. item
        if trimmed.len() > 2
            && trimmed.as_bytes()[0].is_ascii_digit()
            && trimmed.contains(". ")
        {
            if let Some(pos) = trimmed.find(". ") {
                let number = &trimmed[..pos + 1];
                let rest = &trimmed[pos + 2..];
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(format!("  {}", number)).color(muted));
                    render_inline_markdown(ui, rest, dark);
                });
                continue;
            }
        }

        // Regular paragraph text
        ui.horizontal_wrapped(|ui| {
            render_inline_markdown(ui, trimmed, dark);
        });
    }
}

/// Render a single line with inline **bold** spans.
fn render_inline_markdown(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    let mut remaining = text;
    while let Some(start) = remaining.find("**") {
        // Text before the bold marker
        let before = &remaining[..start];
        if !before.is_empty() {
            ui.label(egui::RichText::new(before).color(color));
        }
        remaining = &remaining[start + 2..];
        // Find closing **
        if let Some(end) = remaining.find("**") {
            let bold_text = &remaining[..end];
            ui.label(egui::RichText::new(bold_text).strong().color(color));
            remaining = &remaining[end + 2..];
        } else {
            // No closing **, just render the rest with the **
            ui.label(egui::RichText::new(format!("**{}", remaining)).color(color));
            return;
        }
    }
    if !remaining.is_empty() {
        ui.label(egui::RichText::new(remaining).color(color));
    }
}

/// Format a nanosecond duration into a human-readable string.
fn format_duration_ns(ns: i64) -> String {
    if ns < 1_000 {
        format!("{} ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.1} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.3} s", ns as f64 / 1_000_000_000.0)
    }
}
