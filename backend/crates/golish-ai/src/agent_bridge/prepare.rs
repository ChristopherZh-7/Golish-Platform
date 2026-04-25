//! Per-turn execution context preparation and finalization for [`AgentBridge`].
//!
//! These helpers wrap the boilerplate that surrounds every `execute_with_*_model`
//! call: building the system prompt, starting/refreshing sessions, seeding the
//! conversation history with the user message, constructing the
//! [`AgenticLoopContext`], and (after the loop returns) persisting the final
//! response and emitting the completion event.

use rig::completion::Message;
use rig::message::{AssistantContent, Text, UserContent};
use rig::one_or_many::OneOrMany;
use tokio::sync::mpsc;

use golish_context::token_budget::TokenUsage;
use golish_core::events::AiEvent;
use golish_core::PromptContext;

use super::super::agentic_loop::AgenticLoopContext;
use super::super::contributors::create_default_contributors;
use super::super::prompt_registry::PromptContributorRegistry;
use super::super::system_prompt::build_system_prompt_with_contributions;

use super::terminal_error::TerminalErrorState;
use super::AgentBridge;

impl AgentBridge {
    /// Prepare the execution context for an agent turn.
    ///
    /// This extracts common setup code that was duplicated across all
    /// `execute_with_*_model` methods:
    /// 1. Build system prompt with agent mode and memory file
    /// 2. Inject session context
    /// 3. Start session for persistence
    /// 4. Record user message
    /// 5. Handle sidecar capture (start session, capture prompt)
    /// 6. Prepare initial history with user message
    /// 7. Get or create event channel
    ///
    /// Returns the system prompt, initial history, and event channel sender.
    pub(super) async fn prepare_execution_context(
        &self,
        initial_prompt: &str,
    ) -> (String, Vec<Message>, mpsc::UnboundedSender<AiEvent>) {
        let workspace_path = self.workspace.read().await;
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        let contributors = create_default_contributors(self.sub_agent_registry.clone());
        let mut registry = PromptContributorRegistry::new();
        for contributor in contributors {
            registry.register(contributor);
        }

        let has_web_search = self
            .tool_registry
            .read()
            .await
            .available_tools()
            .iter()
            .any(|t| t.starts_with("web_"));
        let has_sub_agents = *self.use_agents.read().await;

        let (available_skills, matched_skills) = self.match_and_load_skills(initial_prompt).await;

        let prompt_context = PromptContext::new(&self.provider_name, &self.model_name)
            .with_web_search(has_web_search)
            .with_sub_agents(has_sub_agents)
            .with_workspace(workspace_path.display().to_string())
            .with_user_prompt(initial_prompt.to_string())
            .with_available_skills(available_skills)
            .with_matched_skills(matched_skills);

        let mut system_prompt = build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            Some(&registry),
            Some(&prompt_context),
        );
        drop(workspace_path);

