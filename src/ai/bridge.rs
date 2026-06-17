//! Generalized agent↔UI bridge: lets MORE THAN ONE consumer drive the live
//! viewer through the same request/reply primitive the embedded chat agent uses.
//!
//! V1.0 is a FOUNDATIONAL REFACTOR only — no HTTP, no MCP server, no new tools.
//! It provides three durable pieces, all unit-tested here without a live window:
//!
//! 1. [`EventSink`] + [`apply_agent_event`] — the ONE shared handler that turns an
//!    [`AgentEvent`] into a UI effect, routing any reply to the caller-supplied
//!    `reply_tx`. The same logic services events from any number of sources; the
//!    embedded [`crate::ai::ChatPanel`] implements `EventSink`, and so will the
//!    future in-viewer MCP server.
//! 2. [`ViewportToken`] / [`ViewportGuard`] — structural single-driver enforcement.
//!    A non-owning consumer gets `Err("viewport busy")` instead of interleaving.
//!    The guard releases the viewport on EVERY exit path (success, error, timeout,
//!    disconnect, panic) via `Drop` — a consumer that dies mid-hold can NEVER
//!    deadlock the embedded agent out.
//! 3. [`UiBridge`] — the thread-safe handle a second consumer holds to issue a
//!    blocking [`UiBridge::request`] and receive the matching reply.
//!
//! The embedded chat agent's behavior is unchanged: it remains the transparent,
//! sole driver in V1.0.

use super::agent::{AgentEvent, AgentResponse, UiCommand};
use super::PendingNavigation;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ── EventSink + apply_agent_event: the one shared event handler ──────────────

/// The UI's response to an [`AgentEvent`]. Implemented by every consumer-facing
/// surface (the embedded chat panel today, a headless adapter for the in-viewer
/// MCP next). Navigation is the only required method; the chat-specific callbacks
/// default to no-ops so a navigation-only consumer need not implement them.
pub trait EventSink {
    /// A navigation / screenshot request. `reply_tx` is the channel the eventual
    /// `ScreenshotData` reply for THIS event's source must be sent on.
    fn on_navigation(&mut self, nav: PendingNavigation, reply_tx: &Sender<UiCommand>);

    fn on_tool_call(&mut self, _name: String, _purpose: String) {}
    fn on_tool_result(&mut self, _name: String, _summary: String, _full_content: String) {}
    fn on_question(
        &mut self,
        _request_id: u64,
        _question: String,
        _options: Vec<String>,
        _reply_tx: &Sender<UiCommand>,
    ) {
    }
    fn on_clear_highlights(&mut self) {}
    fn on_complete(&mut self, _response: AgentResponse) {}
    fn on_error(&mut self, _error: String) {}
}

/// Dispatch one [`AgentEvent`] to `sink`, routing any reply to `reply_tx`. This is
/// the single source of truth for AgentEvent → UI handling, shared across every
/// event source.
pub fn apply_agent_event<S: EventSink>(sink: &mut S, event: AgentEvent, reply_tx: &Sender<UiCommand>) {
    match event {
        AgentEvent::ToolCall { name, purpose } => sink.on_tool_call(name, purpose),
        AgentEvent::ToolResult { name, summary, full_content } => {
            sink.on_tool_result(name, summary, full_content)
        }
        AgentEvent::ScreenshotRequest { request_id } => {
            sink.on_navigation(PendingNavigation::Screenshot { request_id }, reply_tx)
        }
        AgentEvent::ZoomRequest { request_id, start_ns, stop_ns } => {
            sink.on_navigation(PendingNavigation::Zoom { request_id, start_ns, stop_ns }, reply_tx)
        }
        AgentEvent::PanRequest { request_id, direction, percent } => {
            sink.on_navigation(PendingNavigation::Pan { request_id, direction, percent }, reply_tx)
        }
        AgentEvent::ScrollToRequest { request_id, entry_slug } => {
            sink.on_navigation(PendingNavigation::ScrollTo { request_id, entry_slug }, reply_tx)
        }
        AgentEvent::SetViewRequest {
            request_id,
            start_ns,
            stop_ns,
            entry_slug,
            filter_kinds,
            expand_kinds,
            collapse_kinds,
            vertical_scale,
        } => sink.on_navigation(
            PendingNavigation::SetView {
                request_id,
                start_ns,
                stop_ns,
                entry_slug,
                filter_kinds,
                expand_kinds,
                collapse_kinds,
                vertical_scale,
            },
            reply_tx,
        ),
        AgentEvent::SearchRequest { request_id, query } => {
            sink.on_navigation(PendingNavigation::Search { request_id, query }, reply_tx)
        }
        AgentEvent::ResetViewRequest { request_id } => {
            sink.on_navigation(PendingNavigation::ResetView { request_id }, reply_tx)
        }
        AgentEvent::QuestionForUser { request_id, question, options } => {
            sink.on_question(request_id, question, options, reply_tx)
        }
        AgentEvent::ClearHighlights => sink.on_clear_highlights(),
        AgentEvent::Complete(response) => sink.on_complete(response),
        AgentEvent::Error(error) => sink.on_error(error),
    }
}

