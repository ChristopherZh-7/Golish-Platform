//! Event types captured by the sidecar system.
//!
//! These types represent:
//! 1. Session events - semantic information extracted from agent interactions (for storage/query)
//! 2. UI events - notifications emitted to the frontend for real-time updates
#![allow(dead_code)]

mod ui_events;
mod event_type;
mod session_event;
mod checkpoint;
mod commit_boundary;
mod export;
pub(crate) mod helpers;

pub use ui_events::SidecarEvent;
pub use event_type::{EventType, FileOperation, DecisionType, FeedbackType};
pub use session_event::SessionEvent;
pub use checkpoint::{Checkpoint, SidecarSession};
pub use commit_boundary::{CommitBoundaryDetector, CommitBoundaryInfo};
pub use export::SessionExport;

#[cfg(test)]
mod tests;