        // Inject Layer 1 session context if available
        if let Some(session_context) = self.get_session_context().await {
            if !session_context.is_empty() {
                tracing::debug!(
                    "[agent] Injecting Layer 1 session context ({} chars)",
                    session_context.len()
                );
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&session_context);
            }
        }

        // Inject active execution plan status if one exists
        if let Some(plan_status) = self.plan_manager.format_for_prompt().await {
            tracing::info!(
                "[agent] Injecting active execution plan ({} chars)",
                plan_status.len()
            );
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&plan_status);
        }

        self.start_session().await;
        self.record_user_message(initial_prompt).await;

        // Sidecar capture: only start a new session if one doesn't already exist
        // (sessions span conversations).
        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            let session_id = if let Some(existing_id) = sidecar.current_session_id() {
                tracing::debug!("Reusing existing sidecar session: {}", existing_id);
                Some(existing_id)
            } else {
                match sidecar.start_session(initial_prompt) {
                    Ok(new_id) => {
                        tracing::info!("Started new sidecar session: {}", new_id);
                        Some(new_id)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start sidecar session: {}", e);
                        None
                    }
                }
            };

            if let Some(ref sid) = session_id {
                let prompt_event = SessionEvent::user_prompt(sid.clone(), initial_prompt);
                sidecar.capture(prompt_event);

                self.with_session_manager(|m| {
                    m.set_sidecar_session_id(sid.clone());
                })
                .await;
            }
        }

        let mut history_guard = self.conversation_history.write().await;
        history_guard.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: initial_prompt.to_string(),
            })),
        });
        let initial_history = history_guard.clone();
        drop(history_guard);

        let loop_event_tx = self.get_or_create_event_tx();

        (system_prompt, initial_history, loop_event_tx)
    }

    /// Prepare execution context with rich content (text + images).
    ///
    /// Similar to [`Self::prepare_execution_context`] but accepts
    /// `Vec<UserContent>` instead of a plain string, enabling multi-modal prompts.
    pub(super) async fn prepare_execution_context_with_content(
        &self,
        content: Vec<UserContent>,
        text_for_logging: &str,
    ) -> (String, Vec<Message>, mpsc::UnboundedSender<AiEvent>) {
        tracing::debug!("[prepare_context] Starting context preparation");

        tracing::debug!("[prepare_context] Acquiring workspace read lock");
        let workspace_path = self.workspace.read().await;
        tracing::debug!("[prepare_context] Acquiring agent_mode read lock");
        let agent_mode = *self.agent_mode.read().await;
        let memory_file_path = self.get_memory_file_path_dynamic().await;

        let contributors = create_default_contributors(self.sub_agent_registry.clone());
        let mut registry = PromptContributorRegistry::new();
        for contributor in contributors {
            registry.register(contributor);
        }

        let has_web_search = self
            .tool_registry
            .read()
            .await
            .available_tools()
            .iter()
            .any(|t| t.starts_with("web_"));
        let has_sub_agents = *self.use_agents.read().await;

        let (available_skills, matched_skills) = self.match_and_load_skills(text_for_logging).await;

        let prompt_context = PromptContext::new(&self.provider_name, &self.model_name)
            .with_web_search(has_web_search)
            .with_sub_agents(has_sub_agents)
            .with_workspace(workspace_path.display().to_string())
            .with_user_prompt(text_for_logging.to_string())
            .with_available_skills(available_skills)
            .with_matched_skills(matched_skills);

        let mut system_prompt = build_system_prompt_with_contributions(
            &workspace_path,
            agent_mode,
            memory_file_path.as_deref(),
            Some(&registry),
            Some(&prompt_context),
        );
        drop(workspace_path);

        if let Some(session_context) = self.get_session_context().await {
            if !session_context.is_empty() {
                tracing::debug!(
                    "[agent] Injecting Layer 1 session context ({} chars)",
                    session_context.len()
                );
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&session_context);
            }
        }

        if let Some(plan_status) = self.plan_manager.format_for_prompt().await {
            tracing::info!(
                "[agent] Injecting active execution plan ({} chars)",
                plan_status.len()
            );
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&plan_status);
        }

        self.start_session().await;
        self.record_user_message(text_for_logging).await;

        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            let session_id = if let Some(existing_id) = sidecar.current_session_id() {
                tracing::debug!("Reusing existing sidecar session: {}", existing_id);
                Some(existing_id)
            } else {
                match sidecar.start_session(text_for_logging) {
                    Ok(new_id) => {
                        tracing::info!("Started new sidecar session: {}", new_id);
                        Some(new_id)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to start sidecar session: {}", e);
                        None
                    }
                }
            };

            if let Some(ref sid) = session_id {
                let prompt_event = SessionEvent::user_prompt(sid.clone(), text_for_logging);
                sidecar.capture(prompt_event);

                self.with_session_manager(|m| {
                    m.set_sidecar_session_id(sid.clone());
                })
                .await;
            }
        }

        let mut history_guard = self.conversation_history.write().await;

        let incoming_text_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Text(_)))
            .count();
        let incoming_image_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Image(_)))
            .count();
        tracing::debug!(
            "prepare_context: {} text part(s), {} image(s)",
            incoming_text_count,
            incoming_image_count
        );

        let user_content = match OneOrMany::many(content) {
            Ok(many) => {
                tracing::debug!(
                    "prepare_execution_context_with_content: Created OneOrMany with {} items",
                    many.len()
                );
                many
            }
            Err(_) => {
                tracing::warn!(
                    "prepare_execution_context_with_content: Empty content, using placeholder"
                );
                OneOrMany::one(UserContent::Text(Text {
                    text: "".to_string(),
                }))
            }
        };
        let user_message = Message::User {
            content: user_content,
        };

        history_guard.push(user_message);
        let initial_history = history_guard.clone();
        drop(history_guard);

        let loop_event_tx = self.get_or_create_event_tx();

        (system_prompt, initial_history, loop_event_tx)
    }

    /// Build the AgenticLoopContext with references to all required components.
    pub(super) async fn build_loop_context<'a>(
        &'a self,
        loop_event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    ) -> AgenticLoopContext<'a> {
        AgenticLoopContext {
            event_tx: loop_event_tx,
            tool_registry: &self.tool_registry,
            sub_agent_registry: &self.sub_agent_registry,
            indexer_state: self.indexer_state.as_ref(),
            workspace: &self.workspace,
            client: &self.client,
            approval_recorder: &self.approval_recorder,
            pending_approvals: &self.pending_approvals,
            tool_policy_manager: &self.tool_policy_manager,
            context_manager: &self.context_manager,
            compaction_state: &self.compaction_state,
            loop_detector: &self.loop_detector,
            tool_config: &self.tool_config,
            sidecar_state: self.sidecar_state.as_ref(),
            runtime: self.runtime.as_ref(),
            agent_mode: &self.agent_mode,
            plan_manager: &self.plan_manager,
            api_request_stats: &self.api_request_stats,
            provider_name: &self.provider_name,
            model_name: &self.model_name,
            openai_web_search_config: self.openai_web_search_config.as_ref(),
            openai_reasoning_effort: self.openai_reasoning_effort.as_deref(),
            openrouter_provider_preferences: self.openrouter_provider_preferences.as_ref(),
            model_factory: self.model_factory.as_ref(),
            session_id: self.event_session_id.as_deref(),
            transcript_writer: self.transcript_writer.as_ref(),
            transcript_base_dir: self.transcript_base_dir.as_deref(),
            // MCP tool definitions and executor (otherwise empty in the main app -
            // additional_tool_definitions is also used by the eval framework).
            additional_tool_definitions: {
                let mcp_tools = self.mcp_tool_definitions.read().await;
                mcp_tools.clone()
            },
            custom_tool_executor: self.mcp_tool_executor.read().await.clone(),
            coordinator: self.coordinator.as_ref(),
            db_tracker: self.db_tracker.as_ref(),
            cancelled: Some(&self.cancelled),
            execution_monitor: None,
            execution_mode: *self.execution_mode.read().await,
        }
    }

    /// Set MCP tool definitions.
    /// Called by `configure_bridge` after the MCP manager is initialized.
    pub async fn set_mcp_tools(&self, definitions: Vec<rig::completion::ToolDefinition>) {
        *self.mcp_tool_definitions.write().await = definitions;
    }

    /// Finalize execution after the agentic loop completes.
    ///
    /// Persists the assistant response into history, saves the session,
    /// captures the response in the sidecar, and emits the `Completed` event.
    pub(super) async fn finalize_execution(
        &self,
        accumulated_response: String,
        reasoning: Option<String>,
        final_history: Vec<Message>,
        token_usage: Option<TokenUsage>,
        start_time: std::time::Instant,
    ) -> String {
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Replace conversation history with the full history from the agentic loop.
        // Critical for the OpenAI Responses API where reasoning IDs must be preserved
        // in history for function calls to work correctly across turns.
        {
            let mut history_guard = self.conversation_history.write().await;
            *history_guard = final_history;
        }

        if !accumulated_response.is_empty() {
            self.record_assistant_message(&accumulated_response).await;
            self.save_session().await;
        }

        if let Some(ref sidecar) = self.sidecar_state {
            use golish_sidecar::events::SessionEvent;

            if let Some(session_id) = sidecar.current_session_id() {
                if !accumulated_response.is_empty() {
                    let response_event =
                        SessionEvent::ai_response(session_id, &accumulated_response);
                    sidecar.capture(response_event);
                    tracing::debug!(
                        "[agent] Captured AI response in sidecar ({} chars)",
                        accumulated_response.len()
                    );
                }
            }
        }

        self.emit_event(AiEvent::Completed {
            response: accumulated_response.clone(),
            reasoning,
            input_tokens: token_usage.as_ref().map(|u| u.input_tokens as u32),
            output_tokens: token_usage.as_ref().map(|u| u.output_tokens as u32),
            duration_ms: Some(duration_ms),
        });

        accumulated_response
    }

    /// Restore conversation history from a list of simple messages.
    /// Used when reopening an existing conversation to give the AI context.
    pub async fn restore_conversation_history(&self, messages: Vec<(String, String)>) {
        let mut history = Vec::new();
        for (role, content) in messages {
            match role.as_str() {
                "user" => {
                    history.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text { text: content })),
                    });
                }
                "assistant" => {
                    history.push(Message::Assistant {
                        id: None,
                        content: OneOrMany::one(AssistantContent::Text(Text { text: content })),
                    });
                }
                _ => {
                    tracing::warn!("[restore] Unknown message role: {}", role);
                }
            }
        }
        let count = history.len();
        let mut guard = self.conversation_history.write().await;
        *guard = history;
        tracing::info!("[restore] Restored {} messages to conversation history", count);
    }

    pub(super) async fn persist_terminal_error_state(
        &self,
        terminal_state: &TerminalErrorState,
    ) {
        if let Some(final_history) = terminal_state.final_history.clone() {
            let mut history_guard = self.conversation_history.write().await;
            *history_guard = final_history;
        }

        if let Some(partial_response) = terminal_state
            .partial_response
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            self.record_assistant_message(partial_response).await;

            if let Some(ref sidecar) = self.sidecar_state {
                use golish_sidecar::events::SessionEvent;

                if let Some(session_id) = sidecar.current_session_id() {
                    sidecar.capture(SessionEvent::ai_response(session_id, partial_response));
                }
            }
        }

        self.save_session().await;
    }
}
