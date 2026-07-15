//! Legion AI additions to the viewer core (`app::core`).
//!
//! This is a CHILD module of `core` — the one Rust arrangement that lets the
//! fork's code use `core`'s private types and fields (`Context`, `Config`,
//! `Window`, `Slot`, `ProfApp`) without widening their visibility. Everything
//! the fork adds to `core.rs` lives here; the upstream file keeps only thin
//! `#[cfg(feature = "ai")]`-gated calls into this module, so merges from
//! StanfordLegion/prof-viewer touch at most a handful of one-line seams.
//! See docs/UPSTREAM-DELTA.md for the full fork map.
//!
//! Contents, top to bottom:
//! - [`PersistedAiSettings`] — the panel settings that survive restarts
//! - `Context::ui_bridge` — mints the second (in-viewer MCP) event bridge
//! - startup + per-frame stages called from `ProfApp::new` / `update()`
//!   (`init_app`, `frame_services`, `apply_panel_actions`,
//!   `screenshot_and_mcp_drain`, `sync_item_selection`, `screenshot_watchdog`)
//! - `Slot` overlay painting + gap-click selection
//! - `Window` kind-filter / highlight-manager methods
//! - the highlight/navigation helper layer shared by the embedded agent and
//!   the in-viewer MCP server (`apply_navigation`, `McpDrainSink`, …)
//! - unit tests for the pure pieces

use super::*;

/// AI-panel settings persisted across app restarts via eframe storage.
/// The `ChatPanel` itself is `serde(skip)` (channels, sessions, caches), so this
/// small plain-data mirror carries the values worth keeping. The API key is
/// deliberately absent — eframe storage is plaintext on disk. Empty strings mean
/// "unset" (`apply_persisted` skips them, so CLI flags / defaults survive).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PersistedAiSettings {
    #[serde(default)]
    pub project_root: String,
    #[serde(default)]
    pub duckdb_path: String,
    #[serde(default)]
    pub wiki_path: String,
    /// Composer model picker ("", "opus", "sonnet", "haiku").
    #[serde(default)]
    pub model: String,
    /// Composer strength picker ("", "low", "medium", "high", "max").
    #[serde(default)]
    pub effort: String,
}

impl Context {
    /// Mint a [`UiBridge`](crate::ai::bridge::UiBridge) for a second consumer
    /// (the in-viewer MCP server thread) bound to `consumer_id`. Creates the
    /// second event/command channel pair, stores the UI-side ends so the
    /// per-frame loop drains and replies on them, and hands the consumer-side
    /// ends + a clone of the shared viewport token to the bridge. The embedded
    /// chat agent is unaffected; the bridge's `request` is structurally locked
    /// out via the token while another consumer owns the viewport.
    // dead_code: unused in {ai}-without-viewer-mcp builds (the only caller is
    // the server spawn, which is viewer-mcp-gated).
    #[allow(dead_code)]
    pub fn ui_bridge(&mut self, consumer_id: u64) -> crate::ai::bridge::UiBridge {
        let (event_tx, event_rx) = std::sync::mpsc::channel::<crate::ai::AgentEvent>();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<crate::ai::UiCommand>();
        *self.mcp_event_rx.lock().unwrap() = Some(event_rx);
        self.mcp_cmd_tx = Some(cmd_tx);
        crate::ai::bridge::UiBridge::new(event_tx, cmd_rx, self.viewport_token.clone(), consumer_id)
    }
}

/// AI-side startup for [`ProfApp::new`]: restore persisted panel settings,
/// apply CLI tool paths, and honor the opt-in span-trace env var. (Session
/// reasoning transcripts are separate and ON by default — see `ai/trace.rs`.)
pub(super) fn init_app(result: &mut ProfApp, opts: StartOptions) {
    // Restore last session's AI settings FIRST (project folder, DB path,
    // wiki path), then let explicit CLI flags overwrite — set_tool_paths
    // only writes non-empty values.
    let saved = result.cx.ai_settings.clone();
    result.cx.chat_panel.apply_persisted(&saved);
    // Pre-fill the assistant's tool paths from CLI flags / auto-detection.
    result
        .cx
        .chat_panel
        .set_tool_paths(opts.ai_duckdb_path, opts.ai_code_path, opts.ai_wiki_path);

    // Span-subscriber tracing (this block) is OPT-IN: set
    // LEGION_PROF_AI_TRACE_DIR to a directory to record one JSON line per
    // agent span under <dir>/agent_traces/agent.jsonl. SEPARATE mechanism:
    // session reasoning transcripts are ON by default in
    // ~/.legion_prof_viewer/traces/ (LEGION_PROF_AI_TRACE=off disables) —
    // see `ai/trace.rs::SessionTrace`.
    if let Ok(dir) = std::env::var("LEGION_PROF_AI_TRACE_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            match crate::ai::trace::init_subscriber(std::path::Path::new(dir)) {
                Ok(()) => eprintln!("Agent traces: {dir}/agent_traces/agent.jsonl"),
                Err(e) => eprintln!("Failed to initialize agent tracing: {e}"),
            }
        }
    }
}

/// Per-frame service stage from `update()`: hand the chat panel the shared
/// viewport token, and (viewer-mcp builds) start the in-viewer HTTP MCP server
/// once a DuckDB path is configured.
pub(super) fn frame_services(ctx: &egui::Context, cx: &mut Context) {
    // `ctx` feeds the server's wake hook, which exists only in viewer-mcp builds.
    #[cfg(not(feature = "viewer-mcp"))]
    let _ = ctx;
    // Hand the embedded chat agent a clone of the shared viewport token so
    // its screenshot/nav round-trips are mutually exclusive with the in-viewer
    // MCP driver (single outstanding screenshot across both). Idempotent; no
    // effect on the sole-driver path (the token is always free for it).
    cx.chat_panel
        .ensure_viewport_token(cx.viewport_token.clone());

    // Start the in-viewer HTTP MCP server once a DuckDB path is configured.
    // Runs on its OWN thread — never the egui main thread. One spawn attempt;
    // serves the data, source, wiki, and visual tools over HTTP so Claude Code
    // can connect to this live process.
    #[cfg(feature = "viewer-mcp")]
    if !cx.viewer_mcp_started {
        if let Some(duckdb_path) = cx.chat_panel.duckdb_path() {
            // Mint a UiBridge (consumer MCP_CONSUMER_ID) and hand it to the
            // server so it advertises + routes the 9 VISUAL tools, driving this
            // live window. The bridge's UI-side ends (mcp_event_rx / mcp_cmd_tx)
            // are drained + replied to by the per-frame second-source loop below.
            // The wake hook repaints this (reactive, often-idle) window when a
            // request arrives, so the drain loop runs instead of blocking to
            // timeout.
            let egui_ctx = ctx.clone();
            let bridge = cx
                .ui_bridge(crate::ai::bridge::MCP_CONSUMER_ID)
                .with_wake(move || egui_ctx.request_repaint());
            // Hand the configured wiki root + the LIVE project-root handle to
            // the server (the handle is read per request, so a folder set in
            // the panel at ANY time reaches instructions/read_code — a
            // snapshot at spawn would silently ignore late-set paths forever).
            let wiki_root = cx.chat_panel.wiki_path();
            let code_root = cx.chat_panel.project_root_handle();
            // Prefer the stable well-known port 8765 so existing external
            // `claude mcp add …:8765/mcp` registrations keep working; fall back
            // to an ephemeral port (0) only if 8765 is already taken. Either
            // way, the REAL bound port lands in the chat panel, so the Claude
            // Code backend never assumes a port.
            // The spawn also mints the per-session bearer token every POST /mcp
            // must present (server hardening); the (port, token) pair flows to
            // the chat panel so the Claude Code backend can build its
            // --mcp-config.
            let mut endpoint: Option<(u16, String)> = None;
            // The spawn also returns the ApprovalBroker behind POST
            // /approve — handed to the chat panel, which renders the
            // Deny/Allow/Always-allow dialog for hook-gated tool calls.
            let mut approval_broker = None;
            match crate::ai::viewer_mcp::spawn(
                duckdb_path.clone(),
                crate::ai::viewer_mcp::DEFAULT_MCP_PORT,
                bridge,
                wiki_root.clone(),
                code_root.clone(),
            ) {
                Ok((port, token, broker)) => {
                    endpoint = Some((port, token));
                    approval_broker = Some(broker);
                }
                Err(first_err) => {
                    let egui_ctx2 = ctx.clone();
                    let bridge2 = cx
                        .ui_bridge(crate::ai::bridge::MCP_CONSUMER_ID)
                        .with_wake(move || egui_ctx2.request_repaint());
                    match crate::ai::viewer_mcp::spawn(
                        duckdb_path,
                        0,
                        bridge2,
                        wiki_root,
                        code_root,
                    ) {
                        Ok((port, token, broker)) => {
                            eprintln!(
                                "[legion-viewer] port 8765 unavailable ({first_err}); using ephemeral port {port}"
                            );
                            endpoint = Some((port, token));
                            approval_broker = Some(broker);
                        }
                        Err(e) => eprintln!(
                            "[legion-viewer] in-viewer MCP server failed to start: \
                                 port 8765: {first_err}; ephemeral: {e}"
                        ),
                    }
                }
            }
            cx.chat_panel.set_mcp_endpoint(endpoint);
            cx.chat_panel.set_approval_broker(approval_broker);
            cx.viewer_mcp_started = true;
        }
    }
}