/// Drain all currently-available events from `rx` and apply each to `sink`,
/// routing replies to `reply_tx`. Returns `true` if the channel disconnected.
pub fn drain_source<S: EventSink>(
    rx: &Receiver<AgentEvent>,
    reply_tx: &Sender<UiCommand>,
    sink: &mut S,
) -> bool {
    use std::sync::mpsc::TryRecvError;
    loop {
        match rx.try_recv() {
            Ok(event) => apply_agent_event(sink, event, reply_tx),
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
}

// ── Viewport ownership: structural single-driver guard ───────────────────────

/// A shared, clonable handle to the single viewport-ownership slot. `None` = free;
/// `Some(consumer_id)` = currently held.
#[derive(Clone, Debug)]
pub struct ViewportToken(Arc<Mutex<Option<u64>>>);

impl Default for ViewportToken {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportToken {
    pub fn new() -> Self {
        ViewportToken(Arc::new(Mutex::new(None)))
    }

    /// Try to claim the viewport for `consumer_id`. Succeeds if the viewport is
    /// free OR already held by this same consumer (re-entrant); returns
    /// `Err("viewport busy")` if another consumer holds it. The returned
    /// [`ViewportGuard`] releases the viewport when dropped — on ANY exit path.
    pub fn try_claim(&self, consumer_id: u64) -> Result<ViewportGuard, &'static str> {
        let mut slot = self.0.lock().unwrap();
        // `owns` distinguishes the guard that actually took the slot from a
        // re-entrant (already-held) view. Only the OWNING guard releases on drop,
        // so a re-entrant guard dropping first cannot prematurely free a claim that
        // the original holder still owns.
        let owns = match *slot {
            None => {
                *slot = Some(consumer_id);
                true
            }
            Some(owner) if owner == consumer_id => false,
            Some(_) => return Err("viewport busy"),
        };
        Ok(ViewportGuard {
            token: Arc::clone(&self.0),
            consumer_id,
            owns,
        })
    }

    /// The current owner, or `None` if free. (Diagnostics / tests.)
    pub fn current_owner(&self) -> Option<u64> {
        *self.0.lock().unwrap()
    }
}

/// RAII viewport lease. Dropping it frees the viewport IFF this guard still holds
/// it — so success, early-`?`-return, timeout, disconnect, and panic-unwind all
/// release the token. This is the contract that prevents a dead consumer from
/// deadlocking the embedded agent out of the viewport.
#[must_use = "the viewport is released as soon as the guard is dropped"]
pub struct ViewportGuard {
    token: Arc<Mutex<Option<u64>>>,
    consumer_id: u64,
    /// True only for the guard that actually claimed the slot; re-entrant guards
    /// are non-owning and do not release.
    owns: bool,
}

impl Drop for ViewportGuard {
    fn drop(&mut self) {
        if !self.owns {
            return;
        }
        let mut slot = self.token.lock().unwrap();
        if *slot == Some(self.consumer_id) {
            *slot = None;
        }
    }
}

// ── UiBridge: a second consumer's blocking handle to the live viewer ─────────

/// Default per-request timeout for a [`UiBridge`] (screenshots render on the next
/// frame; this bounds how long a consumer blocks if the UI never replies).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A thread-safe handle the future in-viewer MCP server thread holds to drive the
/// live window. Mirrors the embedded agent's request/reply primitive
/// (`emit` + `wait_for_command`) but adds the viewport-ownership guard so a
/// non-owning consumer is cleanly locked out rather than interleaving.
pub struct UiBridge {
    event_tx: Sender<AgentEvent>,
    cmd_rx: Receiver<UiCommand>,
    token: ViewportToken,
    consumer_id: u64,
    next_request_id: AtomicU64,
}

impl UiBridge {
    pub fn new(
        event_tx: Sender<AgentEvent>,
        cmd_rx: Receiver<UiCommand>,
        token: ViewportToken,
        consumer_id: u64,
    ) -> Self {
        UiBridge {
            event_tx,
            cmd_rx,
            token,
            consumer_id,
            next_request_id: AtomicU64::new(0),
        }
    }

    pub fn alloc_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn consumer_id(&self) -> u64 {
        self.consumer_id
    }

    /// Issue a blocking request against the live viewer.
    ///
    /// `make_event(request_id)` builds the event to send; `match_reply` selects
    /// the matching reply (typically by `request_id`). Claims the viewport for the
    /// duration of the request via an RAII guard, so on EVERY exit path — success,
    /// `viewport busy`, send failure, timeout, disconnect, or panic — the viewport
    /// is released and the embedded agent is never starved.
    pub fn request(
        &self,
        make_event: impl FnOnce(u64) -> AgentEvent,
        match_reply: impl Fn(&UiCommand) -> bool,
        timeout: Duration,
    ) -> Result<UiCommand, String> {
        // RAII: held for the whole request; dropped (released) on any return below.
        let _guard = self.token.try_claim(self.consumer_id).map_err(String::from)?;

        let request_id = self.alloc_request_id();
        self.event_tx
            .send(make_event(request_id))
            .map_err(|_| "viewport event channel disconnected".to_string())?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.cmd_rx.recv_timeout(remaining) {
                Ok(cmd) if match_reply(&cmd) => return Ok(cmd),
                Ok(_) => continue, // stale / mismatched reply — keep waiting
                Err(RecvTimeoutError::Timeout) => {
                    return Err("viewport request timed out".to_string());
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("viewport reply channel disconnected".to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    // ── EventSink fake: records navs and replies synchronously on the supplied
    //    reply_tx (a real UI defers the screenshot to the next frame; this lets us
    //    pin the multi-source reply ROUTING headlessly). ──
    #[derive(Default)]
    struct FakeSink {
        navs: Vec<PendingNavigation>,
        clears: u32,
    }
    impl EventSink for FakeSink {
        fn on_navigation(&mut self, nav: PendingNavigation, reply_tx: &Sender<UiCommand>) {
            // Echo a synthetic screenshot reply on the SOURCE's reply channel.
            let _ = reply_tx.send(UiCommand::ScreenshotData {
                request_id: 0,
                png_bytes: vec![1, 2, 3],
                metadata: "fake".to_string(),
            });
            self.navs.push(nav);
        }
        fn on_clear_highlights(&mut self) {
            self.clears += 1;
        }
    }

    /// The real seam: drain TWO independent sources and confirm each reply lands on
    /// the MATCHING reply channel (not interleaved/crossed).
    #[test]
    fn test_two_source_drain_routes_replies_to_matching_channel() {
        let (etx1, erx1) = channel::<AgentEvent>();
        let (ctx1, crx1) = channel::<UiCommand>();
        let (etx2, erx2) = channel::<AgentEvent>();
        let (ctx2, crx2) = channel::<UiCommand>();

        // Source 1 issues a Zoom; source 2 issues a plain Screenshot.
        etx1.send(AgentEvent::ZoomRequest { request_id: 7, start_ns: 1, stop_ns: 2 }).unwrap();
        etx2.send(AgentEvent::ScreenshotRequest { request_id: 9 }).unwrap();

        let mut sink = FakeSink::default();
        let d1 = drain_source(&erx1, &ctx1, &mut sink);
        let d2 = drain_source(&erx2, &ctx2, &mut sink);
        assert!(!d1 && !d2, "live senders -> not disconnected");

        // Both serviced (order: source1 then source2).
        assert_eq!(sink.navs.len(), 2);
        assert!(matches!(sink.navs[0], PendingNavigation::Zoom { request_id: 7, .. }));
        assert!(matches!(sink.navs[1], PendingNavigation::Screenshot { request_id: 9 }));

        // Each reply landed on its OWN channel — exactly one each, none crossed.
        assert!(matches!(crx1.try_recv(), Ok(UiCommand::ScreenshotData { .. })));
        assert!(crx1.try_recv().is_err(), "channel 1 must not receive channel 2's reply");
        assert!(matches!(crx2.try_recv(), Ok(UiCommand::ScreenshotData { .. })));
        assert!(crx2.try_recv().is_err(), "channel 2 must not receive channel 1's reply");
    }

    #[test]
    fn test_drain_reports_disconnect() {
        let (etx, erx) = channel::<AgentEvent>();
        let (ctx, _crx) = channel::<UiCommand>();
        etx.send(AgentEvent::ClearHighlights).unwrap();
        drop(etx); // sender gone
        let mut sink = FakeSink::default();
        assert!(drain_source(&erx, &ctx, &mut sink), "must report disconnect");
        assert_eq!(sink.clears, 1, "the queued event is still processed before disconnect");
    }

    /// Consumer A owns the viewport -> consumer B is cleanly locked out; after A
    /// releases (guard dropped), B's claim succeeds.
    #[test]
    fn test_viewport_busy_then_free() {
        let token = ViewportToken::new();
        let guard_a = token.try_claim(1).expect("A claims a free viewport");
        assert_eq!(token.current_owner(), Some(1));

        // B is locked out while A holds it.
        assert_eq!(token.try_claim(2).err(), Some("viewport busy"));

        // A is re-entrant, but the re-entrant guard is NON-OWNING: dropping it must
        // NOT release A's claim (pins the premature-release fix).
        {
            let _reentrant = token.try_claim(1).expect("re-entrant claim by the same consumer");
            assert_eq!(token.current_owner(), Some(1));
        }
        assert_eq!(token.current_owner(), Some(1), "re-entrant drop must not release A");

        drop(guard_a);
        assert_eq!(token.current_owner(), None, "owning guard releases on drop");
        assert!(token.try_claim(2).is_ok(), "B claims after A releases");
    }

    /// THE failure-path contract: a consumer that claims then DIES (drops its guard
    /// WITHOUT an explicit release — simulating client disconnect / tool timeout /
    /// thread panic) must free the token, so a DIFFERENT consumer's later claim
    /// succeeds. Happy-path-only release would deadlock here.
    #[test]
    fn test_release_on_failure_not_just_success() {
        let token = ViewportToken::new();
        {
            let _dead = token.try_claim(1).expect("dead consumer claims");
            assert_eq!(token.current_owner(), Some(1));
            // No explicit release — the consumer "dies" and `_dead` drops at scope end.
        }
        assert_eq!(token.current_owner(), None, "guard freed the token on drop, not on success");
        assert!(token.try_claim(2).is_ok(), "a different consumer can now claim");
    }

    /// UiBridge end-to-end with a stub UI thread: claim -> send -> matching reply,
    /// and the viewport is released after the request completes.
    #[test]
    fn test_ui_bridge_request_roundtrip_and_release() {
        let (event_tx, event_rx) = channel::<AgentEvent>();
        let (cmd_tx, cmd_rx) = channel::<UiCommand>();
        let token = ViewportToken::new();
        let bridge = UiBridge::new(event_tx, cmd_rx, token.clone(), 42);

        // Stub UI: on any event, reply with ScreenshotData echoing the request_id.
        let ui = std::thread::spawn(move || {
            if let Ok(ev) = event_rx.recv() {
                let rid = match ev {
                    AgentEvent::ZoomRequest { request_id, .. }
                    | AgentEvent::ScreenshotRequest { request_id } => request_id,
                    _ => return,
                };
                let _ = cmd_tx.send(UiCommand::ScreenshotData {
                    request_id: rid,
                    png_bytes: vec![9],
                    metadata: "ok".to_string(),
                });
            }
        });

        let reply = bridge
            .request(
                |rid| AgentEvent::ScreenshotRequest { request_id: rid },
                |cmd| matches!(cmd, UiCommand::ScreenshotData { request_id: 0, .. }),
                Duration::from_secs(5),
            )
            .expect("request should round-trip");
        assert!(matches!(reply, UiCommand::ScreenshotData { request_id: 0, .. }));
        // Viewport released after the request returned (guard dropped).
        assert_eq!(token.current_owner(), None);
        ui.join().unwrap();
    }

    /// A non-owning consumer's request returns Err("viewport busy") WITHOUT touching
    /// the event channel, while another consumer holds the viewport.
    #[test]
    fn test_ui_bridge_request_busy() {
        let (event_tx, event_rx) = channel::<AgentEvent>();
        let (_cmd_tx, cmd_rx) = channel::<UiCommand>();
        let token = ViewportToken::new();
        let _held = token.try_claim(1).unwrap(); // another consumer owns it

        let bridge = UiBridge::new(event_tx, cmd_rx, token, 2);
        let err = bridge
            .request(
                |rid| AgentEvent::ScreenshotRequest { request_id: rid },
                |_| true,
                Duration::from_secs(1),
            )
            .unwrap_err();
        assert_eq!(err, "viewport busy");
        // No event was sent (the guard failed before the send).
        assert!(event_rx.try_recv().is_err());
    }
}
