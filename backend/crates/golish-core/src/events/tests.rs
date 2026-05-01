//! Snapshot + roundtrip tests for the event wire format.
//!
//! These tests capture the exact JSON structure shipped to the frontend; if any
//! of them fail after a backend change, the frontend contract has been broken.

use super::*;
use serde_json::json;

/// Baseline snapshot tests for AiEvent JSON serialization.
///
/// These tests capture the exact JSON format that the frontend expects.
/// They MUST pass before AND after any migration (e.g., HTTP/SSE server).
///
/// If a test fails after a change, it means the frontend contract has been broken
/// and the frontend code will need to be updated as well.
mod json_serialization;

/// Tests for ToolSource JSON serialization
mod tool_source_serialization;

/// Tests for complete roundtrip (serialize -> deserialize)
mod roundtrip;

/// Tests for AiEventEnvelope serialization
mod envelope_serialization;
