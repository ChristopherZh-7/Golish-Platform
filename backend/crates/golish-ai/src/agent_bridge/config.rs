//! Configuration setters, public accessors, and skill discovery for [`AgentBridge`].
//!
//! Methods that wire optional services (DB pool, PTY manager, sidecar, transcript
//! writer, ...) into a constructed bridge live here, alongside skill-cache
//! refresh / matching logic and the public read-only accessors used by the
//! `golish` crate.

use std::path::PathBuf;
use std::sync::Arc;

use golish_core::{ApiRequestStats, PromptMatchedSkill, PromptSkillInfo};
use golish_pty::PtyManager;
use golish_sidecar::SidecarState;
use golish_skills::SkillMetadata;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;
use tokio::sync::RwLock;

use super::super::agent_mode::AgentMode;
use super::super::execution_mode::ExecutionMode;
use super::super::indexer::IndexerState;
use super::super::llm_client::{LlmClient, LlmClientFactory};
use super::super::planner::PlanManager;
use super::super::transcript::TranscriptWriter;

use super::AgentBridge;

impl AgentBridge {
    // ========================================================================
    // Database / persistence
    // ========================================================================

    /// Get a clone of the database pool (if available).
    pub fn db_pool(&self) -> Option<Arc<sqlx::PgPool>> {
        self.db_pool.clone()
    }