/// Move a completed gap/timeline selection from the window into the chat
/// panel (invoked inside `update()`'s per-window loop).
pub(super) fn take_timeline_selection(cx: &mut Context, window: &mut Window) {
    if let Some((entry_id, interval, label)) = window.config.ai_timeline_selection.take() {
        cx.chat_panel.set_selection(crate::ai::TimelineSelection {
            entry_id,
            entry_label: label,
            interval,
        });
        // A2: record the selection but do NOT auto-open the chat panel —
        // the header "Selected:" banner surfaces it instead. (An already-
        // open panel still updates its pill via set_selection above.)
    }
}

/// The right-aligned "Legion AI" toggle button in the top menu bar.
pub(super) fn toggle_button(ui: &mut egui::Ui, cx: &mut Context) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Gap so the button doesn't sit flush against the window edge
        // (right_to_left => this space reserves the right margin).
        ui.add_space(12.0);
        // Filled Legion red while the panel is open, grey otherwise
        // (#EC3937 is the flat fill of the Legion brick logo).
        // Instead of a single fixed `.fill()` (which reads as a static
        // label), drive the fill from egui's per-state widget visuals
        // so it visibly LIGHTENS on hover and DARKENS on press, plus a
        // subtle raise — the affordance that says "clickable".
        let open = cx.chat_panel.visible;
        let (base, hover, press) = if open {
            (
                egui::Color32::from_rgb(236, 57, 55),
                egui::Color32::from_rgb(248, 92, 90),
                egui::Color32::from_rgb(205, 44, 42),
            )
        } else {
            (
                egui::Color32::from_rgb(232, 234, 238),
                egui::Color32::from_rgb(214, 218, 226),
                egui::Color32::from_rgb(198, 202, 212),
            )
        };
        let border = egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 185, 196));
        {
            let w = &mut ui.visuals_mut().widgets;
            for (st, fill) in [
                (&mut w.inactive, base),
                (&mut w.hovered, hover),
                (&mut w.active, press),
            ] {
                st.weak_bg_fill = fill;
                st.bg_fill = fill;
                st.bg_stroke = border;
            }
            // A 1px raise on hover reinforces "pressable".
            w.hovered.expansion = 1.0;
        }
        let text_color = if open {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(30, 30, 30)
        };
        let label = egui::RichText::new("Legion AI")
            .strong()
            .size(15.5)
            .color(text_color);
        if ui
            .add(
                egui::Button::new(label)
                    .rounding(0.0)
                    // Wider min-size so the red band spreads well past
                    // the text instead of hugging it.
                    .min_size(egui::vec2(170.0, 30.0)),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Toggle Legion AI")
            .clicked()
        {
            cx.chat_panel.toggle();
        }
    });
}

/// The compact header "Selected:" line under the menu bar (shown only when
/// something is selected — no empty chrome otherwise).
pub(super) fn banner_row(ui: &mut egui::Ui, selection_banner: &Option<String>) {
    if let Some(banner) = selection_banner {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(banner)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(59, 130, 246)),
            );
        });
    }
}

/// Resolve user-initiated chat-panel actions (highlight chips, clear buttons)
/// into timeline overlays and selection state.
pub(super) fn apply_panel_actions(cx: &mut Context, windows: &mut [Window]) {
    // Clear all highlight overlays if requested (Clear button or agent tool).
    if cx.chat_panel.take_clear_highlights() {
        for window in windows.iter_mut() {
            window.config.ai_highlights.clear();
        }
    }

    // Clear the active selection (✕ in the composer): deselect bars + region.
    if cx.chat_panel.take_clear_selection() {
        for window in windows.iter_mut() {
            window.config.items_selected.clear();
        }
        cx.ai_region_selection = None;
        cx.last_item_selection.clear();
    }

    let actions = cx.chat_panel.take_pending_highlight_actions();
    if !actions.is_empty() {
        let mut first_entry: Option<EntryID> = None;
        for window in windows.iter_mut() {
            let slug_map = build_slug_map(window);
            for action in &actions {
                if let Some(entry_id) = slug_map.get(&action.highlight.entry_slug) {
                    // Expand the row's ancestors so the highlight actually
                    // draws — kind panels (level 2) are collapsed by default,
                    // and a highlight on a hidden row renders nothing.
                    window.expand_slot(entry_id);
                    if first_entry.is_none() {
                        first_entry = Some(entry_id.clone());
                    }
                    let ai_hl = highlight_to_ai(&action.highlight);
                    let entry = window
                        .config
                        .ai_highlights
                        .entry(entry_id.clone())
                        .or_default();
                    // Dedup: don't stack an identical highlight (same range + label).
                    let dup = entry.iter().any(|h| {
                        h.interval.start.0 == ai_hl.interval.start.0
                            && h.interval.stop.0 == ai_hl.interval.stop.0
                            && h.label == ai_hl.label
                    });
                    if !dup {
                        entry.push(ai_hl);
                    }
                } else {
                    log::warn!(
                        "Highlight: unknown entry_slug '{}'",
                        action.highlight.entry_slug
                    );
                }
            }
            if !window.config.ai_highlights.is_empty() {
                window.config.ai_highlights_enabled = true;
            }
        }
        // Scroll vertically to the first highlighted row so the overlays are
        // on screen (the rows we just expanded may be below the fold).
        if let Some(entry_id) = first_entry {
            cx.ai_scroll_to_entry = Some(entry_id);
        }
        // Zoom to fit ALL highlights that requested it (union of ranges),
        // so "Zoom to all" frames every chip, and a single "Show ▸" frames
        // just that one.
        let zoom: Vec<_> = actions.iter().filter(|a| a.zoom_to).collect();
        if !zoom.is_empty() {
            let start = zoom.iter().map(|a| a.highlight.start_ns).min().unwrap();
            let stop = zoom.iter().map(|a| a.highlight.stop_ns).max().unwrap();
            let interval = Interval::new(Timestamp(start), Timestamp(stop));
            // Pad so the highlighted span sits inside the view with a margin.
            let pad = (interval.duration_ns() / 10).max(1_000);
            ProfApp::zoom(cx, interval.grow(pad));
        }
    }
}

