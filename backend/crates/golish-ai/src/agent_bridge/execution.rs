//! Execution methods for [`AgentBridge`].
//!
//! Public entry points:
//! - [`AgentBridge::execute`] — text prompt, default sub-agent context.
//! - [`AgentBridge::execute_isolated`] — text prompt with a fresh history,
//!   restoring the original history on return (used for Task-mode subtasks).
//! - [`AgentBridge::execute_with_content`] — multi-modal prompts (text +
//!   images), routes to the Vertex Anthropic vision path when available.
//! - [`AgentBridge::execute_with_context`] — text prompt + explicit
//!   `SubAgentContext`. Dispatches to the right LLM client variant.
//! - [`AgentBridge::execute_tool`] / [`AgentBridge::available_tools`] —
//!   direct tool invocation helpers (used by the eval framework / Tauri).
//!
//! All variant-specific execution funnels through two private helpers:
//! - [`AgentBridge::run_generic_turn`] — the standard agentic loop for any
//!   `rig::completion::CompletionModel`.
//! - [`AgentBridge::run_anthropic_thinking_turn`] — the Anthropic-specific
//!   loop with extended thinking support (Vertex Anthropic).

use anyhow::Result;
use rig::message::UserContent;

use golish_core::events::AiEvent;
use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use crate::agentic_loop::{run_agentic_loop, run_agentic_loop_generic};
use crate::llm_client::LlmClient;

use super::terminal_error::{extract_terminal_error_state, should_emit_execution_error_event};
use super::AgentBridge;

impl AgentBridge {
    // ========================================================================
    // Public entry points
    // ========================================================================

