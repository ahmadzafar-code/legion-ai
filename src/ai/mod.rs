//! AI-powered analysis and highlighting for profiling data.
//!
//! This module provides automatic detection of performance issues such as
//! idle gaps, dependency waits, and data stalls in Legion profiling traces.
//!
//! Gap diagnosis is handled by the Python sidecar (`sidecar/server.py`) which
//! gives an LLM direct DuckDB query access for expert-level analysis.

mod analyzer;
mod chat_panel;
mod data_wrapper;

pub use analyzer::{get_kind_from_entry_id, AiHighlight, Analyzer, IdleGapAnalyzer};
pub use chat_panel::{ChatMessage, ChatMessageKind, ChatPanel, TimelineSelection};
pub use data_wrapper::AiDataWrapper;
