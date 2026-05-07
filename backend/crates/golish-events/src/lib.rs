//! Event coordination and transcript system for Golish AI.
//!
//! Provides:
//! - **DomainEvent**: Top-level event enum unifying all domain events
//! - **EventCoordinator**: Single-task message-passing coordinator for AI events
//! - **TranscriptWriter**: Persists AI events to disk in JSONL format

pub mod domain_event;
pub mod event_coordinator;
pub mod transcript;

pub use domain_event::{DomainEvent, IndexerEvent, PentestEvent, PipelineEvent, SidecarEvent};
pub use event_coordinator::{CoordinatorHandle, CoordinatorState, EventCoordinator};
pub use transcript::{
    build_summarizer_input, format_for_summarizer, read_transcript, save_summarizer_input,
    save_summary, should_transcript, transcript_path, TranscriptEvent, TranscriptWriter,
};