    /// Set the database pool for session persistence dual-write and activity tracking.
    pub fn set_db_pool(
        &mut self,
        pool: Arc<sqlx::PgPool>,
        ready_gate: golish_db::DbReadyGate,
    ) {
        let session_uuid = uuid::Uuid::new_v4();
        let ws = self.workspace.try_read().ok();
        let project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.db_tracker = Some(
            crate::db_tracking::DbTracker::new(pool.clone(), session_uuid, ready_gate.clone())
                .with_project_path(project_path),
        );
        self.db_pool = Some(pool.clone());

        // Load prompt template overrides from DB (non-blocking)
        let prompt_reg = self.prompt_registry.clone();
        let pool_for_prompts = pool.clone();
        let sub_reg = self.sub_agent_registry.clone();
        tokio::spawn(async move {
            if let Err(e) = prompt_reg.load_db_overrides(&pool_for_prompts).await {
                tracing::warn!("[prompt-registry] Failed to load DB overrides: {e}");
            } else {
                let new_agents = golish_sub_agents::defaults::create_default_sub_agents_from_registry(&prompt_reg).await;
                let mut reg = sub_reg.write().await;
                reg.register_multiple(new_agents);
                tracing::info!("[prompt-registry] Reloaded sub-agents with DB template overrides");
            }
        });

        // Wire up PlanManager with DB persistence
        let ws = self.workspace.try_read().ok();
        let plan_project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.plan_manager = Arc::new(
            PlanManager::new().with_db(pool.clone(), Some(session_uuid), plan_project_path),
        );

        let plan_manager = self.plan_manager.clone();
        let pool_for_session = pool.clone();
        let mut gate = ready_gate;
        tokio::spawn(async move {
            if !gate.is_ready() {
                if tokio::time::timeout(std::time::Duration::from_secs(60), gate.wait())
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = sqlx::query("INSERT INTO sessions (id) VALUES ($1) ON CONFLICT DO NOTHING")
                .bind(session_uuid)
                .execute(pool_for_session.as_ref())
                .await;

            // Load any active plan from the previous session
            plan_manager.load_from_db().await;
        });
    }

    // ========================================================================
    // Optional service wiring
    // ========================================================================

    /// Set the PtyManager for executing commands in user's terminal
    pub fn set_pty_manager(&mut self, pty_manager: Arc<PtyManager>) {
        self.pty_manager = Some(pty_manager);
    }

    /// Set the IndexerState for code analysis tools
    pub fn set_indexer_state(&mut self, indexer_state: Arc<IndexerState>) {
        self.indexer_state = Some(indexer_state);
    }

    /// Set the SidecarState for context capture
    pub fn set_sidecar_state(&mut self, sidecar_state: Arc<SidecarState>) {
        self.sidecar_state = Some(sidecar_state);
    }

    /// Set the TranscriptWriter for persisting AI events to JSONL.
    pub fn set_transcript_writer(&mut self, writer: TranscriptWriter, base_dir: PathBuf) {
        let writer = Arc::new(writer);
        // Forward to coordinator so bridge-level events (UserMessage, Completed, etc.)
        // are also written to the transcript.
        if let Some(ref coordinator) = self.coordinator {
            coordinator.set_transcript_writer(Arc::clone(&writer));
        }
        self.transcript_writer = Some(writer);
        self.transcript_base_dir = Some(base_dir);
    }

    /// Set the memory file path for project instructions.
    /// This overrides the default CLAUDE.md lookup.
    pub async fn set_memory_file_path(&self, path: Option<PathBuf>) {
        *self.memory_file_path.write().await = path;
    }

    /// Set the SettingsManager for dynamic memory file lookup.
    pub fn set_settings_manager(
        &mut self,
        settings_manager: Arc<golish_settings::SettingsManager>,
    ) {
        self.settings_manager = Some(settings_manager);
    }

    /// Attach an embedder to the DB tracker for semantic memory operations.
    pub fn set_embedder(&mut self, embedder: Arc<dyn golish_db::embeddings::Embedder>) {
        if let Some(ref mut tracker) = self.db_tracker {
            tracker.set_embedder(embedder);
        }
    }

    /// Get the memory file path dynamically from current settings.
    /// This ensures we always use the latest settings, even if they changed
    /// after the AI session was initialized.
    /// Falls back to cached value if `settings_manager` is not available.
    pub(super) async fn get_memory_file_path_dynamic(&self) -> Option<PathBuf> {
        if let Some(ref settings_manager) = self.settings_manager {
            let workspace_path = self.workspace.read().await;
            let settings = settings_manager.get().await;
            if let Some(path) = crate::memory_file::find_memory_file_for_workspace(
                &workspace_path,
                &settings.codebases,
            ) {
                return Some(path);
            }
        }

        self.memory_file_path.read().await.clone()
    }

    /// Set the current session ID for terminal execution
    pub async fn set_session_id(&self, session_id: Option<String>) {
        *self.current_session_id.write().await = session_id;
    }

    /// Update the workspace/working directory.
    /// Also updates the tool registry's workspace so file operations
    /// use the new directory as the base for relative paths.
    pub async fn set_workspace(&self, new_workspace: PathBuf) {
        {
            let current = self.workspace.read().await;
            if *current == new_workspace {
                tracing::trace!(
                    "[cwd-sync] Workspace unchanged, skipping update: {}",
                    new_workspace.display()
                );
                return;
            }
        }

        {
            let mut workspace = self.workspace.write().await;
            *workspace = new_workspace.clone();
        }

        {
            let mut registry = self.tool_registry.write().await;
            registry.set_workspace(new_workspace.clone());
        }

        self.update_session_workspace(new_workspace.clone()).await;

        tracing::debug!(
            "[cwd-sync] Updated workspace to: {}",
            new_workspace.display()
        );

        // Refresh skill cache for new workspace.
        // NOTE: Must be called after dropping workspace write lock, as
        // refresh_skills acquires workspace read lock internally.
        self.refresh_skills().await;
    }

    // ========================================================================
    // Skills
    // ========================================================================

    /// Refresh the skill cache for the current workspace.
    ///
    /// This discovers skills from both global (~/.golish/skills/) and local
    /// (<workspace>/.golish/skills/) directories and caches their metadata
    /// for efficient matching.
    pub async fn refresh_skills(&self) {
        let workspace = self.workspace.read().await;
        let workspace_str = workspace.to_string_lossy().to_string();
        drop(workspace);

        // Run discover_skills in a blocking thread to avoid blocking the tokio runtime.
        // discover_skills scans directories synchronously.
        let workspace_str_clone = workspace_str.clone();
        let skills = match tokio::task::spawn_blocking(move || {
            golish_skills::discover_skills(Some(&workspace_str_clone))
        })
        .await
        {
            Ok(skills) => skills,
            Err(e) => {
                tracing::warn!("[refresh_skills] Failed to discover skills: {}", e);
                return;
            }
        };

        let metadata: Vec<SkillMetadata> = skills.into_iter().map(Into::into).collect();

        *self.skill_cache.write().await = metadata.clone();
        tracing::debug!(
            "[skills] Refreshed skill cache: {} skills discovered",
            metadata.len()
        );
    }

    /// Match skills against a user prompt and load their bodies.
    ///
    /// This is the progressive loading implementation:
    /// 1. Uses cached skill metadata for efficient matching
    /// 2. Only loads full skill bodies for matched skills
    ///
    /// Returns `(available_skills, matched_skills)` for `PromptContext`.
    pub(super) async fn match_and_load_skills(
        &self,
        prompt: &str,
    ) -> (Vec<PromptSkillInfo>, Vec<PromptMatchedSkill>) {
        let skill_cache = self.skill_cache.read().await;

        if skill_cache.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let available_skills: Vec<PromptSkillInfo> = skill_cache
            .iter()
            .map(|s| PromptSkillInfo {
                name: s.name.clone(),
                description: s.description.clone(),
            })
            .collect();

        let matcher = golish_skills::SkillMatcher::default();
        let matches = matcher.match_skills(prompt, &skill_cache);

        if matches.is_empty() {
            tracing::debug!("[skills] No skills matched for prompt");
            return (available_skills, Vec::new());
        }

        tracing::debug!(
            "[skills] {} skills matched for prompt: {:?}",
            matches.len(),
            matches.iter().map(|(s, _, _)| &s.name).collect::<Vec<_>>()
        );

        let mut matched_skills = Vec::new();
        for (meta, score, reason) in matches {
            match golish_skills::load_skill_body(&meta.path) {
                Ok(body) => {
                    matched_skills.push(PromptMatchedSkill {
                        name: meta.name.clone(),
                        description: meta.description.clone(),
                        body,
                        match_score: score,
                        match_reason: reason,
                    });
                }
                Err(e) => {
                    tracing::warn!("[skills] Failed to load body for '{}': {}", meta.name, e);
                }
            }
        }

        (available_skills, matched_skills)
    }

    // ========================================================================
    // Mode toggles
    // ========================================================================

    /// Set the agent mode. Controls how tool approvals are handled.
    pub async fn set_agent_mode(&self, mode: AgentMode) {
        let mut current = self.agent_mode.write().await;
        tracing::debug!("Agent mode changed: {} -> {}", *current, mode);
        *current = mode;
    }

    /// Get the current agent mode.
    pub async fn get_agent_mode(&self) -> AgentMode {
        *self.agent_mode.read().await
    }

    /// Set the useAgents flag (controls whether sub-agent delegation is available).
    pub async fn set_use_agents(&self, enabled: bool) {
        let mut current = self.use_agents.write().await;
        tracing::debug!("useAgents changed: {} -> {}", *current, enabled);
        *current = enabled;
    }

    /// Get the current useAgents setting.
    pub async fn get_use_agents(&self) -> bool {
        *self.use_agents.read().await
    }

    /// Set the execution mode (Chat vs Task).
    pub async fn set_execution_mode(&self, mode: ExecutionMode) {
        let mut current = self.execution_mode.write().await;
        tracing::debug!("Execution mode changed: {} -> {}", *current, mode);
        *current = mode;
    }

    /// Get the current execution mode.
    pub async fn get_execution_mode(&self) -> ExecutionMode {
        *self.execution_mode.read().await
    }

    // ========================================================================
    // System prompt (lightweight standalone variant)
    // ========================================================================

    /// Build the system prompt for the agent.
    ///
    /// This is a simplified version of the prompt building logic from
    /// `prepare_execution_context`.
    pub async fn build_system_prompt(&self) -> String {
        use super::super::system_prompt::build_system_prompt_with_contributions;

        let workspace_path = self.workspace.read().await;
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            None,
            None,
        )
    }