/// Screenshot capture pipeline (agent thread <-> UI thread) plus the second-
/// source drain for the in-viewer MCP bridge.
///
/// Handle screenshot capture pipeline: agent thread ←→ UI thread.
/// Phase 1: deliver completed screenshots (from a previous frame's
/// ViewportCommand::Screenshot) back to the blocked agent.
/// Phase 2: consume new screenshot/zoom requests emitted by the agent
/// (set by poll_events() during chat_panel.show() above).
pub(super) fn screenshot_and_mcp_drain(
    ctx: &egui::Context,
    cx: &mut Context,
    windows: &mut [Window],
) {
    // Phase 1: Check for Event::Screenshot delivered by egui.
    // Extract data inside ctx.input() closure, send outside to avoid
    // capturing &mut cx across the send call.
    // Embedded slot is checked FIRST; the second source's slot only if
    // the embedded one is empty, so the single egui screenshot pipeline
    // serves whichever source is currently active.
    let captured: Option<(u64, Vec<u8>, bool)> = ctx.input(|i| {
        for event in &i.events {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(request_id) = cx.awaiting_screenshot.take() {
                    return Some((request_id, encode_screenshot_png(image), false));
                }
                if let Some((request_id, _, _)) = &cx.mcp_awaiting_screenshot {
                    return Some((*request_id, encode_screenshot_png(image), true));
                }
            }
        }
        None
    });
    if let Some((request_id, png_bytes, is_mcp)) = captured {
        let metadata = build_screenshot_metadata(cx, windows);
        if is_mcp {
            if let Some((_, reply_tx, _)) = cx.mcp_awaiting_screenshot.take() {
                let _ = reply_tx.send(crate::ai::UiCommand::ScreenshotData {
                    request_id,
                    png_bytes,
                    metadata,
                });
            }
        } else {
            cx.chat_panel
                .send_screenshot(request_id, png_bytes, metadata);
        }
    }

    // Phase 2: Check for new navigation requests from the agent thread.
    if let Some(nav) = cx.chat_panel.take_pending_navigation() {
        // Embedded source: apply the view change via the SHARED handler
        // (`apply_navigation`, reused by the second source below), then
        // request the screenshot.
        let request_id = pending_nav_request_id(&nav);
        apply_navigation(cx, windows, &nav);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        cx.awaiting_screenshot = Some(request_id);
    }

    // Second source (the in-viewer MCP bridge): drain the MCP event channel and
    // service ONE navigation this frame, replying on its OWN channel. Empty
    // until a `UiBridge` is minted, so this is dormant for the embedded
    // agent. Only runs when the screenshot pipeline is free this frame.
    if cx.awaiting_screenshot.is_none() && cx.mcp_awaiting_screenshot.is_none() {
        let mut sink = McpDrainSink::default();
        {
            let guard = cx.mcp_event_rx.lock().unwrap();
            if let (Some(rx), Some(reply_tx)) = (guard.as_ref(), cx.mcp_cmd_tx.clone()) {
                crate::ai::bridge::drain_source(rx, &reply_tx, &mut sink);
            }
        }
        // Navigation / screenshot: drive the view + capture, reply with the PNG.
        if let Some((nav, reply_tx)) = sink.pending {
            let request_id = pending_nav_request_id(&nav);
            apply_navigation(cx, windows, &nav);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            // Watchdog deadline > the bridge's request timeout, so the client
            // sees its own timeout first; this only frees a slot egui somehow
            // never fulfilled.
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            cx.mcp_awaiting_screenshot = Some((request_id, reply_tx, deadline));
        }
        // Highlight: apply to the SAME shared state the embedded path writes,
        // scroll to it, ACK (no screenshot — mirrors the embedded text result).
        if let Some((hl, request_id, reply_tx)) = sink.pending_highlight {
            let entry = apply_one_highlight(windows, &hl);
            if let Some(entry_id) = entry {
                cx.ai_scroll_to_entry = Some(entry_id);
            }
            let message = format!(
                "Highlight added on {} [{}, {}].",
                hl.entry_slug, hl.start_ns, hl.stop_ns
            );
            let _ = reply_tx.send(crate::ai::UiCommand::Ack {
                request_id,
                message,
            });
        }
        // Clear highlights: clear the shared state, ACK the count.
        if let Some((request_id, reply_tx)) = sink.pending_clear {
            let n = clear_all_highlights(windows);
            let message = if n == 0 {
                "No highlights to clear.".to_owned()
            } else {
                format!("Cleared highlights on {n} row(s).")
            };
            let _ = reply_tx.send(crate::ai::UiCommand::Ack {
                request_id,
                message,
            });
        }
        // get_selection: a non-driving READ of the human's current
        // selection — the SAME state the embedded `build_selection_preamble`
        // reads. No viewport claim, no screenshot; reply synchronously.
        if let Some((request_id, reply_tx)) = sink.pending_selection {
            let (items, range) = cx.chat_panel.selection_snapshot();
            let _ = reply_tx.send(crate::ai::UiCommand::SelectionData {
                request_id,
                items,
                range,
            });
        }
    }
}

/// Surface the user's task-bar selection to the chat panel so the agent can
/// resolve "this task" to concrete item_uid(s)/entry_slug(s).
pub(super) fn sync_item_selection(cx: &mut Context, windows: &mut [Window]) {
    let mut snapshot: Vec<crate::ai::SelectedItem> = Vec::new();
    for window in windows.iter() {
        if window.config.items_selected.is_empty() {
            continue;
        }
        let id_to_slug: HashMap<EntryID, String> = build_slug_map(window)
            .into_iter()
            .map(|(s, id)| (id, s))
            .collect();
        for (uid, detail) in window.config.items_selected.iter().take(8) {
            let (title, start_ns, stop_ns) = match &detail.meta {
                Some(m) => (
                    m.title.clone(),
                    m.original_interval.start.0,
                    m.original_interval.stop.0,
                ),
                None => (String::new(), 0, 0),
            };
            snapshot.push(crate::ai::SelectedItem {
                item_uid: uid.0,
                entry_slug: id_to_slug.get(&detail.loc.entry_id).cloned(),
                title,
                start_ns,
                stop_ns,
            });
        }
        break;
    }
    let uids: Vec<u64> = snapshot.iter().map(|s| s.item_uid).collect();
    if uids != cx.last_item_selection {
        cx.last_item_selection = uids;
        if snapshot.is_empty() {
            cx.chat_panel.clear_item_selection();
        } else {
            cx.chat_panel.set_item_selection(snapshot);
        }
    }
}

/// Keep repainting while an in-viewer-MCP screenshot is mid-flight, with a
/// watchdog so a never-delivered capture can't become a permanent busy-loop.
///
/// While an MCP screenshot is mid-flight, keep repainting so the
/// capture frame (which delivers `Event::Screenshot`) actually happens — the
/// window is otherwise idle and would stall the request to timeout. A watchdog
/// resets the slot if egui somehow never delivers the screenshot, so this
/// never becomes a permanent busy-loop / lockout.
pub(super) fn screenshot_watchdog(ctx: &egui::Context, cx: &mut Context) {
    if let Some((_, _, deadline)) = &cx.mcp_awaiting_screenshot {
        if std::time::Instant::now() >= *deadline {
            cx.mcp_awaiting_screenshot = None; // bridge already timed out; free the slot
        } else {
            ctx.request_repaint();
        }
    }
}