    /// Execute a text prompt with the default sub-agent context.
    pub async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_with_context(prompt, SubAgentContext::default())
            .await
    }

    /// Execute a prompt in an isolated conversation context.
    ///
    /// Saves the current conversation history, runs the prompt with a fresh
    /// (empty) history, then restores the original history afterward. This
    /// prevents context leakage between Task-mode subtasks.
    pub async fn execute_isolated(&self, prompt: &str) -> Result<String> {
        let saved_history = {
            let mut guard = self.session.conversation_history.write().await;
            std::mem::take(&mut *guard)
        };

        // depth=1 so the Task-mode tool isolation (which only restricts
        // depth==0) does NOT apply to subtask execution. Subtasks need full
        // tool access.
        let mut subtask_ctx = SubAgentContext::default();
        subtask_ctx.depth = 1;

        let result = self.execute_with_context(prompt, subtask_ctx).await;

        {
            let mut guard = self.session.conversation_history.write().await;
            *guard = saved_history;
        }

        result
    }

    /// Execute with rich content (text + images).
    ///
    /// Multi-modal prompts route through this entry point for vision-capable
    /// models. See [`AgentBridge::execute_with_content_and_context`] for the
    /// version that accepts an explicit sub-agent context.
    pub async fn execute_with_content(&self, content: Vec<UserContent>) -> Result<String> {
        let image_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Image(_)))
            .count();
        let text_count = content
            .iter()
            .filter(|c| matches!(c, UserContent::Text(_)))
            .count();
        tracing::debug!(
            "execute_with_content: {} text part(s), {} image(s)",
            text_count,
            image_count
        );

        self.execute_with_content_and_context(content, SubAgentContext::default())
            .await
    }

    /// Execute with rich content + sub-agent context.
    ///
    /// Routes to the Vertex Anthropic agentic loop when the active client is
    /// `LlmClient::VertexAnthropic` (which fully supports inline images via
    /// the Anthropic vision API). All other providers fall back to text-only
    /// execution and a `tracing::warn!` is emitted — the images may be
    /// dropped depending on the provider's tolerance for unknown content
    /// parts.
    pub async fn execute_with_content_and_context(
        &self,
        content: Vec<UserContent>,
        context: SubAgentContext,
    ) -> Result<String> {
        tracing::info!(
            message = "[execute_with_content_and_context] Starting execution",
            content_parts = content.len(),
            depth = context.depth,
            event_session_id = ?self.events.event_session_id,
        );

        if context.depth >= MAX_AGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "Maximum agent recursion depth ({}) exceeded",
                MAX_AGENT_DEPTH
            ));
        }

        if context.depth == 0 {
            self.reset_cancelled();
        }

        let turn_id = uuid::Uuid::new_v4().to_string();
        tracing::debug!(
            message = "[execute_with_content_and_context] Emitting Started event",
            turn_id = %turn_id,
        );
        self.emit_event(AiEvent::Started {
            turn_id: turn_id.clone(),
        });

        let start_time = std::time::Instant::now();

        let text_for_logging = content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        self.emit_event(AiEvent::UserMessage {
            content: text_for_logging.clone(),
        });

        let (system_prompt, initial_history, loop_event_tx) = self
            .prepare_execution_context_with_content(content, &text_for_logging)
            .await;

        let client = self.llm.client.read().await;

        // Vertex Anthropic supports inline images natively via the Anthropic
        // Messages API; route there first.
        if let LlmClient::VertexAnthropic(vertex_model) = &*client {
            let vertex_model = vertex_model.clone();
            drop(client);

            let loop_ctx = self.build_loop_context(&loop_event_tx).await;
            let (accumulated_response, reasoning, final_history, token_usage) = run_agentic_loop(
                &vertex_model,
                &system_prompt,
                initial_history,
                context,
                &loop_ctx,
            )
            .await?;

            return Ok(self
                .finalize_execution(
                    accumulated_response,
                    reasoning,
                    final_history,
                    token_usage,
                    start_time,
                )
                .await);
        }

        tracing::warn!(
            "execute_with_content called on non-Vertex provider; images may not work correctly"
        );

        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        // Macro-like dispatch over text-capable providers using the generic loop.
        // Each arm clones the model, drops the client lock, and forwards to the
        // generic agentic loop. The match itself stays cheap because cloning a
        // rig CompletionModel is just bumping reference counts internally.
        macro_rules! run_with {
            ($model:expr) => {{
                let model = $model.clone();
                drop(client);
                let (accumulated_response, reasoning, final_history, token_usage) =
                    run_agentic_loop_generic(
                        &model,
                        &system_prompt,
                        initial_history,
                        context,
                        &loop_ctx,
                    )
                    .await?;
                Ok(self
                    .finalize_execution(
                        accumulated_response,
                        reasoning,
                        final_history,
                        token_usage,
                        start_time,
                    )
                    .await)
            }};
        }

        match &*client {
            LlmClient::RigAnthropic(model) => run_with!(model),
            LlmClient::RigGemini(model) => run_with!(model),
            LlmClient::RigOpenAi(model) => run_with!(model),
            LlmClient::RigOpenAiResponses(model) => run_with!(model),
            LlmClient::OpenAiReasoning(model) => run_with!(model),
            _ => {
                drop(client);
                Err(anyhow::anyhow!(
                    "execute_with_content not fully supported for this provider"
                ))
            }
        }
    }

    /// Execute a text prompt with an explicit sub-agent context.
    ///
    /// Top-level dispatch over [`LlmClient`] variants. Each variant clones
    /// the model out of the read-locked `client` (so the rest of the code
    /// can drop the lock), then forwards to one of the two private helpers:
    /// - [`Self::run_anthropic_thinking_turn`] for `VertexAnthropic`
    ///   (extended thinking supported).
    /// - [`Self::run_generic_turn`] for everything else.
    ///
    /// On error, the bridge persists any partial state from the loop and
    /// emits an `Error` event — unless the loop already emitted a terminal
    /// error (signaled by `TerminalErrorEmitted`), in which case the
    /// emission is skipped to avoid duplicates.
    pub async fn execute_with_context(
        &self,
        prompt: &str,
        context: SubAgentContext,
    ) -> Result<String> {
        if context.depth >= MAX_AGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "Maximum agent recursion depth ({}) exceeded",
                MAX_AGENT_DEPTH
            ));
        }

        // Only reset at the top level; sub-agents share the same `cancelled`
        // flag and must not clear a cancellation the user triggered
        // mid-execution.
        if context.depth == 0 {
            self.reset_cancelled();
        }

        let turn_id = uuid::Uuid::new_v4().to_string();
        self.emit_event(AiEvent::Started {
            turn_id: turn_id.clone(),
        });
        self.emit_event(AiEvent::UserMessage {
            content: prompt.to_string(),
        });

        let start_time = std::time::Instant::now();
        let client = self.llm.client.read().await;

        let result = match &*client {
            LlmClient::VertexAnthropic(vertex_model) => {
                let vertex_model = vertex_model.clone();
                drop(client);
                self.run_anthropic_thinking_turn(&vertex_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigOpenRouter(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigOpenAi(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigOpenAiResponses(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::OpenAiReasoning(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigAnthropic(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigOllama(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigGemini(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigGroq(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigXai(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigZaiSdk(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::RigNvidia(model) => {
                let model = model.clone();
                drop(client);
                // NVIDIA uses the OpenAI-compatible API path.
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::VertexGemini(model) => {
                let model = model.clone();
                drop(client);
                self.run_generic_turn(&model, prompt, start_time, context).await
            }
            LlmClient::Mock => {
                drop(client);
                Err(anyhow::anyhow!(
                    "Mock client cannot execute - use for testing infrastructure only"
                ))
            }
        };

        // Emit error event on failure so every Started has a matching terminal
        // event (Completed or Error), unless the loop already emitted a
        // terminal error (TerminalErrorEmitted marker).
        if let Err(ref e) = result {
            tracing::error!(
                message = "[execute_with_context] Execution failed after Started event",
                error = %e,
            );

            if let Some(terminal_state) = extract_terminal_error_state(e) {
                self.persist_terminal_error_state(&terminal_state).await;
            }

            if should_emit_execution_error_event(e) {
                self.emit_event(AiEvent::Error {
                    message: e.to_string(),
                    error_type: "execution_error".to_string(),
                });
            } else {
                tracing::debug!(
                    "[execute_with_context] Skipping duplicate Error emission (already emitted in loop)"
                );
            }
        }

        result
    }

    // ========================================================================
    // Private execution helpers (DRY shared body for all model variants)
    // ========================================================================

    /// Run one agentic turn against a generic [`rig::completion::CompletionModel`].
    ///
    /// All providers except Vertex Anthropic flow through this method.
    async fn run_generic_turn<M>(
        &self,
        model: &M,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String>
    where
        M: rig::completion::CompletionModel + Sync,
    {
        let (system_prompt, initial_history, loop_event_tx) =
            self.prepare_execution_context(initial_prompt).await;
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        let (accumulated_response, reasoning, final_history, token_usage) =
            run_agentic_loop_generic(model, &system_prompt, initial_history, context, &loop_ctx)
                .await?;

        Ok(self
            .finalize_execution(
                accumulated_response,
                reasoning,
                final_history,
                token_usage,
                start_time,
            )
            .await)
    }

    /// Run one agentic turn against the Anthropic-specific path that supports
    /// extended thinking (Vertex Anthropic).
    async fn run_anthropic_thinking_turn(
        &self,
        model: &rig_anthropic_vertex::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
        let (system_prompt, initial_history, loop_event_tx) =
            self.prepare_execution_context(initial_prompt).await;
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        // run_agentic_loop is the Anthropic-specific entry point with
        // extended-thinking support; it preserves reasoning blocks in the
        // history (required by the Anthropic API when thinking is enabled).
        // The sidecar session is intentionally NOT ended here — it persists
        // across prompts. See `finalize_execution` and the `Drop` impl for
        // session lifecycle.
        let (accumulated_response, reasoning, final_history, token_usage) =
            run_agentic_loop(model, &system_prompt, initial_history, context, &loop_ctx).await?;

        Ok(self
            .finalize_execution(
                accumulated_response,
                reasoning,
                final_history,
                token_usage,
                start_time,
            )
            .await)
    }

    // ========================================================================
    // Direct tool helpers (eval framework / Tauri)
    // ========================================================================

    /// Execute a tool directly by name, bypassing the agentic loop.
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let registry = self.tool_registry.read().await;
        registry
            .execute_tool(tool_name, args)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    /// List the names of available tools.
    pub async fn available_tools(&self) -> Vec<serde_json::Value> {
        let registry = self.tool_registry.read().await;
        registry
            .available_tools()
            .into_iter()
            .map(|name| serde_json::json!({ "name": name }))
            .collect()
    }

    /// Get the sidecar session context (state.md content) for prompt injection.
    pub async fn get_session_context(&self) -> Option<String> {
        let sidecar = self.services.sidecar_state.as_ref()?;

        match sidecar.get_injectable_context().await {
            Ok(context) => context,
            Err(e) => {
                tracing::warn!("Failed to get session context: {}", e);
                None
            }
        }
    }
}