    // ========================================================================
    // Public Accessors (for the `golish` crate)
    // ========================================================================

    /// Get the sub-agent registry.
    pub fn sub_agent_registry(&self) -> &Arc<RwLock<SubAgentRegistry>> {
        &self.sub_agent_registry
    }

    /// Get the prompt template registry.
    pub fn prompt_registry(&self) -> &golish_sub_agents::PromptRegistry {
        &self.prompt_registry
    }

    /// Get the provider name.
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Get the plan manager.
    pub fn plan_manager(&self) -> &Arc<PlanManager> {
        &self.plan_manager
    }

    /// Get the LLM client.
    pub fn client(&self) -> &Arc<RwLock<LlmClient>> {
        &self.client
    }

    /// Get the tool registry.
    pub fn tool_registry(&self) -> &Arc<RwLock<ToolRegistry>> {
        &self.tool_registry
    }

    /// Get the workspace path.
    pub fn workspace(&self) -> &Arc<RwLock<PathBuf>> {
        &self.workspace
    }

    /// Get the indexer state.
    pub fn indexer_state(&self) -> Option<&Arc<IndexerState>> {
        self.indexer_state.as_ref()
    }

    /// Get the model factory (for sub-agent model overrides).
    pub fn model_factory(&self) -> Option<&Arc<LlmClientFactory>> {
        self.model_factory.as_ref()
    }

    /// Set the model factory for sub-agent model overrides.
    pub fn set_model_factory(&mut self, factory: Arc<LlmClientFactory>) {
        self.model_factory = Some(factory);
    }

    /// Override the tool configuration (e.g. to disable all tools for title-gen sessions).
    pub fn set_tool_config(&mut self, config: crate::tool_definitions::ToolConfig) {
        self.tool_config = config;
    }

    pub fn event_session_id(&self) -> Option<&str> {
        self.event_session_id.as_deref()
    }

    pub fn transcript_base_dir(&self) -> Option<&std::path::Path> {
        self.transcript_base_dir.as_deref()
    }

    pub fn api_request_stats(&self) -> &Arc<ApiRequestStats> {
        &self.api_request_stats
    }

    /// Get the current MCP tool definitions.
    /// Returns a clone of the tool definitions for external inspection.
    pub async fn mcp_tool_definitions(&self) -> Vec<rig::completion::ToolDefinition> {
        self.mcp_tool_definitions.read().await.clone()
    }

    /// Set MCP tool executor for handling MCP tool calls.
    /// Should be called together with `set_mcp_tools`.
    /// Takes `&self` (uses interior mutability) so it can be called after bridge creation.
    #[allow(clippy::type_complexity)]
    pub async fn set_mcp_executor(
        &self,
        executor: Arc<
            dyn Fn(
                    &str,
                    &serde_json::Value,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
                > + Send
                + Sync,
        >,
    ) {
        *self.mcp_tool_executor.write().await = Some(executor);
    }
}