/// Paint the persistent Shift+drag region-selection band over the timeline.
pub(super) fn paint_region_band(ui: &egui::Ui, cx: &Context, rect: Rect) {
    if let Some(region) = cx.ai_region_selection {
        let a = cx.view_interval.unlerp(region.start).clamp(0.0, 1.0);
        let b = cx.view_interval.unlerp(region.stop).clamp(0.0, 1.0);
        if b > a {
            let x0 = rect.left() + a * rect.width();
            let x1 = rect.left() + b * rect.width();
            let band = Rect::from_min_max(Pos2::new(x0, rect.min.y), Pos2::new(x1, rect.max.y));
            ui.painter().rect(
                band,
                0.0,
                Color32::from_rgba_unmultiplied(50, 100, 255, 30),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(50, 100, 255, 120)),
            );
        }
    }
}

/// The "Legion AI" section of the Show Controls window.
pub(super) fn show_legion_ai_controls(ui: &mut egui::Ui) {
    // Bound the content width: the window auto-sizes, and this section's
    // greedy widgets (the full-width `separator`, the table's `remainder`
    // column) would otherwise stretch it across the whole screen. Set here —
    // not in upstream's `display_controls` — so plain builds keep upstream's
    // layout untouched.
    ui.set_max_width(600.0);

    // Local replica of `display_controls`' nested row helper — that helper is
    // function-local in the upstream code, so it cannot be shared from here.
    fn show_row_ui(
        body: &mut egui_extras::TableBody<'_>,
        label: &str,
        thunk: impl FnMut(&mut egui::Ui),
    ) {
        body.row(20.0, |mut row| {
            row.col(|ui| {
                ui.strong(label);
            });
            row.col(thunk);
        });
    }

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(4.0);
    ui.strong("Legion AI");
    ui.add_space(4.0);
    TableBuilder::new(ui)
        .striped(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        // Fixed first column so every value starts at the same x. An
        // auto column let the longest labels overflow and shove their
        // values right, misaligning the table.
        .column(Column::exact(230.0))
        .column(Column::remainder())
        .body(|mut body| {
            let mut show_row = |a, b| {
                show_row_ui(&mut body, a, |ui| {
                    ui.label(b);
                });
            };
            // Timeline gestures that feed the agent context.
            show_row("Select a Task", "Click a task bar (click again to clear)");
            show_row("Select a Time Region", "Shift + Drag across the timeline");
            show_row("Select an Idle Gap", "Shift + Click on empty space");
            // Panel + composer controls.
            show_row(
                "Open / Close the Co-Pilot",
                "\"Legion AI\" button (top-right)",
            );
            show_row("Show / Hide the Sidebar", "\"Sidebar\" button (top-left)");
            show_row("Add Context", "+ menu: DuckDB, code, or a file");
            show_row("Model & Strength", "Model picker in the composer");
            show_row("Stop a Running Answer", "Stop button while a turn runs");
            show_row("New Session", "\u{21ba} in the panel header");
            show_row(
                "Ask About a Selection",
                "Select above, then ask \"what's here?\"",
            );
        });
}

impl Slot {
    /// Render AI highlight overlays FIRST (as background, behind items).
    pub(super) fn ai_paint_highlight_overlays(
        &self,
        ui: &egui::Ui,
        config: &Config,
        cx: &Context,
        rect: Rect,
    ) {
        if config.ai_highlights_enabled {
            if let Some(highlights) = config.ai_highlights.get(&self.entry_id) {
                for hl in highlights {
                    // Honor the per-highlight enable toggle (manager checkbox).
                    if !hl.enabled {
                        continue;
                    }
                    // Map interval to normalized [0,1] within view
                    let norm_start = cx.view_interval.unlerp(hl.interval.start).clamp(0.0, 1.0);
                    let norm_stop = cx.view_interval.unlerp(hl.interval.stop).clamp(0.0, 1.0);

                    // Skip if highlight is outside view
                    if norm_stop <= 0.0 || norm_start >= 1.0 {
                        continue;
                    }

                    // Full slot height rect
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let hl_rect = Rect::from_min_max(min, max);

                    // Semi-transparent red fill (very low opacity for background look)
                    let fill_color = Color32::from_rgba_unmultiplied(255, 0, 0, 40);
                    ui.painter().rect_filled(hl_rect, 0.0, fill_color);
                    ui.painter().rect_stroke(
                        hl_rect,
                        0.0,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 0, 0, 80)),
                    );
                }
            }
        }
    }

    /// Detect Shift+click on empty gap space: select the enclosing idle gap
    /// for the AI panel (items consume hover, so a live `hover_pos` here means
    /// the pointer is over gap space).
    pub(super) fn ai_gap_click_selection(
        &self,
        ui: &egui::Ui,
        config: &mut Config,
        cx: &Context,
        rect: Rect,
        hover_pos: Option<Pos2>,
        tile_id: TileID,
    ) {
        let pointer_in_rect = ui.rect_contains_pointer(rect);
        if pointer_in_rect && hover_pos.is_some() {
            // hover_pos is still Some => mouse is NOT over any item (items consume it)
            ui.input(|i| {
                if i.pointer.any_click() && i.pointer.primary_released() && i.modifiers.shift {
                    if let Some(pos) = i.pointer.hover_pos() {
                        let norm_x = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
                        let click_time = cx.view_interval.lerp(norm_x);

                        // Find the gap containing this timestamp using row 0 items
                        if let Some(Ok(tile_data)) =
                            self.tiles.get(&tile_id).and_then(|t| t.as_ref())
                        {
                            if let Some(row) = tile_data.items.first() {
                                let mut gap_interval = None;

                                // Check gaps between consecutive items
                                for window in row.windows(2) {
                                    let prev_end = window[0].interval.stop;
                                    let next_start = window[1].interval.start;
                                    if click_time >= prev_end && click_time <= next_start {
                                        gap_interval = Some(Interval::new(prev_end, next_start));
                                        break;
                                    }
                                }

                                // Check gap before first item
                                if gap_interval.is_none() {
                                    if let Some(first) = row.first() {
                                        if click_time < first.interval.start {
                                            gap_interval = Some(Interval::new(
                                                tile_id.0.start,
                                                first.interval.start,
                                            ));
                                        }
                                    }
                                }

                                // Check gap after last item
                                if gap_interval.is_none() {
                                    if let Some(last) = row.last() {
                                        if click_time > last.interval.stop {
                                            gap_interval = Some(Interval::new(
                                                last.interval.stop,
                                                tile_id.0.stop,
                                            ));
                                        }
                                    }
                                }

                                // Empty row — whole tile is a gap
                                if gap_interval.is_none() && row.is_empty() {
                                    gap_interval = Some(tile_id.0);
                                }

                                if let Some(gap) = gap_interval {
                                    config.ai_timeline_selection =
                                        Some((self.entry_id.clone(), gap, self.long_name.clone()));
                                }
                            }
                        }
                    }
                }
            });
        }
    }

    /// Post-item overlays: the blue AI region-selection tint, then hover
    /// tooltips for AI highlights (whose fills were painted pre-items).
    pub(super) fn ai_selection_tint_and_tooltips(
        &self,
        ui: &mut egui::Ui,
        config: &Config,
        cx: &Context,
        rect: Rect,
        hover_pos: Option<Pos2>,
        tile_id: TileID,
    ) {
        // Render AI timeline selection highlight (blue overlay)
        if let Some((ref sel_entry_id, ref sel_interval, _)) = config.ai_timeline_selection {
            if sel_entry_id == &self.entry_id {
                let norm_start = cx.view_interval.unlerp(sel_interval.start).clamp(0.0, 1.0);
                let norm_stop = cx.view_interval.unlerp(sel_interval.stop).clamp(0.0, 1.0);
                if norm_stop > 0.0 && norm_start < 1.0 {
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let sel_rect = Rect::from_min_max(min, max);
                    let fill = Color32::from_rgba_unmultiplied(50, 100, 255, 30);
                    ui.painter().rect_filled(sel_rect, 0.0, fill);
                    ui.painter().rect_stroke(
                        sel_rect,
                        0.0,
                        Stroke::new(1.5, Color32::from_rgba_unmultiplied(80, 140, 255, 150)),
                    );
                }
            }
        }

        // Handle AI highlight tooltips (rendering done above, before items)
        if config.ai_highlights_enabled {
            if let Some(highlights) = config.ai_highlights.get(&self.entry_id) {
                for hl in highlights {
                    // Honor the per-highlight enable toggle (manager checkbox).
                    if !hl.enabled {
                        continue;
                    }
                    // Map interval to normalized [0,1] within view
                    let norm_start = cx.view_interval.unlerp(hl.interval.start).clamp(0.0, 1.0);
                    let norm_stop = cx.view_interval.unlerp(hl.interval.stop).clamp(0.0, 1.0);

                    // Skip if highlight is outside view
                    if norm_stop <= 0.0 || norm_start >= 1.0 {
                        continue;
                    }

                    // Full slot height rect
                    let min = rect.lerp_inside(Vec2::new(norm_start, 0.0));
                    let max = rect.lerp_inside(Vec2::new(norm_stop, 1.0));
                    let hl_rect = Rect::from_min_max(min, max);

                    // Tooltip on hover
                    if let Some(h) = hover_pos {
                        if h.x >= min.x && h.x <= max.x && hl_rect.contains(h) {
                            let tooltip_id =
                                ("ai_highlight", tile_id.0.start.0, hl.interval.start.0);
                            ui.show_tooltip_ui(tooltip_id, &hl_rect, |ui| {
                                ui.label(RichText::new(&hl.label).strong());
                                ui.label(format!("Interval: {}", hl.interval));
                            });
                        }
                    }
                }
            }
        }
    }
}

