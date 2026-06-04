//! Subtask execution phases for [`TaskOrchestrator`].
//!
//! The per-stage execution logic (enrichment, planning, reflector retry, the
//! stage gate, and the Executor-driven operation loop) lives in [`execute`].

mod execute;
