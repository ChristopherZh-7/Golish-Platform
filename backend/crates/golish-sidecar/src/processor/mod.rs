//! Event processor for simplified sidecar.
//!
//! Processes events asynchronously, updating:
//! - `state.md` with session context
//! - `patches/staged/` with commit patches (L2)
//!
//! ## Module layout
//!
//! - [`session_state`]  — per-session in-memory state, dedup, file tracking
//! - [`event_handler`]  — routes a `SessionEvent` to file/log/state updates
//! - [`synthesis`]      — LLM synthesis for `state.md`, titles, commit messages
//! - [`git`]            — git status/diff helpers used by the processor
//! - [`patches`]        — staged-patch generation orchestration

mod event_handler;
mod git;
mod patches;
mod session_state;
mod synthesis;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use tauri::AppHandle;

use crate::commits::BoundaryReason;
use crate::events::{SessionEvent, SidecarEvent};

use golish_synthesis::SynthesisConfig;

use event_handler::{handle_end_session, handle_event};
use patches::generate_patch;
use session_state::{SessionProcessorState, DUPLICATE_WINDOW_SECS};

/// Event sent to the processor
#[derive(Debug)]
pub enum ProcessorTask {
    /// Process a new event
    ProcessEvent {
        session_id: String,
        event: Box<SessionEvent>,
    },
    /// End a session
    EndSession { session_id: String },
    /// Shutdown the processor
    Shutdown,
}

/// Configuration for the processor
#[derive(Clone)]
pub struct ProcessorConfig {
    /// Directory containing sessions
    pub sessions_dir: PathBuf,
    /// Whether to generate staged patches (L2)
    pub generate_patches: bool,
    /// Synthesis configuration for commit messages
    pub synthesis: SynthesisConfig,
    /// App handle for emitting events (Tauri)
    pub app_handle: Option<Arc<AppHandle>>,
}

impl std::fmt::Debug for ProcessorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessorConfig")
            .field("sessions_dir", &self.sessions_dir)
            .field("generate_patches", &self.generate_patches)
            .field("synthesis", &self.synthesis)
            .finish()
    }
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            sessions_dir: crate::session::default_sessions_dir(),
            generate_patches: true,
            synthesis: SynthesisConfig::default(),
            app_handle: None,
        }
    }
}

impl ProcessorConfig {
    /// Emit a sidecar event to the frontend
    pub fn emit_event(&self, event: SidecarEvent) {
        use tauri::Emitter;
        if let Some(handle) = &self.app_handle {
            if let Err(e) = handle.emit("sidecar-event", &event) {
                tracing::warn!("Failed to emit sidecar event from processor: {}", e);
            }
        }
    }
}

/// Event processor
pub struct Processor {
    task_tx: mpsc::Sender<ProcessorTask>,
    /// Handle to the processor task, used to await completion during shutdown
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Processor {
    /// Create a new processor and spawn its background task
    pub fn spawn(config: ProcessorConfig) -> Self {
        tracing::info!(
            "[processor] Spawning processor: synthesis.enabled={}, synthesis.backend={:?}",
            config.synthesis.enabled,
            config.synthesis.backend
        );

        let (task_tx, task_rx) = mpsc::channel(256);

        let task_handle = tokio::spawn(async move {
            run_processor(config, task_rx).await;
        });

        Self {
            task_tx,
            task_handle: Some(task_handle),
        }
    }

    /// Process an event (non-blocking, queues for async processing)
    pub fn process_event(&self, session_id: String, event: SessionEvent) {
        tracing::trace!(
            "[processor] Queuing event: type={}, session={}",
            event.event_type.name(),
            session_id
        );
        let task = ProcessorTask::ProcessEvent {
            session_id,
            event: Box::new(event),
        };
        if let Err(e) = self.task_tx.try_send(task) {
            tracing::warn!("[processor] Failed to queue event for processing: {}", e);
        }
    }

    /// Signal session end
    pub fn end_session(&self, session_id: String) {
        let task = ProcessorTask::EndSession { session_id };
        if let Err(e) = self.task_tx.try_send(task) {
            tracing::warn!("Failed to queue session end: {}", e);
        }
    }

    /// Shutdown the processor and wait for it to complete all pending work
    ///
    /// This sends a shutdown signal and then waits for the processor task to finish,
    /// ensuring that any pending operations (like patch generation) complete.
    pub async fn shutdown(mut self) {
        let _ = self.task_tx.send(ProcessorTask::Shutdown).await;

        if let Some(handle) = self.task_handle.take() {
            match handle.await {
                Ok(()) => tracing::debug!("Processor task completed successfully"),
                Err(e) => tracing::warn!("Processor task panicked: {}", e),
            }
        }
    }
}

/// Main processor loop
async fn run_processor(config: ProcessorConfig, mut task_rx: mpsc::Receiver<ProcessorTask>) {
    tracing::info!("Sidecar processor started");

    let mut session_states: HashMap<String, SessionProcessorState> = HashMap::new();

    while let Some(task) = task_rx.recv().await {
        match task {
            ProcessorTask::ProcessEvent { session_id, event } => {
                let event_type = event.event_type.name();
                let session_state = session_states
                    .entry(session_id.clone())
                    .or_insert_with(SessionProcessorState::new);

                let is_high_frequency = session_state.track_event_frequency(event_type);
                if is_high_frequency {
                    let count = session_state
                        .recent_event_counts
                        .get(event_type)
                        .copied()
                        .unwrap_or(0);
                    tracing::warn!(
                        session_id = %session_id,
                        event_type = %event_type,
                        count_in_window = count,
                        window_secs = DUPLICATE_WINDOW_SECS,
                        "High frequency events detected - possible duplicate issue"
                    );
                }

                if let Err(e) = handle_event(&config, &session_id, &event, session_state).await {
                    tracing::error!("Failed to process event for {}: {}", session_id, e);
                }
            }
            ProcessorTask::EndSession { session_id } => {
                tracing::info!(
                    "[processor] EndSession task received for session: {}",
                    session_id
                );

                if let Some(session_state) = session_states.get_mut(&session_id) {
                    tracing::info!(
                        "[processor] Session {} ending: generate_patches={}",
                        session_id,
                        config.generate_patches
                    );

                    if config.generate_patches {
                        tracing::info!(
                            "[processor] Generating patch for session {} (using git to detect files)",
                            session_id
                        );
                        if let Err(e) = generate_patch(
                            &config,
                            &session_id,
                            session_state,
                            BoundaryReason::SessionEnd,
                        )
                        .await
                        {
                            tracing::error!(
                                "Failed to generate final patch for {}: {}",
                                session_id,
                                e
                            );
                        }
                    } else {
                        tracing::debug!(
                            "[processor] Patch generation disabled for session {}",
                            session_id
                        );
                    }
                } else {
                    tracing::warn!(
                        "[processor] No session state found for session {}, cannot generate patch",
                        session_id
                    );
                }

                if let Err(e) = handle_end_session(&config, &session_id).await {
                    tracing::error!("Failed to end session {}: {}", session_id, e);
                }

                session_states.remove(&session_id);
            }
            ProcessorTask::Shutdown => {
                tracing::info!("Sidecar processor shutting down");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_processor_lifecycle() {
        let temp = TempDir::new().unwrap();
        let config = ProcessorConfig {
            sessions_dir: temp.path().to_path_buf(),
            generate_patches: true,
            synthesis: SynthesisConfig::default(),
            app_handle: None,
        };

        let processor = Processor::spawn(config);
        processor.shutdown().await;
    }
}