impl Window {
    /// Set the kind filter to show only the given processor kinds (empty = all).
    /// Requested names are matched case-insensitively AND by substring, so a
    /// request of "gpu" selects both "gpudev" and "gpuhost". Returns the number
    /// of known kinds matched (0 means nothing matched — caller may warn).
    fn set_kind_filter(&mut self, kinds: &[String]) -> usize {
        self.config.kind_filter.clear();
        for req in kinds {
            let needle = req.to_lowercase();
            for k in &self.config.kinds {
                let hay = k.to_lowercase();
                if hay == needle || hay.contains(&needle) {
                    self.config.kind_filter.insert(k.clone());
                }
            }
        }
        self.config.kind_filter.len()
    }

    /// Expand or collapse every kind panel matching `kind` (case-insensitive,
    /// substring), mirroring the Expand/Collapse-by-kind controls. A request of
    /// "gpu" matches both "gpudev" and "gpuhost".
    fn set_kind_expanded(&mut self, kind: &str, expanded: bool) {
        let needle = kind.to_lowercase();
        for node in &mut self.panel.slots {
            for k in &mut node.slots {
                // Scope the immutable label borrow so it ends before toggle_expanded().
                let matches = {
                    let label = k.label_text();
                    label == needle || label.contains(needle.as_str())
                };
                if matches && k.expanded != expanded {
                    k.toggle_expanded();
                }
            }
        }
    }

    /// Highlight manager — reuses the search-results backend shape (count
    /// header + ScrollArea + the zoom/expand/scroll click handler). FLAT list across
    /// `ai_highlights`, sorted by `id`; each row = an enable checkbox + the label as a
    /// button that zooms to (and expands) the highlight. Globals: toggle all / clear
    /// all / zoom to the union of enabled highlights.
    pub(super) fn highlight_manager(&mut self, ui: &mut egui::Ui, cx: &mut Context) {
        let total: usize = self.config.ai_highlights.values().map(Vec::len).sum();
        ui.heading(format!("Profile {}: Highlights ({total})", self.index));
        if total == 0 {
            ui.label("No highlights.");
            return;
        }

        // Globals row: toggle all overlays · clear all · zoom to the enabled union.
        ui.horizontal(|ui| {
            if ui
                .button("Toggle all")
                .on_hover_text("Show or hide all overlays")
                .clicked()
            {
                self.config.ai_highlights_enabled = !self.config.ai_highlights_enabled;
            }
            if ui
                .button("Clear all")
                .on_hover_text("Remove all highlights")
                .clicked()
            {
                self.config.ai_highlights.clear();
            }
            if ui
                .button("Zoom to all")
                .on_hover_text("Frame the union of enabled highlights")
                .clicked()
            {
                if let Some(u) = highlight_union(&self.config.ai_highlights) {
                    ProfApp::zoom(cx, u);
                }
            }
        });

        // Flat list, stable order by id. Record a clicked row, then act AFTER the
        // scroll area releases the &mut borrow of ai_highlights (mirrors search_results).
        let mut clicked: Option<(EntryID, Interval, Option<ItemUID>)> = None;
        ScrollArea::vertical()
            .max_height(ui.available_height().min(240.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (eid, hl) in flatten_highlights_sorted(&mut self.config.ai_highlights) {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut hl.enabled, "");
                        let label = if hl.label.is_empty() {
                            format!("highlight {}", hl.id)
                        } else {
                            hl.label.clone()
                        };
                        if ui.add(egui::Button::new(label).small()).clicked() {
                            clicked = Some((eid.clone(), hl.interval, hl.item_uid));
                        }
                    });
                }
            });

        if let Some((eid, interval, item_uid)) = clicked {
            // Same as the search-result click: zoom to (a padded) interval + expand
            // the row. uid-less gaps/regions stop here; a future task-target also
            // scrolls to the item.
            ProfApp::zoom(cx, interval.grow(interval.duration_ns() / 20));
            self.expand_slot(&eid);
            if let Some(uid) = item_uid {
                self.config.scroll_to_item(ItemLocator {
                    entry_id: eid,
                    irow: None,
                    item_uid: uid,
                });
            }
        }
    }
}

// ── Agent highlight helpers ──────────────────────────────────────────────────

/// Reproduce the slug-part algorithm from `duckdb_data::sanitize_short`:
/// remove spaces, extract ASCII alphanumeric runs, join with `_`, lowercase.
fn slug_part(name: &str) -> String {
    let no_spaces: String = name.chars().filter(|c| *c != ' ').collect();
    let mut result = String::new();
    let mut in_word = false;
    for c in no_spaces.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_word && !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            in_word = true;
        } else {
            in_word = false;
        }
    }
    result
}

/// Build a `slug → EntryID` map by traversing the 3-level panel hierarchy.
/// Matches the slug generation in `duckdb_data::walk_entry_list`.
fn build_slug_map(window: &Window) -> HashMap<String, EntryID> {
    let mut map = HashMap::new();
    // window.panel = Panel<Panel<Panel<Slot>>>
    // level-1: node panels  (N0, N1, …)
    // level-2: kind panels  (CPU, GPU, Utility, …)
    // level-3: slot entries (C0, C1, …)
    for node_panel in &window.panel.slots {
        let node_slug = slug_part(&node_panel.short_name);
        map.insert(node_slug.clone(), node_panel.entry_id.clone());

        for kind_panel in &node_panel.slots {
            let kind_slug = format!("{}_{}", node_slug, slug_part(&kind_panel.short_name));
            map.insert(kind_slug.clone(), kind_panel.entry_id.clone());

            for slot in &kind_panel.slots {
                let slot_slug = format!("{}_{}", kind_slug, slug_part(&slot.short_name));
                map.insert(slot_slug.clone(), slot.entry_id.clone());
            }
        }
    }
    map
}

