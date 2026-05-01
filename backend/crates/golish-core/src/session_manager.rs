//! Session manager abstraction trait.
//!
//! Provides a provider-agnostic interface for session persistence.
//! The concrete implementation lives in `golish-session`.

use std::path::PathBuf;

/// Trait for managing conversation session persistence.
///
/// `golish-ai` depends on this trait; the concrete implementation
/// (file-based session storage) is injected by the application layer.
pub trait SessionManager: Send + Sync {
    fn add_user_message(&mut self, content: &str);
    fn add_assistant_message(&mut self, content: &str);
    fn update_workspace_sync(&mut self, path: PathBuf);
    fn save(&self) -> anyhow::Result<()>;
    fn finalize(&mut self) -> anyhow::Result<PathBuf>;
    fn set_agent_mode(&mut self, mode: String);
    fn set_sidecar_session_id(&mut self, id: String);
}

/// Factory trait for creating session managers.
#[async_trait::async_trait]
pub trait SessionManagerFactory: Send + Sync {
    async fn create(
        &self,
        workspace: PathBuf,
        model: &str,
        provider: &str,
    ) -> anyhow::Result<Box<dyn SessionManager>>;
}
