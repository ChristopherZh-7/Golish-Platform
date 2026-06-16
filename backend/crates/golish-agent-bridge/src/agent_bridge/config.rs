//! Configuration setters, public accessors, and skill discovery for [`AgentBridge`].
//!
//! Methods that wire optional services (DB pool, PTY manager, sidecar, transcript
//! writer, ...) into a constructed bridge live here, alongside skill-cache
//! refresh / matching logic and the public read-only accessors used by the
//! `golish` crate.

use std::path::PathBuf;
use std::sync::Arc;

use golish_core::{ApiRequestStats, PromptMatchedSkill, PromptSkillInfo};

use crate::sidecar_trait::SessionCaptureBackend;
use golish_sub_agents::SubAgentRegistry;
use golish_tools::ToolRegistry;
use tokio::sync::RwLock;

use super::super::agent_mode::AgentMode;
use super::super::execution_mode::ExecutionMode;
use super::super::llm_client::{LlmClient, LlmClientFactory};
use super::super::planner::PlanManager;
use super::super::transcript::TranscriptWriter;
use golish_indexer::IndexerState;

use super::AgentBridge;

impl AgentBridge {
    // ========================================================================
    // Database / persistence
    // ========================================================================

    /// Set the database tracking backend and readiness gate.
    ///
    /// This replaces the old `set_db_pool`. The caller provides:
    /// - A `DbTrackingBackend` (abstracts all recording + memory SQL)
    /// - A `DbReadinessGate` (waits for PG to be ready)
    /// - A `SubAgentChainPersistence` (for sub-agent chain persistence)
    pub fn set_db_backend(
        &mut self,
        backend: Arc<dyn crate::db_traits::DbTrackingBackend>,
        ready_gate: impl crate::db_traits::DbReadinessGate + Clone + 'static,
        chain_persistence: Arc<dyn golish_sub_agents::SubAgentChainPersistence>,
    ) {
        let session_uuid = uuid::Uuid::new_v4();
        let ws = self.workspace.try_read().ok();
        let project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.services.db_tracker = Some(
            crate::db_tracking::DbTracker::new(backend.clone(), session_uuid, ready_gate.clone())
                .with_project_path(project_path),
        );
        self.services.chain_persistence = Some(chain_persistence.clone());

        // Load prompt template overrides from DB (non-blocking)
        let prompt_reg = self.prompt_registry.clone();
        let sub_reg = self.sub_agent_registry.clone();
        let cp = chain_persistence.clone();
        tokio::spawn(async move {
            let rows = cp.load_prompt_template_overrides().await;
            if let Err(e) = prompt_reg.load_overrides(rows).await {
                tracing::warn!("[prompt-registry] Failed to load DB overrides: {e}");
            } else {
                let new_agents =
                    golish_sub_agents::defaults::create_default_sub_agents_from_registry(
                        &prompt_reg,
                    )
                    .await;
                // Preserve per-agent model/LLM-param overrides: this reload only
                // refreshes prompt text from DB templates, and a plain
                // `register_multiple` would replace the definitions wholesale —
                // wiping the `model_override` (and temperature/max_tokens/top_p)
                // that `apply_sub_agent_model_settings` / `set_sub_agent_model`
                // set. That race made stage-run sub-agents silently fall back to
                // the main model instead of their configured (e.g. xiaomi) one.
                let mut reg = sub_reg.write().await;
                reg.register_preserving_overrides(new_agents);
                tracing::info!("[prompt-registry] Reloaded sub-agents with DB template overrides");
            }
        });

        // Wire up PlanManager with DB persistence
        let ws = self.workspace.try_read().ok();
        let plan_project_path = ws
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|s| s != ".");
        self.plan_manager =
            Arc::new(PlanManager::new().with_db_repo(Some(session_uuid), plan_project_path));