/// Monotonic source of unique highlight ids (the manager's stable ordering key).
static NEXT_HIGHLIGHT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Allocate the next unique highlight id.
fn next_highlight_id() -> u64 {
    NEXT_HIGHLIGHT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Build the renderable [`AiHighlight`] from an agent [`Highlight`]. Severity is no
/// longer used (one uniform light-red overlay); the optional `item_uid` task-target
/// is `None` (the tool is region/interval-based today). Used by BOTH the embedded
/// apply and the MCP handler, so the id is allocated here for uniqueness across both.
fn highlight_to_ai(hl: &Highlight) -> AiHighlight {
    use crate::timestamp::Timestamp;
    AiHighlight {
        id: next_highlight_id(),
        interval: Interval::new(Timestamp(hl.start_ns), Timestamp(hl.stop_ns)),
        label: hl.label.clone(),
        item_uid: None,
        enabled: true,
    }
}

/// Encode an egui [`ColorImage`](egui::ColorImage) as a PNG byte vector.
///
/// Used by the screenshot capture pipeline to convert the egui viewport
/// screenshot into PNG format for the Claude API's vision capability.
fn encode_screenshot_png(color_image: &egui::ColorImage) -> Vec<u8> {
    use image::ImageEncoder;
    let rgba: Vec<u8> = color_image
        .pixels
        .iter()
        .flat_map(|p| p.to_array())
        .collect();
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(
            &rgba,
            color_image.size[0] as u32,
            color_image.size[1] as u32,
            image::ExtendedColorType::Rgba8,
        )
        .expect("PNG encode failed");
    buf
}

/// Apply a navigation/screenshot request to the live view (zoom / pan / scroll /
/// filter / search / reset). Shared by the embedded chat agent and any second
/// consumer (the in-viewer MCP bridge) so both run identical view logic. Does NOT request
/// the screenshot — the caller sends the `ViewportCommand` and records the
/// awaiting slot.
fn apply_navigation(cx: &mut Context, windows: &mut [Window], nav: &crate::ai::PendingNavigation) {
    use crate::ai::PendingNavigation;
    match nav {
        PendingNavigation::Screenshot { .. } => {
            // Plain screenshot — no navigation changes needed.
        }
        PendingNavigation::Zoom {
            start_ns, stop_ns, ..
        } => {
            let interval = Interval::new(Timestamp(*start_ns), Timestamp(*stop_ns));
            ProfApp::zoom(cx, interval);
        }
        PendingNavigation::Pan {
            direction, percent, ..
        } => {
            let pct = (percent.round() as i64).clamp(1, 200);
            let dir = if direction.as_str() == "left" {
                PanDirection::Left
            } else {
                PanDirection::Right
            };
            ProfApp::pan(cx, Percentage::from(pct), dir);
        }
        PendingNavigation::ScrollTo { entry_slug, .. } => {
            for window in windows.iter_mut() {
                let slug_map = build_slug_map(window);
                if let Some(entry_id) = slug_map.get(entry_slug) {
                    window.expand_slot(entry_id);
                    cx.ai_scroll_to_entry = Some(entry_id.clone());
                    break;
                }
            }
        }
        PendingNavigation::SetView {
            start_ns,
            stop_ns,
            entry_slug,
            filter_kinds,
            expand_kinds,
            collapse_kinds,
            vertical_scale,
            ..
        } => {
            let interval = Interval::new(Timestamp(*start_ns), Timestamp(*stop_ns));
            ProfApp::zoom(cx, interval);
            if let Some(scale) = vertical_scale {
                cx.scale_factor = (*scale as f32).clamp(0.25, 4.0);
            }
            for window in windows.iter_mut() {
                if let Some(kinds) = filter_kinds {
                    let matched = window.set_kind_filter(kinds);
                    if matched == 0 && !kinds.is_empty() {
                        log::warn!("set_view filter_kinds matched no known kinds: {kinds:?}");
                    }
                }
                if let Some(kinds) = expand_kinds {
                    for k in kinds {
                        window.set_kind_expanded(k, true);
                    }
                }
                if let Some(kinds) = collapse_kinds {
                    for k in kinds {
                        window.set_kind_expanded(k, false);
                    }
                }
            }
            if let Some(slug) = entry_slug {
                for window in windows.iter_mut() {
                    let slug_map = build_slug_map(window);
                    if let Some(entry_id) = slug_map.get(slug) {
                        window.expand_slot(entry_id);
                        cx.ai_scroll_to_entry = Some(entry_id.clone());
                        break;
                    }
                }
            }
        }
        PendingNavigation::Search { query, .. } => {
            for window in windows.iter_mut() {
                window.config.search_state.query = query.clone();
                window.search(cx);
            }
        }
        PendingNavigation::ResetView { .. } => {
            ProfApp::zoom(cx, cx.total_interval);
            cx.scale_factor = 1.0;
            for window in windows.iter_mut() {
                window.config.kind_filter.clear();
                window.config.search_state.query = String::new();
                window.search(cx);
            }
        }
    }
}

/// Build the header "Selected:" banner line from a `selection_snapshot`
/// (`items`, `range`). Pure + egui-free so it is unit-testable, and it reads the
/// SAME snapshot `get_selection` returns, so the header and the MCP agent agree on
/// what is selected. Returns `None` when nothing is selected (the header then
/// renders no empty chrome). At most the first 2 task bars are shown in full; the
/// rest collapse to "+N more".
pub(super) fn format_selection_banner(
    items: &[crate::ai::SelectedItemInfo],
    range: &Option<(String, i64, i64)>,
) -> Option<String> {
    use crate::timestamp::Timestamp;
    if items.is_empty() && range.is_none() {
        return None;
    }
    const SHOWN: usize = 2;
    let mut parts: Vec<String> = Vec::new();
    if let Some((label, start, stop)) = range {
        parts.push(format!(
            "{}–{} ({label})",
            Timestamp(*start),
            Timestamp(*stop)
        ));
    }
    for it in items.iter().take(SHOWN) {
        let title = if it.title.is_empty() {
            format!("uid {}", it.item_uid)
        } else {
            it.title.clone()
        };
        let slug = it.entry_slug.as_deref().unwrap_or("?");
        parts.push(format!(
            "{title} @ {}–{} ({slug})",
            Timestamp(it.start_ns),
            Timestamp(it.stop_ns)
        ));
    }
    if items.len() > SHOWN {
        parts.push(format!("+{} more", items.len() - SHOWN));
    }
    Some(format!("Selected: {}", parts.join("  ·  ")))
}

/// The `request_id` carried by any navigation variant.
fn pending_nav_request_id(nav: &crate::ai::PendingNavigation) -> u64 {
    use crate::ai::PendingNavigation;
    match nav {
        PendingNavigation::Screenshot { request_id }
        | PendingNavigation::Zoom { request_id, .. }
        | PendingNavigation::Pan { request_id, .. }
        | PendingNavigation::ScrollTo { request_id, .. }
        | PendingNavigation::SetView { request_id, .. }
        | PendingNavigation::Search { request_id, .. }
        | PendingNavigation::ResetView { request_id } => *request_id,
    }
}

/// Apply ONE AI highlight to the live timeline state shared with the embedded
/// path (`window.config.ai_highlights`): expand the row, dedup-push the overlay,
/// enable rendering. Returns the matched `EntryID` (for scroll-to). Mirrors the
/// embedded highlight-action application (kept separate to leave the embedded
/// sole-driver path independent); used by the MCP source.
fn apply_one_highlight(windows: &mut [Window], hl: &crate::ai::Highlight) -> Option<EntryID> {
    let mut found = None;
    for window in windows.iter_mut() {
        let slug_map = build_slug_map(window);
        if let Some(entry_id) = slug_map.get(&hl.entry_slug) {
            window.expand_slot(entry_id);
            let ai_hl = highlight_to_ai(hl);
            let entry = window
                .config
                .ai_highlights
                .entry(entry_id.clone())
                .or_default();
            let dup = entry.iter().any(|h| {
                h.interval.start.0 == ai_hl.interval.start.0
                    && h.interval.stop.0 == ai_hl.interval.stop.0
                    && h.label == ai_hl.label
            });
            if !dup {
                entry.push(ai_hl);
            }
            window.config.ai_highlights_enabled = true;
            if found.is_none() {
                found = Some(entry_id.clone());
            }
        }
    }
    found
}

/// Clear ALL AI highlight overlays from every window. Returns the number of rows
/// that had highlights (for a truthful ACK).
fn clear_all_highlights(windows: &mut [Window]) -> usize {
    let mut n = 0;
    for window in windows.iter_mut() {
        n += window.config.ai_highlights.len();
        window.config.ai_highlights.clear();
    }
    n
}

/// Flatten `ai_highlights` into the manager's row order: a flat list of
/// `(entry_id, &mut highlight)` sorted by the stable `id`. Used by the manager (the
/// `&mut` lets each row drive its enable checkbox) and unit-tested for deterministic
/// ordering. Pure (no egui).
fn flatten_highlights_sorted(
    map: &mut HashMap<EntryID, Vec<AiHighlight>>,
) -> Vec<(EntryID, &mut AiHighlight)> {
    let mut rows: Vec<(EntryID, &mut AiHighlight)> = map
        .iter_mut()
        .flat_map(|(eid, v)| v.iter_mut().map(move |h| (eid.clone(), h)))
        .collect();
    rows.sort_by_key(|(_, h)| h.id);
    rows
}

/// Union of the intervals of all ENABLED highlights (for "Zoom to all"); disabled
/// highlights are ignored. `None` when nothing is enabled. Pure.
fn highlight_union(map: &HashMap<EntryID, Vec<AiHighlight>>) -> Option<Interval> {
    use crate::timestamp::Timestamp;
    let mut bounds: Option<(i64, i64)> = None;
    for h in map.values().flatten().filter(|h| h.enabled) {
        let (s, e) = (h.interval.start.0, h.interval.stop.0);
        bounds = Some(match bounds {
            None => (s, e),
            Some((bs, be)) => (bs.min(s), be.max(e)),
        });
    }
    bounds.map(|(s, e)| Interval::new(Timestamp(s), Timestamp(e)))
}

/// Sink for draining the SECOND event source (the in-viewer MCP). Records the ONE
/// request serviced per drain (the viewport token guarantees a single outstanding
/// request across both sources): a navigation/screenshot (applied via the shared
/// screenshot pipeline) OR a highlight / clear-highlights (applied + ACKed by the
/// drain region). It does not emit chat events.
#[derive(Default)]
struct McpDrainSink {
    pending: Option<(
        crate::ai::PendingNavigation,
        std::sync::mpsc::Sender<crate::ai::UiCommand>,
    )>,
    /// (highlight, request_id, reply channel) — applied to the live state + ACKed.
    pending_highlight: Option<(
        crate::ai::Highlight,
        u64,
        std::sync::mpsc::Sender<crate::ai::UiCommand>,
    )>,
    /// (request_id, reply channel) for a clear-highlights request.
    pending_clear: Option<(u64, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
    /// (request_id, reply channel) for a get_selection READ (non-driving).
    pending_selection: Option<(u64, std::sync::mpsc::Sender<crate::ai::UiCommand>)>,
}

impl crate::ai::bridge::EventSink for McpDrainSink {
    fn on_navigation(
        &mut self,
        nav: crate::ai::PendingNavigation,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending = Some((nav, reply_tx.clone()));
    }

    fn on_highlight(
        &mut self,
        request_id: u64,
        entry_slug: String,
        start_ns: i64,
        stop_ns: i64,
        severity: String,
        label: String,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_highlight = Some((
            crate::ai::Highlight {
                entry_slug,
                start_ns,
                stop_ns,
                severity,
                label,
            },
            request_id,
            reply_tx.clone(),
        ));
    }

    fn on_clear_highlights_request(
        &mut self,
        request_id: u64,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_clear = Some((request_id, reply_tx.clone()));
    }

    fn on_get_selection(
        &mut self,
        request_id: u64,
        reply_tx: &std::sync::mpsc::Sender<crate::ai::UiCommand>,
    ) {
        self.pending_selection = Some((request_id, reply_tx.clone()));
    }
}

/// Build a metadata string describing the visible time range and entry
/// slugs in the current screenshot.  Sent alongside the PNG so Claude knows
/// the numeric context of the image.
fn build_screenshot_metadata(cx: &Context, windows: &[Window]) -> String {
    let start = cx.view_interval.start.0;
    let stop = cx.view_interval.stop.0;
    let duration_ms = (stop - start) as f64 / 1_000_000.0;

    // Collect visible entry slugs from the first window
    let mut entry_slugs: Vec<String> = Vec::new();
    if let Some(window) = windows.first() {
        for node_panel in &window.panel.slots {
            // Node filter: use same logic as is_slot_visible (level 1)
            let node_idx = node_panel.entry_id.last_slot_index().unwrap_or(0);
            if node_idx < window.config.min_node || node_idx > window.config.max_node {
                continue;
            }
            if !node_panel.expanded {
                continue;
            }
            let node_slug = slug_part(&node_panel.short_name);

            for kind_panel in &node_panel.slots {
                // Kind filter: use same logic as is_slot_visible (level 2)
                if !window.config.kind_filter.is_empty()
                    && !window.config.kind_filter.contains(&kind_panel.short_name)
                {
                    continue;
                }
                if !kind_panel.expanded {
                    continue;
                }
                let kind_slug = format!("{}_{}", node_slug, slug_part(&kind_panel.short_name));

                for slot in &kind_panel.slots {
                    let slot_slug = format!("{}_{}", kind_slug, slug_part(&slot.short_name));
                    entry_slugs.push(slot_slug);
                }
            }
        }
    }

    // If a search is active, report the query and how many tasks matched.
    let search_note = windows
        .first()
        .filter(|w| !w.config.search_state.query.is_empty())
        .map(|w| {
            format!(
                " Active search: \"{}\" ({} matches highlighted).",
                w.config.search_state.query,
                w.config.search_state.result_set.len()
            )
        })
        .unwrap_or_default();

    format!(
        "Screenshot captured. Visible time range: {} ns \u{2013} {} ns ({:.2} ms). \
         Visible entries (top to bottom): {}.{} \
         Use these entry_slugs and time range for follow-up queries.",
        start,
        stop,
        duration_ms,
        entry_slugs.join(", "),
        search_note
    )
}

/// Pins that the in-viewer MCP sink (`McpDrainSink`) RECORDS each visual
/// variant rather than silently no-op'ing (the default `EventSink` methods are
/// no-ops; an unrecorded request would block `UiBridge::request` to timeout). The
/// actual UI application (`apply_navigation` / `apply_one_highlight`) needs a live
/// window and is covered by the end-to-end smoke.
#[cfg(test)]
mod mcp_sink_tests {
    use super::McpDrainSink;
    use crate::ai::AgentEvent;
    use crate::ai::bridge::apply_agent_event;
    use std::sync::mpsc::channel;

    /// Drive one event into a fresh sink, return the sink.
    fn drive(ev: AgentEvent) -> McpDrainSink {
        let (tx, _rx) = channel();
        let mut sink = McpDrainSink::default();
        apply_agent_event(&mut sink, ev, &tx);
        sink
    }

    #[test]
    fn test_mcp_sink_records_every_navigation_variant() {
        let evs = vec![
            AgentEvent::ScreenshotRequest { request_id: 1 },
            AgentEvent::ZoomRequest {
                request_id: 2,
                start_ns: 0,
                stop_ns: 10,
            },
            AgentEvent::PanRequest {
                request_id: 3,
                direction: "left".into(),
                percent: 25.0,
            },
            AgentEvent::ScrollToRequest {
                request_id: 4,
                entry_slug: "n0_cpu_c1".into(),
            },
            AgentEvent::SetViewRequest {
                request_id: 5,
                start_ns: 0,
                stop_ns: 10,
                entry_slug: None,
                filter_kinds: None,
                expand_kinds: None,
                collapse_kinds: None,
                vertical_scale: None,
            },
            AgentEvent::SearchRequest {
                request_id: 6,
                query: "x".into(),
            },
            AgentEvent::ResetViewRequest { request_id: 7 },
        ];
        for ev in evs {
            let sink = drive(ev);
            assert!(
                sink.pending.is_some(),
                "nav variant must be RECORDED, not a no-op"
            );
            assert!(sink.pending_highlight.is_none() && sink.pending_clear.is_none());
        }
    }

    #[test]
    fn test_mcp_sink_records_highlight() {
        let sink = drive(AgentEvent::HighlightRequest {
            request_id: 9,
            entry_slug: "n0_cpu_c1".into(),
            start_ns: 1,
            stop_ns: 2,
            severity: "high".into(),
            label: "blk".into(),
        });
        let (hl, rid, _tx) = sink
            .pending_highlight
            .expect("highlight must be RECORDED, not a no-op");
        assert_eq!(rid, 9);
        assert_eq!(hl.entry_slug, "n0_cpu_c1");
        assert_eq!((hl.start_ns, hl.stop_ns), (1, 2));
        assert!(sink.pending.is_none() && sink.pending_clear.is_none());
    }

    #[test]
    fn test_mcp_sink_records_clear() {
        let sink = drive(AgentEvent::ClearHighlightsRequest { request_id: 11 });
        let (rid, _tx) = sink
            .pending_clear
            .expect("clear must be RECORDED, not a no-op");
        assert_eq!(rid, 11);
        assert!(sink.pending.is_none() && sink.pending_highlight.is_none());
    }

    #[test]
    fn test_mcp_sink_records_get_selection() {
        let sink = drive(AgentEvent::GetSelection { request_id: 13 });
        let (rid, _tx) = sink
            .pending_selection
            .expect("get_selection must be RECORDED, not a no-op");
        assert_eq!(rid, 13);
        assert!(
            sink.pending.is_none()
                && sink.pending_highlight.is_none()
                && sink.pending_clear.is_none()
        );
    }
}

/// Pins the header "Selected:" banner formatting (egui-free).
#[cfg(test)]
mod banner_tests {
    use super::format_selection_banner;
    use crate::ai::SelectedItemInfo;

    fn item(uid: u64, title: &str) -> SelectedItemInfo {
        SelectedItemInfo {
            item_uid: uid,
            entry_slug: Some("n0_cpu_c1".into()),
            title: title.into(),
            start_ns: 1_000_000_000,
            stop_ns: 1_200_000_000,
        }
    }

    #[test]
    fn test_format_selection_banner_empty() {
        // Nothing selected -> None (header renders no chrome).
        assert_eq!(format_selection_banner(&[], &None), None);
    }

    #[test]
    fn test_format_selection_banner_range_only() {
        let b = format_selection_banner(
            &[],
            &Some(("n0_cpu_c2".into(), 1_000_000_000, 1_500_000_000)),
        )
        .expect("range -> Some");
        assert!(b.starts_with("Selected:"), "banner: {b}");
        assert!(b.contains("n0_cpu_c2"), "range label shown: {b}");
        assert!(
            !b.contains("more"),
            "no overflow for a range-only selection: {b}"
        );
    }

    #[test]
    fn test_format_selection_banner_items() {
        let b =
            format_selection_banner(&[item(48, "top_level <6>")], &None).expect("items -> Some");
        assert!(b.starts_with("Selected:"));
        assert!(b.contains("top_level <6>"), "title shown: {b}");
        assert!(
            b.contains('@') && b.contains("n0_cpu_c1"),
            "interval + slug shown: {b}"
        );
        assert!(!b.contains("more"));
    }

    #[test]
    fn test_format_selection_banner_many_items() {
        // 3 items, SHOWN=2 -> first two in full, the rest collapse to "+1 more".
        let many = vec![item(1, "alpha"), item(2, "beta"), item(3, "gamma")];
        let b = format_selection_banner(&many, &None).expect("items -> Some");
        assert!(
            b.contains("alpha") && b.contains("beta"),
            "first two shown: {b}"
        );
        assert!(
            !b.contains("gamma"),
            "3rd item collapsed, not shown by title: {b}"
        );
        assert!(b.contains("+1 more"), "overflow summarized: {b}");
    }
}

/// Highlight-model tests: the apply path builds an AiHighlight with the
/// right fields and a unique, monotonic id.
#[cfg(test)]
mod highlight_model_tests {
    use super::*;
    use crate::ai::Highlight;

    fn hl(label: &str) -> Highlight {
        Highlight {
            entry_slug: "n0_cpu_c1".into(),
            start_ns: 100,
            stop_ns: 200,
            severity: "critical".into(), // accepted-but-ignored (no severity colors)
            label: label.into(),
        }
    }

    #[test]
    fn test_highlight_to_ai_fields_and_monotonic_id() {
        let a = highlight_to_ai(&hl("blk"));
        let b = highlight_to_ai(&hl("blk2"));
        // Fields carried correctly.
        assert_eq!(a.label, "blk");
        assert_eq!((a.interval.start.0, a.interval.stop.0), (100, 200));
        assert!(
            a.item_uid.is_none(),
            "region/interval highlight has no task target"
        );
        assert!(a.enabled, "new highlights start enabled");
        // Unique + monotonic ids across BOTH apply sites (allocated in highlight_to_ai).
        assert!(
            b.id > a.id,
            "ids must be monotonic + unique: {} then {}",
            a.id,
            b.id
        );
    }

    use crate::ai::AiHighlight;
    use crate::data::EntryID;
    use crate::timestamp::{Interval, Timestamp};
    use std::collections::HashMap;

    fn ahl(id: u64, start: i64, stop: i64, enabled: bool) -> AiHighlight {
        AiHighlight {
            id,
            interval: Interval::new(Timestamp(start), Timestamp(stop)),
            label: format!("h{id}"),
            item_uid: None,
            enabled,
        }
    }

    #[test]
    fn test_flatten_highlights_sorted_is_deterministic() {
        let mut map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        // Out-of-order ids across two entries -> flat list sorted by id.
        map.insert(
            EntryID::root().child(0),
            vec![ahl(30, 0, 1, true), ahl(10, 0, 1, true)],
        );
        map.insert(EntryID::root().child(1), vec![ahl(20, 0, 1, false)]);
        let order: Vec<u64> = flatten_highlights_sorted(&mut map)
            .iter()
            .map(|(_, h)| h.id)
            .collect();
        assert_eq!(
            order,
            vec![10, 20, 30],
            "flat list sorted by id across all entries"
        );
    }

    #[test]
    fn test_highlight_union_enabled_only() {
        let mut map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        // ENABLED [100,200] + [400,500]; a DISABLED [0,1000] must be IGNORED.
        map.insert(
            EntryID::root().child(0),
            vec![ahl(1, 100, 200, true), ahl(2, 0, 1000, false)],
        );
        map.insert(EntryID::root().child(1), vec![ahl(3, 400, 500, true)]);
        let u = highlight_union(&map).expect("some enabled -> Some");
        assert_eq!(
            (u.start.0, u.stop.0),
            (100, 500),
            "union of ENABLED only (disabled [0,1000] ignored)"
        );

        // All disabled -> None.
        let mut none_map: HashMap<EntryID, Vec<AiHighlight>> = HashMap::new();
        none_map.insert(EntryID::root().child(0), vec![ahl(1, 100, 200, false)]);
        assert!(highlight_union(&none_map).is_none(), "no enabled -> None");
        // Empty -> None.
        assert!(highlight_union(&HashMap::new()).is_none());
    }
}
