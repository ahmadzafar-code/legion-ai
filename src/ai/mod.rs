//! AI-powered analysis and highlighting for profiling data.
//!
//! This module provides:
//! - Chat panel UI (`chat_panel`)
//! - Native Rust AI agent + direct tool calls (`agent`, `tools`)

pub mod agent;
pub mod bridge;
mod chat_panel;
/// Backend B ("your Claude Code" embedded chat, P1) — allow/deny tool lists +
/// P0-gate findings. The subprocess driver lands in P2
/// (`IMPLEMENTATION-PLAN-cc-backend.md`).
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
pub use chat_panel::{ChatMessage, ChatMessageKind, ChatPanel, HighlightAction, PendingNavigation, SelectedItem, TimelineSelection};
