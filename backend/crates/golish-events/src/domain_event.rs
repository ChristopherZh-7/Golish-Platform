//! Top-level domain event enum that unifies all event types.
//!
//! All event emission should eventually route through [`DomainEvent`] so that
//! the frontend has a single, typed event channel instead of ad-hoc
//! `serde_json::Value` payloads.
//!
//! ## Migration plan
//!
//! 1. **Phase 1 (current)**: Define `DomainEvent` with `Ai(AiEvent)` as the
//!    primary variant. Other domains start with opaque payloads.
//! 2. **Phase 2**: Replace `EventEmitterHandle::emit("ai-event", &ai_event)`
//!    calls with `emit_domain(DomainEvent::Ai(ai_event))`.
//! 3. **Phase 3**: Define typed `PentestEvent`, `PipelineEvent`, etc. in
//!    their respective domain crates and add them as variants here.

use golish_core::events::AiEvent;
use serde::{Deserialize, Serialize};

/// Top-level event enum. Each variant wraps a domain-specific event payload.
///
/// Tagged with `"domain"` so the frontend can `switch` on the discriminant
/// and dispatch to the correct handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "domain", content = "payload")]
pub enum DomainEvent {
    /// AI agent lifecycle, tool calls, streaming, sub-agents, etc.
    Ai(AiEvent),

    /// Pentest tool lifecycle events (scan started, progress, completed).
    Pentest(PentestEvent),

    /// Pipeline orchestration events (step started, completed, error).
    Pipeline(PipelineEvent),

    /// Code indexer events (indexing started, progress, completed).
    Indexer(IndexerEvent),

    /// Sidecar context capture events.
    Sidecar(SidecarEvent),
}

impl DomainEvent {
    pub fn domain_name(&self) -> &'static str {
        match self {
            Self::Ai(_) => "ai",
            Self::Pentest(_) => "pentest",
            Self::Pipeline(_) => "pipeline",
            Self::Indexer(_) => "indexer",
            Self::Sidecar(_) => "sidecar",
        }
    }
}

impl From<AiEvent> for DomainEvent {
    fn from(event: AiEvent) -> Self {
        Self::Ai(event)
    }
}

/// Pentest domain events — stub for Phase 2 migration.
///
/// Will be replaced with a proper enum from `golish-pentest` or
/// `golish-scan-runner` once those crates define their event types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PentestEvent {
    ScanStarted { target: String, tool: String },
    ScanProgress { target: String, tool: String, percent: u8 },
    ScanCompleted { target: String, tool: String, findings_count: u32 },
    ScanError { target: String, tool: String, error: String },
    ToolInstalled { tool_id: String },
    ToolRemoved { tool_id: String },
}

/// Pipeline orchestration events — stub for Phase 2 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineEvent {
    PipelineStarted { pipeline_id: String, name: String },
    StepStarted { pipeline_id: String, step_name: String },
    StepCompleted { pipeline_id: String, step_name: String },
    StepError { pipeline_id: String, step_name: String, error: String },
    PipelineCompleted { pipeline_id: String },
    PipelineCancelled { pipeline_id: String },
}

/// Indexer events — stub for Phase 2 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IndexerEvent {
    IndexingStarted { path: String },
    IndexingProgress { path: String, files_processed: u32, total_files: u32 },
    IndexingCompleted { path: String, files_indexed: u32 },
    IndexingError { path: String, error: String },
}

/// Sidecar events — stub for Phase 2 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarEvent {
    Connected { session_id: String },
    Disconnected { session_id: String },
    CaptureStarted { session_id: String },
    CaptureCompleted { session_id: String },
}
