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
use std::future::Future;

use golish_core::events::AiEvent;
use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use crate::agentic_loop::{run_agentic_loop, run_agentic_loop_generic};

use super::failover::{failover_decision, fallback_model};
use super::terminal_error::{extract_terminal_error_state, should_emit_execution_error_event};
use super::AgentBridge;

impl AgentBridge {
    // ========================================================================
    // Public entry points
    // ========================================================================

    /// Execute a text prompt with the default sub-agent context.
    ///
    /// Production top-level callers must hold a [`super::TopLevelRequestLease`]
    /// acquired from `begin_top_level_request`. This raw seam deliberately does
    /// not reset cancellation; nested fallback execution shares its owner.
    pub async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_with_context(prompt, SubAgentContext::default())
            .await
    }

    /// Execute a text prompt while appending hidden, single-turn instructions to
    /// the system prompt. UI events, sidecar capture, and conversation history
    /// still record the original user prompt.
    pub async fn execute_with_turn_instructions(
        &self,
        prompt: &str,
        turn_instructions: &str,
    ) -> Result<String> {
        self.execute_with_context_inner(prompt, SubAgentContext::default(), Some(turn_instructions))
            .await
    }

    /// Execute a prompt in an isolated conversation context.
    ///
    /// Saves the current conversation history, runs the prompt with a fresh
    /// (empty) history, then restores the original history afterward. This
    /// prevents context leakage between Task-mode subtasks.
    pub async fn execute_isolated(&self, prompt: &str) -> Result<String> {
        self.execute_isolated_with_context(prompt, SubAgentContext::default())
            .await
    }

    /// Execute a Task-mode prompt with fresh history while preserving explicit
    /// top-level request context for loop-dispatched specialist workers.
    ///
    /// `BridgeAgentExecutor` uses this to carry `ExecutionContext.task_input`
    /// into `SubAgentContext.original_request`; `stage_run` may quote a bounded,
    /// lower-priority excerpt into its per-org worker objective. Isolation and
    /// history restoration remain identical to [`Self::execute_isolated`].
    pub async fn execute_isolated_with_context(
        &self,
        prompt: &str,
        context: SubAgentContext,
    ) -> Result<String> {
        // Use depth=0 so Task-mode primary executes with the same restricted
        // orchestration-only tool set as PentAGI's primary agent. Callers must
        // keep the supplied context at depth=0; a non-zero depth would select a
        // different tool surface and is therefore rejected rather than silently
        // changing Task-mode policy.
        if context.depth != 0 {
            return Err(anyhow::anyhow!(
                "isolated Task-mode execution requires depth=0 context"
            ));
        }

        self.run_with_isolated_history(self.execute_with_context_inner(prompt, context, None))
            .await
    }

    /// Restore a backup left by an aborted/panicked isolated execution.
    ///
    /// Acquire the async history lock *before* taking the synchronous backup.
    /// If this future is itself cancelled while waiting, the backup remains in
    /// the recovery slot for the next owner.
    pub(super) async fn restore_isolated_history_recovery(&self) -> bool {
        let mut history = self.session.conversation_history.write().await;
        let mut recovery = self
            .isolated_history_recovery
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(saved_history) = recovery.take() else {
            return false;
        };
        *history = saved_history;
        true
    }

    /// Run one future with a fresh Task history while preserving durable chat
    /// history across success, ordinary error/Stop, future abort, and panic.
    ///
    /// The backup is synchronously published immediately after `mem::take`, with
    /// no await between those operations. Normal completion restores it here;
    /// abort/panic leaves it for `begin_top_level_request` to recover before the
    /// next execution.
    async fn run_with_isolated_history<F, T>(&self, future: F) -> T
    where
        F: Future<Output = T>,
    {
        // A panic caught by an outer caller may re-enter under the same owner.
        // Recover that prior scope before starting another isolated scope.
        self.restore_isolated_history_recovery().await;

        let saved_history = {
            let mut history = self.session.conversation_history.write().await;
            std::mem::take(&mut *history)
        };
        {
            let mut recovery = self
                .isolated_history_recovery
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            debug_assert!(recovery.is_none());
            *recovery = Some(saved_history);
        }

        let result = future.await;
        self.restore_isolated_history_recovery().await;
        result
    }

    /// Execute with rich content (text + images).
    ///
    /// Production top-level callers must already hold the bridge's universal
    /// request lease. Cancellation is reset only by successful acquisition.
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

        if !client.supports_thinking() {
            tracing::warn!(
                "execute_with_content called on non-Vertex provider; images may not work correctly"
            );
        }

        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        golish_llm_providers::dispatch_llm_client_split!(&*client,
            vertex_anthropic(va) => {
                let va = va.clone();
                drop(client);
                let (accumulated_response, reasoning, final_history, token_usage) =
                    golish_core::with_agent_session(
                        self.event_session_id().map(str::to_string),
                        run_agentic_loop(&va, &system_prompt, initial_history, context, &loop_ctx),
                    )
                    .await?;
                Ok(self.finalize_execution(accumulated_response, reasoning, final_history, token_usage, start_time).await)
            },
            generic(m) => {
                let m = m.clone();
                drop(client);
                let (accumulated_response, reasoning, final_history, token_usage) =
                    golish_core::with_agent_session(
                        self.event_session_id().map(str::to_string),
                        run_agentic_loop_generic(&m, &system_prompt, initial_history, context, &loop_ctx),
                    )
                    .await?;
                Ok(self.finalize_execution(accumulated_response, reasoning, final_history, token_usage, start_time).await)
            },
            mock => {
                drop(client);
                Err(anyhow::anyhow!("Mock client cannot execute - use for testing infrastructure only"))
            },
        )
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
        self.execute_with_context_inner(prompt, context, None).await
    }

    async fn execute_with_context_inner(
        &self,
        prompt: &str,
        context: SubAgentContext,
        turn_instructions: Option<&str>,
    ) -> Result<String> {
        if context.depth >= MAX_AGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "Maximum agent recursion depth ({}) exceeded",
                MAX_AGENT_DEPTH
            ));
        }

        let turn_id = uuid::Uuid::new_v4().to_string();
        self.emit_event(AiEvent::Started {
            turn_id: turn_id.clone(),
        });
        self.emit_event(AiEvent::UserMessage {
            content: prompt.to_string(),
        });

        let start_time = std::time::Instant::now();
        // E3 · keep a copy of the context for a possible fallback-model retry
        // (the primary dispatch moves `context` into the chosen variant arm).
        let failover_context = context.clone();
        let client = self.llm.client.read().await;

        let result = golish_llm_providers::dispatch_llm_client_split!(&*client,
            vertex_anthropic(va) => {
                let va = va.clone();
                drop(client);
                self.run_anthropic_thinking_turn(&va, prompt, start_time, context, turn_instructions).await
            },
            generic(m) => {
                let m = m.clone();
                drop(client);
                self.run_generic_turn(&m, prompt, start_time, context, turn_instructions).await
            },
            mock => {
                drop(client);
                Err(anyhow::anyhow!(
                    "Mock client cannot execute - use for testing infrastructure only"
                ))
            },
        );

        // E3 · provider failover: if the primary model run failed with a
        // recoverable error and a distinct fallback model is configured, rebuild
        // a client for the fallback model and retry the turn once. Default OFF
        // (GOLISH_LLM_FALLBACK_MODEL unset → this is a no-op, primary `result`
        // passes through unchanged).
        // Do not poll the provider-dispatch-heavy failover future after a
        // successful primary turn. In debug builds that future has a large
        // generated poll frame; nesting it under the equally large primary
        // dispatch frame exhausted even the GUI runtime's 32 MiB worker stack.
        let result = match result {
            Ok(response) => Ok(response),
            Err(primary_error) => {
                self.maybe_failover_to_fallback_model(
                    primary_error,
                    prompt,
                    start_time,
                    failover_context,
                    turn_instructions,
                )
                .await
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
        turn_instructions: Option<&str>,
    ) -> Result<String>
    where
        M: rig::completion::CompletionModel + Sync,
    {
        let (system_prompt, initial_history, loop_event_tx) = self
            .prepare_execution_context(initial_prompt, turn_instructions)
            .await;
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        let (accumulated_response, reasoning, final_history, token_usage) =
            golish_core::with_agent_session(
                self.event_session_id().map(str::to_string),
                run_agentic_loop_generic(
                    model,
                    &system_prompt,
                    initial_history,
                    context,
                    &loop_ctx,
                ),
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
    }

    /// E3 · provider failover.
    ///
    /// If `result` is a recoverable failure and a distinct fallback model is
    /// configured (`GOLISH_LLM_FALLBACK_MODEL`) with a client factory available,
    /// rebuild a client for the fallback model and run the turn once more.
    /// Otherwise the input `result` passes through untouched (default OFF).
    ///
    /// The primary failure did NOT finalize the conversation history (the loop
    /// returns before `finalize_execution`), so the retry starts from the same
    /// state. `Started` / `UserMessage` events were already emitted by the
    /// caller and are not re-emitted here.
    async fn maybe_failover_to_fallback_model(
        &self,
        primary_error: anyhow::Error,
        prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
        turn_instructions: Option<&str>,
    ) -> Result<String> {
        let e = primary_error;

        let Some(fallback) = failover_decision(
            &e.to_string(),
            fallback_model().as_deref(),
            &self.llm.model_name,
            self.llm.model_factory.is_some(),
        ) else {
            return Err(e);
        };

        let Some(factory) = self.llm.model_factory.as_ref() else {
            return Err(e);
        };

        let fallback_client = match factory
            .get_or_create(&self.llm.provider_name, &fallback)
            .await
        {
            Ok(client) => client,
            Err(build_err) => {
                tracing::warn!(
                    error = %build_err,
                    fallback = %fallback,
                    "[resilience] failed to build fallback client; surfacing primary error"
                );
                return Err(e);
            }
        };

        tracing::warn!(
            primary = %self.llm.model_name,
            fallback = %fallback,
            error = %e,
            "[resilience] primary model failed with a recoverable error; failing over to fallback model"
        );
        self.emit_event(AiEvent::Warning {
            message: format!(
                "Primary model '{}' failed; retrying with fallback model '{}'.",
                self.llm.model_name, fallback
            ),
        });

        golish_llm_providers::dispatch_llm_client_split!(&*fallback_client,
            vertex_anthropic(va) => {
                let va = va.clone();
                self.run_anthropic_thinking_turn(&va, prompt, start_time, context, turn_instructions).await
            },
            generic(m) => {
                let m = m.clone();
                self.run_generic_turn(&m, prompt, start_time, context, turn_instructions).await
            },
            mock => Err(anyhow::anyhow!(
                "Fallback model resolved to a mock client - check GOLISH_LLM_FALLBACK_MODEL"
            )),
        )
    }

    /// Run one agentic turn against the Anthropic-specific path that supports
    /// extended thinking (Vertex Anthropic).
    async fn run_anthropic_thinking_turn(
        &self,
        model: &rig_anthropic_vertex::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
        turn_instructions: Option<&str>,
    ) -> Result<String> {
        let (system_prompt, initial_history, loop_event_tx) = self
            .prepare_execution_context(initial_prompt, turn_instructions)
            .await;
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        // run_agentic_loop is the Anthropic-specific entry point with
        // extended-thinking support; it preserves reasoning blocks in the
        // history (required by the Anthropic API when thinking is enabled).
        // The sidecar session is intentionally NOT ended here — it persists
        // across prompts. See `finalize_execution` and the `Drop` impl for
        // session lifecycle.
        let (accumulated_response, reasoning, final_history, token_usage) =
            golish_core::with_agent_session(
                self.event_session_id().map(str::to_string),
                run_agentic_loop(model, &system_prompt, initial_history, context, &loop_ctx),
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

#[cfg(test)]
mod isolated_history_tests {
    use std::any::Any;
    use std::sync::Arc;

    use async_trait::async_trait;
    use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};

    use super::AgentBridge;

    #[derive(Debug)]
    struct MockRuntime;

    #[async_trait]
    impl GolishRuntime for MockRuntime {
        fn emit(&self, _event: RuntimeEvent) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn request_approval(
            &self,
            _request_id: String,
            _tool_name: String,
            _args: serde_json::Value,
            _risk_level: String,
        ) -> Result<ApprovalResult, RuntimeError> {
            Err(RuntimeError::ApprovalTimeout(0))
        }

        fn is_interactive(&self) -> bool {
            false
        }

        fn auto_approve(&self) -> bool {
            false
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    async fn real_bridge() -> (tempfile::TempDir, Arc<AgentBridge>) {
        let workspace = tempfile::tempdir().expect("temp workspace");
        let bridge = AgentBridge::new_openrouter_with_runtime(
            workspace.path().to_path_buf(),
            "test-model",
            "test-key",
            None,
            Arc::new(MockRuntime),
        )
        .await
        .expect("test bridge");
        (workspace, Arc::new(bridge))
    }

    async fn seed_history(bridge: &AgentBridge) -> String {
        bridge
            .restore_conversation_history(vec![
                ("user".to_string(), "durable user".to_string()),
                ("assistant".to_string(), "durable assistant".to_string()),
            ])
            .await;
        format!("{:?}", *bridge.session.conversation_history.read().await)
    }

    async fn overwrite_with_temporary_history(bridge: &AgentBridge) {
        bridge
            .restore_conversation_history(vec![(
                "user".to_string(),
                "temporary isolated content".to_string(),
            )])
            .await;
    }

    #[tokio::test]
    async fn isolated_history_restores_on_success_error_and_stop() {
        let (_workspace, bridge) = real_bridge().await;
        let baseline = seed_history(&bridge).await;
        let request = bridge.begin_top_level_request().await.unwrap();

        let inner = bridge.clone();
        let success = bridge
            .run_with_isolated_history(async move {
                overwrite_with_temporary_history(&inner).await;
                Ok::<_, &'static str>("ok")
            })
            .await;
        assert_eq!(success, Ok("ok"));
        assert_eq!(
            format!("{:?}", *bridge.session.conversation_history.read().await),
            baseline
        );

        let inner = bridge.clone();
        let failed = bridge
            .run_with_isolated_history(async move {
                overwrite_with_temporary_history(&inner).await;
                Err::<(), _>("ordinary error")
            })
            .await;
        assert_eq!(failed, Err("ordinary error"));
        assert_eq!(
            format!("{:?}", *bridge.session.conversation_history.read().await),
            baseline
        );

        let inner = bridge.clone();
        let stopped = bridge
            .run_with_isolated_history(async move {
                overwrite_with_temporary_history(&inner).await;
                inner.cancel();
                Err::<(), _>("stopped")
            })
            .await;
        assert_eq!(stopped, Err("stopped"));
        assert!(bridge.is_cancelled());
        assert_eq!(
            format!("{:?}", *bridge.session.conversation_history.read().await),
            baseline
        );

        bridge
            .clear_top_level_request_state(&request)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn next_owner_recovers_history_after_real_future_abort() {
        let (_workspace, bridge) = real_bridge().await;
        let baseline = seed_history(&bridge).await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task_bridge = bridge.clone();
        let task = tokio::spawn(async move {
            let _request = task_bridge.begin_top_level_request().await.unwrap();
            task_bridge
                .run_with_isolated_history(async move {
                    let _ = started_tx.send(());
                    std::future::pending::<()>().await;
                })
                .await;
        });

        started_rx.await.unwrap();
        assert_eq!(bridge.conversation_history_len().await, 0);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        let next = bridge.begin_top_level_request().await.unwrap();
        assert_eq!(
            format!("{:?}", *bridge.session.conversation_history.read().await),
            baseline
        );
        bridge.clear_top_level_request_state(&next).await.unwrap();
    }

    #[tokio::test]
    async fn next_owner_recovers_history_after_async_panic() {
        let (_workspace, bridge) = real_bridge().await;
        let baseline = seed_history(&bridge).await;
        let task_bridge = bridge.clone();
        let task = tokio::spawn(async move {
            let _request = task_bridge.begin_top_level_request().await.unwrap();
            task_bridge
                .run_with_isolated_history(async move {
                    panic!("simulated isolated execution panic");
                })
                .await;
        });

        assert!(task.await.unwrap_err().is_panic());
        let next = bridge.begin_top_level_request().await.unwrap();
        assert_eq!(
            format!("{:?}", *bridge.session.conversation_history.read().await),
            baseline
        );
        bridge.clear_top_level_request_state(&next).await.unwrap();
    }
}
