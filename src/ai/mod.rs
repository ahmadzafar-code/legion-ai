//! AI-powered analysis and highlighting for profiling data.
//!
//! This module provides automatic detection of performance issues such as
//! idle gaps, dependency waits, and data stalls in Legion profiling traces.

mod analyzer;
mod data_wrapper;

pub use analyzer::{get_kind_from_entry_id, AiHighlight, Analyzer, IdleGapAnalyzer};
pub use data_wrapper::AiDataWrapper;
