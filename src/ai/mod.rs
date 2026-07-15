//! AI-powered analysis and highlighting for profiling data.
//!
//! This module provides:
//! - Chat panel UI (`chat_panel`)
//! - Native Rust AI agent + direct tool calls (`agent`, `tools`)
//!
//! # Conventions
//!
//! **Errors** are `Result<_, String>` throughout: the messages are written for
//! their actual consumers — the model (tool results) and the chat transcript —
//! and never cross a typed error boundary, so a structured error type would add
//! conversion noise without a consumer.
//!
//! **Locks**: `.lock().unwrap()` is deliberate fail-fast. Every critical
//! section in this layer is short and non-reentrant; a poisoned lock means a
//! holder panicked mid-update, and limping on with torn agent/UI state is worse
//! than crashing.

pub mod agent;
pub mod bridge;
mod chat_panel;
/// The Claude Code backend: spawns the user's own `claude` as a persistent
/// stream-json subprocess wired to the in-viewer MCP server, with a per-call
/// approval bridge for action tools.
#[cfg(feature = "viewer-mcp")]
pub mod claude_code;
/// Transport-agnostic MCP dispatch core (needs duckdb for the query tools); wrapped
/// by the stdio bin and the in-viewer HTTP server.
#[cfg(feature = "duckdb")]
pub mod mcp_core;
pub mod tools;
pub mod trace;
#[cfg(feature = "viewer-mcp")]
pub mod viewer_mcp;

use crate::data::ItemUID;
use crate::timestamp::Interval;

/// Truncate `s` to at most `max` bytes, backing up to the nearest UTF-8 char
/// boundary so the result is always valid UTF-8. A fixed-offset byte slice
/// (`&s[..max]`) panics — aborting the whole viewer — the instant `max` lands
/// inside a multi-byte character, which profiler text (µs, em-dashes, arrows,
/// bullets) hits routinely. This never panics; it returns `s` unchanged when it
/// already fits. Every truncation of model/user text in this layer must route
/// through here.
pub(crate) fn truncate_on_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod truncate_tests {
    use super::truncate_on_boundary;

    #[test]
    fn never_splits_a_multibyte_char() {
        // 'µ' (U+00B5) is 2 bytes; a string of them has NO char boundary at any
        // odd byte, so a fixed &s[..N] with odd N would panic. Sweep every cut.
        let s = "µ".repeat(50); // 100 bytes
        for max in 0..=s.len() {
            let out = truncate_on_boundary(&s, max);
            assert!(out.len() <= max);
            assert!(s.starts_with(out), "must be a prefix");
            // The real proof: it produced a valid &str for every cut (no panic).
        }
    }

    #[test]
    fn passes_through_when_it_fits_and_backs_up_when_it_doesnt() {
        assert_eq!(truncate_on_boundary("abc", 10), "abc");
        assert_eq!(truncate_on_boundary("abc", 3), "abc");
        // em-dash '—' is 3 bytes (E2 80 94): cutting at 1 or 2 backs up to 0.
        assert_eq!(truncate_on_boundary("—x", 2), "");
        assert_eq!(truncate_on_boundary("—x", 3), "—");
    }
}

/// A highlighted region on the timeline (a task, an idle gap, or a region). Every
/// highlight renders as the SAME uniform light-red overlay — no severity, no color.
/// Managed via the left-panel highlight manager (toggle / clear / zoom).
#[derive(Debug, Clone)]
pub struct AiHighlight {
    /// Monotonic unique id — stable ordering key for the manager list.
    pub id: u64,
    /// The time interval this highlight covers.
    pub interval: Interval,
    /// Human-readable description (shown as the manager row label).
    pub label: String,
    /// Optional task target. `None` (today's region/interval highlights) ⇒ the
    /// manager zooms to the interval only; `Some` ⇒ also scroll-to-item.
    pub item_uid: Option<ItemUID>,
    /// Whether this highlight is rendered (manager checkbox). Cleared highlights are
    /// removed; disabled ones stay in the list but don't draw.
    pub enabled: bool,
}

pub use agent::{AgentEvent, Highlight, SelectedItemInfo, UiCommand};
pub use chat_panel::{
    ChatPanel, HighlightAction, PendingNavigation, SelectedItem, TimelineSelection,
};
