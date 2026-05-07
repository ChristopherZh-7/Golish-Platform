//! Event types captured by the sidecar system.
//!
//! These types represent:
//! 1. Session events - semantic information extracted from agent interactions (for storage/query)
//! 2. UI events - notifications emitted to the frontend for real-time updates
#![allow(dead_code)]

mod checkpoint;
mod commit_boundary;
mod event_type;
mod export;
pub(crate) mod helpers;
mod session_event;
mod ui_events;

pub use checkpoint::{Checkpoint, SidecarSession};
pub use commit_boundary::{CommitBoundaryDetector, CommitBoundaryInfo};
pub use event_type::{DecisionType, EventType, FeedbackType, FileOperation};
pub use export::SessionExport;
pub use session_event::SessionEvent;
pub use ui_events::SidecarEvent;

#[cfg(test)]
mod tests;
