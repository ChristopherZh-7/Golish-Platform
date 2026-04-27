//! Session persistence extension for AgentBridge.
//!
//! This module contains methods for managing conversation session persistence.

use std::path::PathBuf;

use rig::completion::Message;

use super::agent_bridge::AgentBridge;
use golish_session::GolishSessionManager;

impl AgentBridge {
    // ========================================================================
    // Session Persistence Methods
    // ========================================================================

    pub async fn set_session_persistence_enabled(&self, enabled: bool) {
        *self.session.session_persistence_enabled.write().await = enabled;
        tracing::debug!("Session persistence enabled: {}", enabled);
    }

    pub async fn is_session_persistence_enabled(&self) -> bool {
        *self.session.session_persistence_enabled.read().await
    }

    pub(crate) async fn start_session(&self) {
        if !*self.session.session_persistence_enabled.read().await {
            return;
        }

        let mut manager_guard = self.session.session_manager.write().await;
        if manager_guard.is_some() {
            return;
        }

        let workspace = self.workspace.read().await.clone();
        match GolishSessionManager::new(workspace, &self.llm.model_name, &self.llm.provider_name).await {
            Ok(mut manager) => {
                if let Some(ref pool) = self.services.db_pool {
                    manager.set_db_pool(pool.clone());
                }
                *manager_guard = Some(manager);
                tracing::debug!("Session started for persistence");
            }
            Err(e) => {
                tracing::warn!("Failed to start session for persistence: {}", e);
            }
        }
    }

    pub(crate) async fn with_session_manager<F>(&self, f: F)
    where
        F: FnOnce(&mut GolishSessionManager),
    {
        let mut guard = self.session.session_manager.write().await;
        if let Some(ref mut manager) = *guard {
            f(manager);
        }
    }

    /// Record a user message in the current session.
    pub(crate) async fn record_user_message(&self, content: &str) {
        self.with_session_manager(|m| m.add_user_message(content))
            .await;
    }

    /// Record an assistant message in the current session.
    pub(crate) async fn record_assistant_message(&self, content: &str) {
        self.with_session_manager(|m| m.add_assistant_message(content))
            .await;
    }

    /// Update the session workspace path.
    pub(crate) async fn update_session_workspace(&self, new_path: PathBuf) {
        let mut guard = self.session.session_manager.write().await;
        if let Some(ref mut manager) = *guard {
            manager.update_workspace(new_path).await;
        }
    }

    pub(crate) async fn save_session(&self) {
        self.sync_agent_mode_to_session().await;

        let manager_guard = self.session.session_manager.read().await;
        if let Some(ref manager) = *manager_guard {
            if let Err(e) = manager.save() {
                tracing::warn!("Failed to save session: {}", e);
            }
        }
    }

    pub async fn finalize_session(&self) -> Option<PathBuf> {
        self.sync_agent_mode_to_session().await;

        let mut manager_guard = self.session.session_manager.write().await;
        if let Some(ref mut manager) = manager_guard.take() {
            match manager.finalize() {
                Ok(path) => {
                    tracing::info!("Session finalized: {}", path.display());
                    return Some(path);
                }
                Err(e) => {
                    tracing::warn!("Failed to finalize session: {}", e);
                }
            }
        }
        None
    }

    async fn sync_agent_mode_to_session(&self) {
        let mode = self.access.agent_mode.read().await;
        let mode_str = mode.to_string();
        drop(mode);

        let mut guard = self.session.session_manager.write().await;
        if let Some(ref mut manager) = *guard {
            manager.set_agent_mode(mode_str);
        }
    }

    // ========================================================================
    // Conversation History Methods
    // ========================================================================

    pub async fn clear_conversation_history(&self) {
        self.finalize_session().await;

        let mut history = self.session.conversation_history.write().await;
        history.clear();
        tracing::debug!("Conversation history cleared");
    }

    pub async fn conversation_history_len(&self) -> usize {
        self.session.conversation_history.read().await.len()
    }

    pub async fn restore_session(&self, messages: Vec<golish_session::GolishSessionMessage>) {
        self.finalize_session().await;

        let rig_messages: Vec<Message> =
            messages.iter().filter_map(|m| m.to_rig_message()).collect();

        let mut history = self.session.conversation_history.write().await;
        *history = rig_messages;

        tracing::info!(
            "Restored session with {} messages ({} in history)",
            messages.len(),
            history.len()
        );
    }
}