        let plan_manager = self.plan_manager.clone();
        let be = backend.clone();
        let mut gate = ready_gate;
        tokio::spawn(async move {
            if !gate.is_ready()
                && tokio::time::timeout(std::time::Duration::from_secs(60), gate.wait())
                    .await
                    .is_err()
            {
                return;
            }
            be.ensure_session(session_uuid).await;

            plan_manager.load_from_db().await;
        });
    }

    /// Override the DB tracker's session UUID after [`Self::set_db_backend`].
    ///
    /// `set_db_backend` builds the tracker with a fresh random session UUID. The
    /// headless `--stage-run` path calls this with the orchestrator's session id
    /// (resolved from the chat-session key) so `tool_calls` are recorded under the
    /// SAME session the harness gate queries — otherwise the red_team scoping
    /// cross-check finds no tool calls and fail-opens. No-op if no tracker is set.
    pub fn set_tracker_session_uuid(&mut self, session_uuid: uuid::Uuid) {
        if let Some(tracker) = self.services.db_tracker.as_mut() {
            tracker.set_session_uuid(session_uuid);
        }
    }

    // ========================================================================
    // Optional service wiring
    // ========================================================================

    /// Set the IndexerState for code analysis tools
    pub fn set_indexer_state(&mut self, indexer_state: Arc<IndexerState>) {
        self.services.indexer_state = Some(indexer_state);
    }

    /// Set the session capture backend (decoupled from golish-sidecar).
    pub fn set_sidecar_state(&mut self, sidecar_state: Arc<dyn SessionCaptureBackend>) {
        self.services.sidecar_state = Some(sidecar_state);
    }

    /// Set the graph knowledge base backend (decoupled from golish-graphiti).
    pub fn set_graph_backend(
        &mut self,
        backend: Arc<dyn crate::tool_executors::graph_trait::GraphKnowledgeBase>,
    ) {
        self.services.graph_backend = Some(backend);
    }

    /// Set the TranscriptWriter for persisting AI events to JSONL.
    pub fn set_transcript_writer(&mut self, writer: TranscriptWriter, base_dir: PathBuf) {
        let writer = Arc::new(writer);
        if let Some(ref coordinator) = self.events.coordinator {
            coordinator.set_transcript_writer(Arc::clone(&writer));
        }
        self.events.transcript_writer = Some(writer);
        self.events.transcript_base_dir = Some(base_dir);
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
        self.services.settings_manager = Some(settings_manager);
    }

    /// Set the database repository provider for trait-based DB access.
    pub fn set_db_repo(&mut self, repo: Arc<dyn crate::db_traits::DbRepoProvider>) {
        if let Some(ref mut tracker) = self.services.db_tracker {
            tracker.set_repo(repo);
        }
    }

    /// Attach an embedder to the DB tracker for semantic memory operations.
    pub fn set_embedder(&mut self, embedder: Arc<dyn crate::db_traits::TextEmbedder>) {
        if let Some(ref mut tracker) = self.services.db_tracker {
            tracker.set_embedder(embedder);
        }
    }

    /// Get the memory file path dynamically from current settings.
    /// This ensures we always use the latest settings, even if they changed
    /// after the AI session was initialized.
    /// Falls back to cached value if `settings_manager` is not available.
    pub(super) async fn get_memory_file_path_dynamic(&self) -> Option<PathBuf> {
        if let Some(ref settings_manager) = self.services.settings_manager {
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
        let Some(ref provider) = self.skill_provider else {
            tracing::debug!("[refresh_skills] No skill provider configured, skipping");
            return;
        };

        let workspace = self.workspace.read().await;
        let workspace_str = workspace.to_string_lossy().to_string();
        drop(workspace);

        let provider = Arc::clone(provider);
        let metadata = match tokio::task::spawn_blocking(move || {
            provider.discover_skills(Some(&workspace_str))
        })
        .await
        {
            Ok(skills) => skills,
            Err(e) => {
                tracing::warn!("[refresh_skills] Failed to discover skills: {}", e);
                return;
            }
        };

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

        let Some(ref provider) = self.skill_provider else {
            return (available_skills, Vec::new());
        };

        let matches = provider.match_skills(prompt, &skill_cache);

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
            match provider.load_skill_body(&meta.path) {
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
        let mut current = self.access.agent_mode.write().await;
        tracing::debug!("Agent mode changed: {} -> {}", *current, mode);
        *current = mode;
    }

    /// Get the current agent mode.
    pub async fn get_agent_mode(&self) -> AgentMode {
        *self.access.agent_mode.read().await
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

    /// Set the selected harness operation profile id for this session
    /// (`None` clears it, reverting Task runs to the env default profile).
    pub async fn set_harness_profile(&self, profile: Option<String>) {
        *self.harness_profile.write().await = profile;
    }

    /// Get the selected harness operation profile id for this session.
    pub async fn get_harness_profile(&self) -> Option<String> {
        self.harness_profile.read().await.clone()
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
        let agent_mode = *self.access.agent_mode.read().await;
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

    /// C2c · Side-channel handle: the active harness stage (read side). Lets the
    /// `submit_stage_deliverable` tool validate the submitted `stage_id` matches.
    pub fn harness_active_stage_handle(
        &self,
    ) -> Arc<RwLock<Option<golish_agent_kit::harness::StageKind>>> {
        self.harness_active_stage.clone()
    }

    /// Side-channel handle: the active engagement org id (read side). Injected
    /// into `manage_targets` so its `list` action confines to the bound
    /// engagement's org subtree (设计 2026-06-15-engagement-org-isolation),
    /// mirroring `in_scope_targets_impl` — a sibling engagement's assets left in
    /// the same workspace never leak into this run's working set.
    pub fn harness_active_org_id_handle(&self) -> Arc<RwLock<Option<uuid::Uuid>>> {
        self.harness_active_org_id.clone()
    }

    /// C2c · Side-channel handle: the captured-StageDeliverable sink (write side).
    /// The `submit_stage_deliverable` tool writes here; the Task-mode executor
    /// reads it at stage close and feeds it to the deterministic gate.
    pub fn harness_last_deliverable_handle(&self) -> Arc<RwLock<Option<String>>> {
        self.harness_last_deliverable.clone()
    }

    /// Lead-agent decision side-channel handle. The `start_operation` tool writes
    /// the requested operation (JSON `{objective, analysis}`) here; the Task-mode
    /// router reads it after the lead turn to decide whether to run the planner.
    pub fn pending_plan_request_handle(&self) -> Arc<RwLock<Option<String>>> {
        self.pending_plan_request.clone()
    }

    /// Get the prompt template registry.
    pub fn prompt_registry(&self) -> &golish_sub_agents::PromptRegistry {
        &self.prompt_registry
    }

    /// Get the provider name.
    pub fn provider_name(&self) -> &str {
        &self.llm.provider_name
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.llm.model_name
    }

    /// Get the plan manager.
    pub fn plan_manager(&self) -> &Arc<PlanManager> {
        &self.plan_manager
    }

    /// Get the LLM client.
    pub fn client(&self) -> &Arc<RwLock<LlmClient>> {
        &self.llm.client
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
        self.services.indexer_state.as_ref()
    }

    /// Get the model factory (for sub-agent model overrides).
    pub fn model_factory(&self) -> Option<&Arc<LlmClientFactory>> {
        self.llm.model_factory.as_ref()
    }

    /// Set the model factory for sub-agent model overrides.
    pub fn set_model_factory(&mut self, factory: Arc<LlmClientFactory>) {
        self.llm.model_factory = Some(factory);
    }

    /// Override the tool configuration (e.g. to disable all tools for title-gen sessions).
    pub fn set_tool_config(&mut self, config: crate::tool_definitions::ToolSelectionConfig) {
        self.tool_config = config;
    }

    pub fn event_session_id(&self) -> Option<&str> {
        self.events.event_session_id.as_deref()
    }

    pub fn transcript_base_dir(&self) -> Option<&std::path::Path> {
        self.events.transcript_base_dir.as_deref()
    }

    pub fn api_request_stats(&self) -> &Arc<ApiRequestStats> {
        &self.api_request_stats
    }

    /// Get the current MCP tool definitions.
    /// Returns a clone of the tool definitions for external inspection.
    pub async fn mcp_tool_definitions(&self) -> Vec<rig::completion::ToolDefinition> {
        self.mcp_tool_definitions.read().await.clone()
    }

    pub async fn set_mcp_executor(&self, executor: Arc<dyn crate::agentic_loop::McpToolExecutor>) {
        *self.mcp_tool_executor.write().await = Some(executor);
    }

    // ========================================================================
    // Domain hooks
    // ========================================================================

    /// Set the post-shell-command hook for structured output detection/storage.
    pub fn set_post_shell_hook(&mut self, hook: crate::agentic_loop::PostShellHook) {
        self.post_shell_hook = Some(hook);
    }

    /// Set the output classifier that detects structured storage for shell output.
    pub fn set_output_classifier(&mut self, classifier: crate::agentic_loop::OutputClassifier) {
        self.output_classifier = Some(classifier);
    }
}
