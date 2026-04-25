//! Wire-format events streamed from `AgentBridge` to the frontend.
//!
//! ## Layout
//!
//! - [`event`]        — the [`AiEvent`] enum + `event_type()` helper (one big enum
//!                       on purpose: it is the wire contract with the frontend)
//! - [`tool_source`]  — [`ToolSource`] (origin of a tool call)
//! - [`envelope`]     — [`AiEventEnvelope`] (seq + ts wrapper for reliable delivery)
//!
//! Public re-exports below mean upstream code can keep using
//! `golish_core::events::{AiEvent, AiEventEnvelope, ToolSource}` (or via the
//! crate-level glob `pub use events::*` in `lib.rs`).

mod envelope;
mod event;
mod tool_source;

pub use envelope::AiEventEnvelope;
pub use event::AiEvent;
pub use tool_source::ToolSource;

#[cfg(test)]
mod tests;
