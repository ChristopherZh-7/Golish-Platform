//! Event capture bridge for the sidecar system.
//!
//! This module provides the integration point between the agentic loop and
//! the sidecar event capture system. Split into thematic submodules:
//!
//! - [`context`]: [`CaptureContext`] — the per-turn state machine that
//!   correlates tool requests with results and forwards `SessionEvent`s.
//! - [`extractors`]: pluck file paths / rename operations / output text out
//!   of raw tool args/results.
//! - [`tool_classification`]: classify tool names as read / write / edit.
//! - [`diff`]: unified-diff generation for write/edit captures.
//! - [`format`]: arg summarization, decision-type inference, truncation.

mod context;
mod diff;
mod extractors;
mod format;
mod tool_classification;

#[cfg(test)]
mod tests;

pub use context::CaptureContext;
