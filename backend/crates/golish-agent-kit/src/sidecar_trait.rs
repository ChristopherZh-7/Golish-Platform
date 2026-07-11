//! Trait abstraction for the sidecar session capture system.
//!
//! Decouples `golish-ai` from `golish-sidecar` so the AI crate only knows
//! about session lifecycle and event capture through these traits, while the
//! concrete implementation lives in the application layer.

use golish_core::events::AiEvent;

/// Abstraction over the sidecar session capture backend.
///
/// Manages session lifecycle (start / end / current) and captures events
/// produced during an AI agent turn.
#[async_trait::async_trait]
pub trait SessionCaptureBackend: Send + Sync {
    /// Get the current active session ID, if any.
    fn current_session_id(&self) -> Option<String>;

    /// Start a new capture session with the given initial prompt.
    /// Returns the new session ID.
    fn start_session(&self, initial_request: &str) -> anyhow::Result<String>;

    /// End the current session. Returns info about the ended session, or
    /// `None` if no session was active.
    fn end_session(&self) -> anyhow::Result<Option<EndedSessionInfo>>;

    /// Resume an existing capture session owned by this backend instance.
    fn resume_session(&self, session_id: &str) -> anyhow::Result<()>;

    /// Find a legacy capture session by workspace and approximate start time.
    async fn find_matching_session(
        &self,
        workspace_path: &std::path::Path,
        started_at: chrono::DateTime<chrono::Utc>,
        tolerance_secs: Option<i64>,
    ) -> anyhow::Result<Option<String>>;

    /// Capture a user prompt event for the given session.
    fn capture_user_prompt(&self, session_id: &str, text: &str);

    /// Capture an AI response event for the given session.
    fn capture_ai_response(&self, session_id: &str, text: &str);

    /// Process a single AI event (stateless, one-shot capture).
    ///
    /// Used for events like `Reasoning` that don't need cross-event
    /// state correlation.
    fn capture_event(&self, event: &AiEvent);

    /// Create a stateful event processor for the agentic loop.
    ///
    /// The processor maintains state across events (e.g. correlating
    /// tool-request → tool-result pairs) and is kept alive for the
    /// duration of a single agentic-loop iteration.
    fn create_event_processor(&self) -> Box<dyn AiEventProcessor>;

    /// Get the injectable session context (e.g. state.md content) for
    /// prompt injection. Returns `None` when no active session exists.
    async fn get_injectable_context(&self) -> anyhow::Result<Option<String>>;
}

/// Stateful processor for `AiEvent`s during the agentic loop.
///
/// Unlike the one-shot `capture_event`, this correlates events across
/// a single loop iteration (e.g. matching tool requests with their results).
pub trait AiEventProcessor: Send {
    fn process(&mut self, event: &AiEvent);
}

/// Minimal info about a session that was ended.
pub struct EndedSessionInfo {
    pub session_id: String,
}
