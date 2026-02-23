//! AI-powered analysis and highlighting for profiling data.
//!
//! This module provides:
//! - Chat panel UI (`chat_panel`)
//! - Native Rust AI agent + direct tool calls (`agent`, `tools`)

pub mod agent;
mod chat_panel;
pub mod tools;

use crate::timestamp::Interval;

/// A highlighted region on the timeline indicating a performance issue.
#[derive(Debug, Clone)]
pub struct AiHighlight {
    /// The time interval this highlight covers.
    pub interval: Interval,
    /// Color for rendering the highlight overlay.
    pub color: egui::Color32,
    /// Human-readable description.
    pub label: String,
    /// Confidence score in [0.0, 1.0] range.
    pub confidence: f32,
}

pub use agent::{AgentEvent, Highlight, UiCommand};
pub use chat_panel::{ChatMessage, ChatMessageKind, ChatPanel, HighlightAction, TimelineSelection};
