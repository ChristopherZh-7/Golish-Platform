//! Request and response types for the Anthropic Vertex AI API.
//!
//! Originally one 654-line file. Split here by concern; all
//! previously-public types are re-exported flat at this `mod.rs`
//! level so callers continue to see the stable
//! `rig_anthropic_vertex::types::*` surface.
//!
//! - [`messages`]: thinking / cache / system blocks + `ContentBlock`,
//!   `ImageSource`, `Role`, `Message` (the core message-shape types).
//! - [`request`]: `ToolDefinition` + `CompletionRequest` (request body).
//! - [`response`]: `Usage`, `StopReason`, `CompletionResponse` (with
//!   text/tool-uses/thinking accessors).
//! - [`streaming`]: SSE event types (`StreamEvent`, `ContentDelta`,
//!   `Citation`, `StreamError`, …).
//! - [`web_tools`]: Claude-native server tools — web_search /
//!   web_fetch — plus their result-content union types.

mod messages;
mod request;
mod response;
mod streaming;
mod web_tools;

#[cfg(test)]
mod tests;

pub use messages::*;
pub use request::*;
pub use response::*;
pub use streaming::*;
pub use web_tools::*;

/// Anthropic API version for Vertex AI.
pub const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

/// Maximum tokens default.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;
