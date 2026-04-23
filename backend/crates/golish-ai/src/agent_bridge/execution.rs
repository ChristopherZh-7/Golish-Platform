//! Execution methods for AgentBridge.
//!
//! Contains all execute, execute_with_*, and model-dispatch methods.

use anyhow::Result;
use rig::message::UserContent;
use rig::providers::anthropic as rig_anthropic;
use rig::providers::gemini as rig_gemini;
use rig::providers::groq as rig_groq;
use rig::providers::ollama as rig_ollama;
use rig::providers::openai as rig_openai;
use rig::providers::openrouter as rig_openrouter;
use rig::providers::xai as rig_xai;

use golish_core::events::AiEvent;
use golish_sub_agents::{SubAgentContext, MAX_AGENT_DEPTH};

use crate::llm_client::{rig_gemini_vertex, rig_zai_sdk, LlmClient};
use super::{
    extract_terminal_error_state, should_emit_execution_error_event, AgentBridge,
};
use crate::agentic_loop::{run_agentic_loop, run_agentic_loop_generic};

impl AgentBridge {
    // ========================================================================
    // Main Execution Methods
    // ========================================================================

    /// Execute a prompt with agentic tool loop.
    pub async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_with_context(prompt, SubAgentContext::default())
            .await
    }

    /// Execute a prompt in an isolated conversation context.
    ///
    /// Saves the current conversation history, runs the prompt with a
    /// fresh (empty) history, then restores the original history afterward.
    /// This prevents context leakage between Task-mode subtasks.
    pub async fn execute_isolated(&self, prompt: &str) -> Result<String> {
        let saved_history = {
            let mut guard = self.conversation_history.write().await;
            std::mem::take(&mut *guard)
        };

        // Depth=1 so that Task-mode tool isolation (which only restricts depth==0)
        // does NOT apply to subtask execution. Subtasks need full tools to do work.
        let mut subtask_ctx = SubAgentContext::default();
        subtask_ctx.depth = 1;

        let result = self
            .execute_with_context(prompt, subtask_ctx)
            .await;

        {
            let mut guard = self.conversation_history.write().await;
            *guard = saved_history;
        }

        result
    }

    /// Execute with rich content (text + images).
    ///
    /// This method accepts multiple content parts, enabling multi-modal prompts
    /// with images for vision-capable models.
    ///
    /// # Arguments
    ///
    /// * `content` - Vector of UserContent (text, images, etc.)
    ///
    /// # Example
    ///
    /// ```ignore
    /// use rig::message::UserContent;
    ///
    /// let content = vec![
    ///     UserContent::text("What's in this image?"),
    ///     UserContent::image_base64("...", Some(ImageMediaType::PNG), None),
    /// ];
    /// let response = bridge.execute_with_content(content).await?;
    /// ```
    pub async fn execute_with_content(&self, content: Vec<UserContent>) -> Result<String> {
        // Log content types for debugging
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

    /// Execute with rich content and sub-agent context.
    pub async fn execute_with_content_and_context(
        &self,
        content: Vec<UserContent>,
        context: SubAgentContext,
    ) -> Result<String> {
        tracing::info!(
            message = "[execute_with_content_and_context] Starting execution",
            content_parts = content.len(),
            depth = context.depth,
            event_session_id = ?self.event_session_id,
        );

        // Check recursion depth
        if context.depth >= MAX_AGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "Maximum agent recursion depth ({}) exceeded",
                MAX_AGENT_DEPTH
            ));
        }

        // Generate a unique turn ID
        let turn_id = uuid::Uuid::new_v4().to_string();
        tracing::debug!(
            message = "[execute_with_content_and_context] Emitting Started event",
            turn_id = %turn_id,
        );

        // Emit turn started event
        self.emit_event(AiEvent::Started {
            turn_id: turn_id.clone(),
        });

        let start_time = std::time::Instant::now();

        // Extract text for logging/session recording
        let text_for_logging = content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Emit user message event for transcript
        self.emit_event(AiEvent::UserMessage {
            content: text_for_logging.clone(),
        });

        // Prepare execution context with rich content
        let (system_prompt, initial_history, loop_event_tx) = self
            .prepare_execution_context_with_content(content, &text_for_logging)
            .await;

        let client = self.client.read().await;

        // Currently only Vertex Anthropic supports images properly
        // Other providers would need their own image handling
        match &*client {
            LlmClient::VertexAnthropic(vertex_model) => {
                let vertex_model = vertex_model.clone();
                drop(client);

                let loop_ctx = self.build_loop_context(&loop_event_tx).await;
                let (accumulated_response, reasoning, final_history, token_usage) =
                    run_agentic_loop(
                        &vertex_model,
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
            }
            // For other providers, fall back to text-only execution
            _ => {
                drop(client);
                tracing::warn!(
                    "execute_with_content called on non-Vertex provider, images may not work correctly"
                );

                let loop_ctx = self.build_loop_context(&loop_event_tx).await;
                let client = self.client.read().await;

                // Use generic execution for other providers
                match &*client {
                    LlmClient::RigAnthropic(model) => {
                        let model = model.clone();
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
                    }
                    LlmClient::RigGemini(model) => {
                        let model = model.clone();
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
                    }
                    LlmClient::RigOpenAi(model) => {
                        let model = model.clone();
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
                    }
                    LlmClient::RigOpenAiResponses(model) => {
                        let model = model.clone();
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
                    }
                    LlmClient::OpenAiReasoning(model) => {
                        let model = model.clone();
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
                    }
                    _ => {
                        drop(client);
                        Err(anyhow::anyhow!(
                            "execute_with_content not fully supported for this provider"
                        ))
                    }
                }
            }
        }
    }

    // ========================================================================
    // Cancellation-Enabled Execution Methods (server feature only)
    // ========================================================================

    /// Execute a prompt with cancellation support.
    ///
    /// The cancellation token allows external cancellation of the execution,
    /// which is essential for HTTP server timeouts and client disconnections.
    ///
    /// # Arguments
    ///
    /// * `prompt` - The user prompt to execute
    /// * `cancel_token` - Token that can be used to cancel the execution
    ///
    /// # Returns
    ///
    /// * `Ok(String)` - The accumulated response from the agent
    /// * `Err` - If execution was cancelled or failed
    ///
    /// Execute a prompt with context (for sub-agent calls).
    pub async fn execute_with_context(
        &self,
        prompt: &str,
        context: SubAgentContext,
    ) -> Result<String> {
        // Check recursion depth
        if context.depth >= MAX_AGENT_DEPTH {
            return Err(anyhow::anyhow!(
                "Maximum agent recursion depth ({}) exceeded",
                MAX_AGENT_DEPTH
            ));
        }

        // Generate a unique turn ID
        let turn_id = uuid::Uuid::new_v4().to_string();

        // Emit turn started event
        self.emit_event(AiEvent::Started {
            turn_id: turn_id.clone(),
        });

        // Emit user message event for transcript
        self.emit_event(AiEvent::UserMessage {
            content: prompt.to_string(),
        });

        let start_time = std::time::Instant::now();
        let client = self.client.read().await;

        // Execute with the appropriate model and capture the result
        let result = match &*client {
            LlmClient::VertexAnthropic(vertex_model) => {
                let vertex_model = vertex_model.clone();
                drop(client);

                self.execute_with_vertex_model(&vertex_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigOpenRouter(openrouter_model) => {
                let openrouter_model = openrouter_model.clone();
                drop(client);

                self.execute_with_openrouter_model(&openrouter_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigOpenAi(openai_model) => {
                let openai_model = openai_model.clone();
                drop(client);

                self.execute_with_openai_model(&openai_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigOpenAiResponses(openai_model) => {
                let openai_model = openai_model.clone();
                drop(client);

                self.execute_with_openai_responses_model(&openai_model, prompt, start_time, context)
                    .await
            }
            LlmClient::OpenAiReasoning(openai_model) => {
                let openai_model = openai_model.clone();
                drop(client);

                self.execute_with_openai_reasoning_model(&openai_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigAnthropic(anthropic_model) => {
                let anthropic_model = anthropic_model.clone();
                drop(client);

                // Use the generic execution path (same as OpenRouter/OpenAI)
                self.execute_with_anthropic_model(&anthropic_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigOllama(ollama_model) => {
                let ollama_model = ollama_model.clone();
                drop(client);

                // Use the generic execution path (same as OpenRouter/OpenAI)
                self.execute_with_ollama_model(&ollama_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigGemini(gemini_model) => {
                let gemini_model = gemini_model.clone();
                drop(client);

                self.execute_with_gemini_model(&gemini_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigGroq(groq_model) => {
                let groq_model = groq_model.clone();
                drop(client);

                self.execute_with_groq_model(&groq_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigXai(xai_model) => {
                let xai_model = xai_model.clone();
                drop(client);

                self.execute_with_xai_model(&xai_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigZaiSdk(zai_sdk_model) => {
                let zai_sdk_model = zai_sdk_model.clone();
                drop(client);

                self.execute_with_zai_sdk_model(&zai_sdk_model, prompt, start_time, context)
                    .await
            }
            LlmClient::RigNvidia(nvidia_model) => {
                let nvidia_model = nvidia_model.clone();
                drop(client);

                self.execute_with_openai_model(
                    &nvidia_model,
                    prompt,
                    start_time,
                    context,
                )
                .await
            }
            LlmClient::VertexGemini(vertex_gemini_model) => {
                let vertex_gemini_model = vertex_gemini_model.clone();
                drop(client);

                self.execute_with_vertex_gemini_model(
                    &vertex_gemini_model,
                    prompt,
                    start_time,
                    context,
                )
                .await
            }
            LlmClient::Mock => {
                drop(client);
                Err(anyhow::anyhow!(
                    "Mock client cannot execute - use for testing infrastructure only"
                ))
            }
        };

        // Emit error event if execution failed.
        // This ensures every Started event has a matching terminal event (Completed or Error)
        // unless the loop already emitted a terminal error event.
        // Note: Completed is emitted in finalize_execution() on the success path.
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

    /// Execute with Vertex AI model using the agentic loop.
    ///
    /// Uses `run_agentic_loop` which is Anthropic-specific (supports extended thinking).
    async fn execute_with_vertex_model(
        &self,
        model: &rig_anthropic_vertex::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
        // Prepare common execution context (system prompt, history, event channel)
        let (system_prompt, initial_history, loop_event_tx) =
            self.prepare_execution_context(initial_prompt).await;

        // Build agentic loop context
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        // Run the Anthropic-specific agentic loop (supports extended thinking)
        let (accumulated_response, reasoning, final_history, token_usage) =
            run_agentic_loop(model, &system_prompt, initial_history, context, &loop_ctx).await?;

        // Finalize execution (persist response and full history, emit events)
        // Note: Sidecar session is NOT ended here - it persists across prompts.
        // See finalize_execution and Drop impl for session lifecycle.
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

    /// Execute with OpenRouter model using the generic agentic loop.
    async fn execute_with_openrouter_model(
        &self,
        model: &rig_openrouter::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
        // Prepare common execution context (system prompt, history, event channel)
        let (system_prompt, initial_history, loop_event_tx) =
            self.prepare_execution_context(initial_prompt).await;

        // Build agentic loop context
        let loop_ctx = self.build_loop_context(&loop_event_tx).await;

        // Run the generic agentic loop (works with any rig CompletionModel)
        let (accumulated_response, reasoning, final_history, token_usage) =
            run_agentic_loop_generic(model, &system_prompt, initial_history, context, &loop_ctx)
                .await?;

        // Finalize execution (persist response and full history, emit events)
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

    /// Execute with OpenAI model using the generic agentic loop.
    async fn execute_with_openai_model(
        &self,
        model: &rig_openai::completion::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with OpenAI Responses API model using the generic agentic loop.
    /// This uses the Responses API which has better tool support than the Chat Completions API.
    async fn execute_with_openai_responses_model(
        &self,
        model: &rig_openai::responses_api::ResponsesCompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with OpenAI reasoning model using the generic agentic loop.
    ///
    /// Uses our custom rig-openai-responses provider which properly separates
    /// reasoning deltas from text deltas in the streaming response.
    async fn execute_with_openai_reasoning_model(
        &self,
        model: &rig_openai_responses::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with Anthropic model using the generic agentic loop.
    ///
    /// This method is generic over the HTTP client type H, allowing it to work
    /// with both standard reqwest::Client and custom logging clients.
    async fn execute_with_anthropic_model<H>(
        &self,
        model: &rig_anthropic::completion::CompletionModel<H>,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String>
    where
        H: rig::http_client::HttpClientExt + Clone + Send + Sync + Default + 'static,
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

    /// Execute with Ollama model using the generic agentic loop.
    async fn execute_with_ollama_model(
        &self,
        model: &rig_ollama::CompletionModel<reqwest::Client>,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with Gemini model using the generic agentic loop.
    async fn execute_with_gemini_model(
        &self,
        model: &rig_gemini::completion::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with Groq model using the generic agentic loop.
    async fn execute_with_groq_model(
        &self,
        model: &rig_groq::CompletionModel<reqwest::Client>,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with xAI (Grok) model using the generic agentic loop.
    async fn execute_with_xai_model(
        &self,
        model: &rig_xai::completion::CompletionModel<reqwest::Client>,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with Z.AI SDK model using the generic agentic loop.
    async fn execute_with_zai_sdk_model(
        &self,
        model: &rig_zai_sdk::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute with Vertex Gemini model using the generic agentic loop.
    async fn execute_with_vertex_gemini_model(
        &self,
        model: &rig_gemini_vertex::CompletionModel,
        initial_prompt: &str,
        start_time: std::time::Instant,
        context: SubAgentContext,
    ) -> Result<String> {
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

    /// Execute a tool by name (public API).
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let registry = self.tool_registry.read().await;
        let result = registry.execute_tool(tool_name, args).await;

        result.map_err(|e| anyhow::anyhow!(e))
    }

    /// Get available tools for the LLM.
    pub async fn available_tools(&self) -> Vec<serde_json::Value> {
        let registry = self.tool_registry.read().await;
        let tool_names = registry.available_tools();

        tool_names
            .into_iter()
            .map(|name| serde_json::json!({ "name": name }))
            .collect()
    }

    /// Get session context for injection into agent prompt
    pub async fn get_session_context(&self) -> Option<String> {
        let sidecar = self.sidecar_state.as_ref()?;

        // Use the simplified sidecar API to get injectable context (state.md content)
        match sidecar.get_injectable_context().await {
            Ok(context) => context,
            Err(e) => {
                tracing::warn!("Failed to get session context: {}", e);
                None
            }
        }
    }
}

