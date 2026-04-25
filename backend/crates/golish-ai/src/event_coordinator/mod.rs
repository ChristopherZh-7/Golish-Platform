//! Event Coordinator — single-task message-passing coordinator for AI events.
//!
//! Centralizes event-related state (sequence numbers, frontend-ready flag,
//! event buffer, pending approvals) into one tokio task that processes
//! commands in deterministic order, eliminating the deadlock possibilities
//! you can get with shared lock-based mutable state.
//!
//! # Architecture
//!
//! ```text
//! AgentBridge                          EventCoordinator (single tokio task)
//! ┌─────────────────┐                  ┌─────────────────────────────────┐
//! │ coordinator:    │───send()────────▶│ Owns:                            │
//! │ CoordinatorHandle                  │  - event_sequence: u64           │
//! └─────────────────┘                  │  - frontend_ready: bool          │
//!                                      │  - event_buffer: Vec<Envelope>   │
//!                                      │  - pending_approvals: HashMap    │
//!                                      │ Emits via:                       │
//!                                      │  - runtime: Arc<dyn GolishRuntime>│
//!                                      └─────────────────────────────────┘
//! ```
//!
//! # Submodules
//!
//! - [`commands`]: [`CoordinatorCommand`] enum + [`CoordinatorState`] snapshot.
//! - [`handle`]: [`CoordinatorHandle`] (cheap-to-clone send-side API).
//! - [`coordinator`]: [`EventCoordinator`] struct + spawn + the command loop.

mod commands;
mod coordinator;
mod handle;

#[cfg(test)]
mod tests;

pub use commands::{CoordinatorCommand, CoordinatorState};
pub use coordinator::EventCoordinator;
pub use handle::CoordinatorHandle;
