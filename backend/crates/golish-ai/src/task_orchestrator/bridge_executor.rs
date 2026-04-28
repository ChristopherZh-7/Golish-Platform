//! Bridge-based implementation of `AgentExecutor`.
//!
//! Connects the `TaskOrchestrator` to the `AgentBridge`'s LLM client:
//! - Generator / Refiner / Reporter use one-shot completions (no tools, no history)
//! - Primary Agent subtask execution uses the full agentic loop (with tools & sub-agents)

use std::sync::Arc;

use anyhow::{Context, Result};
use rig::completion::{AssistantContent, CompletionModel as RigCompletionModel, CompletionRequest};
use rig::message::{Message, Text, UserContent};
use rig::one_or_many::OneOrMany;

use golish_llm_providers::LlmClient;

use super::prompts;
use super::{AgentExecutor, AgentResult, AgentTokenUsage, ExecutionContext, GeneratorOutput, PlannedSubtask, RefinerOutput};
use crate::agent_bridge::AgentBridge;

/// Truncate a string to at most `max_bytes` bytes, ensuring the cut lands on a
/// UTF-8 char boundary so we never panic on multi-byte characters.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from max_bytes until we hit a char boundary.
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `AgentExecutor` implementation backed by an `AgentBridge`.
pub struct BridgeAgentExecutor {
    pub(crate) bridge: Arc<AgentBridge>,
}

impl BridgeAgentExecutor {
    pub fn new(bridge: Arc<AgentBridge>) -> Self {
        Self { bridge }
    }

    /// Try to build a per-phase LLM client from settings `sub_agent_models`.
    /// Returns None if no override configured, meaning "use session default".
    async fn phase_client(&self, phase_key: &str) -> Option<LlmClient> {
        let settings_mgr = self.bridge.services.settings_manager.as_ref()?;
        let settings = settings_mgr.get().await;
        let config = settings.ai.sub_agent_models.get(phase_key)?;
        let provider = config.provider.as_ref()?;
        let model = config.model.as_ref()?;

        match golish_llm_providers::create_client_for_model(*provider, model, &settings).await {
            Ok(client) => {
                tracing::info!(
                    phase = phase_key,
                    provider = %provider,
                    model = %model,
                    "Using per-phase model override"
                );
                Some(client)
            }
            Err(e) => {
                tracing::warn!(
                    phase = phase_key,
                    error = %e,
                    "Failed to create per-phase client, falling back to session default"
                );
                None
            }
        }
    }

    /// Dispatch a subtask to a specific sub-agent directly.
    /// Currently unused: Primary orchestrates via sub_agent_* tools in the agentic loop.
    /// Retained for potential future use (e.g., bypass Primary for performance-critical paths).
    #[allow(dead_code)]
    async fn execute_via_sub_agent(
        &self,
        agent_def: &golish_sub_agents::SubAgentDefinition,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &super::ExecutionContext,
    ) -> Result<String> {
        use golish_sub_agents::{SubAgentContext, SubAgentExecutorContext};

        let event_tx = self.bridge.get_or_create_event_tx();
        let parent_request_id = uuid::Uuid::new_v4().to_string();

        let context = SubAgentContext {
            original_request: subtask_description.to_string(),
            conversation_summary: Some(execution_context.summary()),
            depth: 0,
            ..Default::default()
        };

        let args = serde_json::json!({
            "task": format!("{}\n\n{}", subtask_title, subtask_description),
        });

        let tool_provider =
            crate::tool_provider_impl::DefaultToolProvider::with_db_tracker(
                self.bridge.services.db_tracker.as_ref(),
            );
        let db_pool_arc = self.bridge.db_pool();

        let sub_ctx = SubAgentExecutorContext {
            event_tx: &event_tx,
            tool_registry: self.bridge.tool_registry(),
            workspace: self.bridge.workspace(),
            provider_name: self.bridge.provider_name(),
            model_name: self.bridge.model_name(),
            session_id: None,
            transcript_base_dir: None,
            api_request_stats: Some(self.bridge.api_request_stats()),
            briefing: None,
            temperature_override: agent_def.temperature,
            max_tokens_override: agent_def.max_tokens,
            top_p_override: agent_def.top_p,
            db_pool: db_pool_arc.as_ref(),
            sub_agent_registry: Some(self.bridge.sub_agent_registry()),
            post_shell_hook: self.bridge.post_shell_hook.clone(),
        };

        let client = self.bridge.client().read().await;
        let result =
            crate::agentic_loop::sub_agent_dispatch::execute_sub_agent_with_client(
                agent_def,
                &args,
                &context,
                &*client,
                sub_ctx,
                &tool_provider,
                &parent_request_id,
            )
            .await
            .context(format!(
                "Sub-agent '{}' failed for subtask '{}'",
                agent_def.id, subtask_title
            ))?;

        Ok(result.response)
    }

    /// Load LLM parameter overrides (temperature, max_tokens, top_p) for a phase.
    async fn phase_params(&self, phase_key: &str) -> LlmParamOverrides {
        let Some(settings_mgr) = self.bridge.services.settings_manager.as_ref() else {
            return LlmParamOverrides::default();
        };
        let settings = settings_mgr.get().await;
        let Some(config) = settings.ai.sub_agent_models.get(phase_key) else {
            return LlmParamOverrides::default();
        };
        LlmParamOverrides {
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            top_p: config.top_p,
        }
    }

    /// One-shot LLM completion with optional per-phase model override.
    ///
    /// Includes a 120-second timeout to prevent indefinite hangs on
    /// unresponsive providers.
    async fn simple_completion_for_phase(
        &self,
        system_prompt: &str,
        user_message: &str,
        phase_key: Option<&str>,
    ) -> Result<String> {
        let (phase_override, params) = if let Some(key) = phase_key {
            (self.phase_client(key).await, self.phase_params(key).await)
        } else {
            (None, LlmParamOverrides::default())
        };

        let request = build_one_shot_request(system_prompt, user_message, &params);

        tracing::debug!(
            phase = ?phase_key,
            user_msg_len = user_message.len(),
            "[TaskMode] Starting one-shot completion"
        );

        let result = if let Some(ref override_client) = phase_override {
            tokio::time::timeout(
                std::time::Duration::from_secs(120),
                complete_with_client(override_client, request),
            )
            .await
        } else {
            let client = { let g = self.bridge.llm.client.read().await; (*g).clone() };
            tokio::time::timeout(
                std::time::Duration::from_secs(120),
                async move { complete_with_client(&client, request).await },
            )
            .await
        };

        result
            .map_err(|_| anyhow::anyhow!(
                "LLM completion timed out (120s) for phase {:?}",
                phase_key
            ))?
    }
}

/// Overrides for LLM call parameters loaded from per-agent settings.
#[derive(Debug, Default)]
struct LlmParamOverrides {
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    top_p: Option<f32>,
}

async fn complete_with_client(client: &LlmClient, request: CompletionRequest) -> Result<String> {
    macro_rules! one_shot {
        ($model:expr) => {{
            let response = $model
                .completion(request)
                .await
                .map_err(|e| anyhow::anyhow!("LLM completion failed: {}", e))?;
            Ok(extract_text(&response.choice))
        }};
    }

    match client {
        LlmClient::VertexAnthropic(m) => one_shot!(m),
        LlmClient::VertexGemini(m) => one_shot!(m),
        LlmClient::RigOpenRouter(m) => one_shot!(m),
        LlmClient::RigOpenAi(m) => one_shot!(m),
        LlmClient::RigOpenAiResponses(m) => one_shot!(m),
        LlmClient::OpenAiReasoning(m) => one_shot!(m),
        LlmClient::RigAnthropic(m) => one_shot!(m),
        LlmClient::RigOllama(m) => one_shot!(m),
        LlmClient::RigGemini(m) => one_shot!(m),
        LlmClient::RigGroq(m) => one_shot!(m),
        LlmClient::RigXai(m) => one_shot!(m),
        LlmClient::RigZaiSdk(m) => one_shot!(m),
        LlmClient::RigNvidia(m) => one_shot!(m),
        LlmClient::Mock => Err(anyhow::anyhow!("Mock client cannot execute completions")),
    }
}

fn build_one_shot_request(
    system_prompt: &str,
    user_message: &str,
    overrides: &LlmParamOverrides,
) -> CompletionRequest {
    CompletionRequest {
        model: None,
        preamble: Some(system_prompt.to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: user_message.to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(overrides.temperature.unwrap_or(0.3) as f64),
        max_tokens: Some(overrides.max_tokens.unwrap_or(8192) as u64),
        tool_choice: None,
        additional_params: overrides.top_p.map(|tp| {
            serde_json::json!({ "top_p": tp })
        }),
        output_schema: None,
    }
}

fn extract_text(choice: &OneOrMany<AssistantContent>) -> String {
    choice
        .iter()
        .filter_map(|c| match c {
            AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Whether the user's message is an actionable task or casual conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserIntent {
    Task,
    Conversation,
}

/// Quick LLM classification: is this message an actionable task or just conversation?
///
/// Uses a minimal one-shot call with low max_tokens so it completes fast.
/// Falls back to `Task` if anything goes wrong (conservative — don't silently ignore tasks).
///
/// The prompt is truncated to 500 bytes for classification — the full content
/// is not needed to determine intent, and large prompts can cause timeouts
/// on some providers (e.g., Nvidia NIM).
pub async fn classify_user_intent(bridge: &AgentBridge, prompt: &str) -> UserIntent {
    // Only send a small prefix; intent is obvious from the first few hundred chars.
    let truncated_prompt = truncate_to_char_boundary(prompt, 500);

    tracing::info!(
        prompt_len = prompt.len(),
        truncated_len = truncated_prompt.len(),
        "[IntentClassifier] Starting classification"
    );

    let request = CompletionRequest {
        model: None,
        preamble: Some(prompts::intent_classifier_prompt().to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: truncated_prompt.to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(8),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    };

    // Clone the client out and drop the read lock before the API call to avoid
    // holding the lock during a potentially slow network request.
    let client = { let g = bridge.llm.client.read().await; (*g).clone() };

    // Wrap the API call in a timeout so we don't hang forever if the provider
    // is unresponsive.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        complete_with_client(&client, request),
    )
    .await;

    match result {
        Ok(Ok(response)) => {
            let word = response.trim().to_uppercase();
            tracing::info!(
                classification = %word,
                prompt_preview = %truncate_to_char_boundary(prompt, 80),
                "[IntentClassifier] Result"
            );
            if word.contains("CHAT") {
                UserIntent::Conversation
            } else {
                UserIntent::Task
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(
                error = %e,
                "[IntentClassifier] Classification failed, defaulting to Task"
            );
            UserIntent::Task
        }
        Err(_) => {
            tracing::warn!(
                "[IntentClassifier] Classification timed out (15s), defaulting to Task"
            );
            UserIntent::Task
        }
    }
}

/// Try to extract JSON from a response that may contain markdown fences.
fn extract_json_from_response(response: &str) -> &str {
    let trimmed = response.trim();

    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }
    trimmed
}

#[async_trait::async_trait]
impl AgentExecutor for BridgeAgentExecutor {
    async fn generate_subtasks(&self, task_input: &str) -> Result<GeneratorOutput> {
        tracing::info!("[TaskMode/Generator] Decomposing task into subtasks");
        let response = self
            .simple_completion_for_phase(prompts::generator_prompt(), task_input, Some("pipeline_generator"))
            .await
            .context("Generator LLM call failed")?;

        let json_str = extract_json_from_response(&response);
        serde_json::from_str::<GeneratorOutput>(json_str).context(format!(
            "Failed to parse generator JSON. Raw response:\n{}",
            truncate_to_char_boundary(&response, 500)
        ))
    }

    async fn execute_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: Option<&str>,
    ) -> Result<AgentResult> {
        let agent_label = agent_type.unwrap_or("primary");
        tracing::info!(
            "[TaskMode] Executing subtask: {} (suggested agent: {})",
            subtask_title,
            agent_label,
        );
        let start = std::time::Instant::now();

        // Try to use orchestrator agent's prompt from the sub-agent registry
        // (user-customizable via Settings). Falls back to hardcoded prompt
        // if orchestrator definition is not found.
        let prompt = {
            let registry = self.bridge.sub_agent_registry();
            let reg = registry.read().await;
            let orchestrator_prompt = reg
                .get("orchestrator")
                .map(|def| {
                    def.system_prompt
                        .replace("{{execution_context}}", &execution_context.render_xml())
                });
            drop(reg);

            if let Some(base_prompt) = orchestrator_prompt {
                format!(
                    "{}\n\n## CURRENT SUBTASK\n\nTitle: {}\nDescription: {}\n{}",
                    base_prompt,
                    subtask_title,
                    subtask_description,
                    agent_type
                        .map(|a| format!("Suggested specialist: {}", a))
                        .unwrap_or_default(),
                )
            } else {
                tracing::warn!(
                    "[TaskMode] Orchestrator agent not found in registry, using fallback prompt"
                );
                prompts::primary_agent_subtask_prompt_with_agent(
                    subtask_title,
                    subtask_description,
                    &execution_context.summary(),
                    agent_type,
                )
            }
        };

        let content = self.bridge.execute_isolated(&prompt).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(AgentResult::with_usage(
            content,
            AgentTokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                duration_ms,
                phase: format!("primary_subtask:{}", agent_label),
            },
        ))
    }

    async fn refine_plan(
        &self,
        execution_context: &ExecutionContext,
        remaining_subtasks: &[PlannedSubtask],
    ) -> Result<RefinerOutput> {
        tracing::info!(
            "[TaskMode/Refiner] Refining plan ({} remaining subtasks)",
            remaining_subtasks.len()
        );
        let remaining_json = serde_json::to_string_pretty(remaining_subtasks)?;
        let system = prompts::refiner_prompt(&execution_context.summary(), &remaining_json);

        let response = self
            .simple_completion_for_phase(&system, "Analyze completed work and adjust the remaining plan.", Some("pipeline_refiner"))
            .await
            .context("Refiner LLM call failed")?;

        let json_str = extract_json_from_response(&response);
        serde_json::from_str::<RefinerOutput>(json_str).context(format!(
            "Failed to parse refiner JSON. Raw response:\n{}",
            truncate_to_char_boundary(&response, 500)
        ))
    }

    async fn generate_report(&self, execution_context: &ExecutionContext) -> Result<AgentResult> {
        tracing::info!("[TaskMode/Reporter] Generating final report");
        let start = std::time::Instant::now();
        let system = prompts::reporter_prompt(&execution_context.summary());
        let content = self.simple_completion_for_phase(&system, "Generate the final task report based on all completed subtask results.", Some("pipeline_reporter"))
            .await
            .context("Reporter LLM call failed")?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(AgentResult::with_usage(
            content,
            AgentTokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                duration_ms,
                phase: "reporter".to_string(),
            },
        ))
    }

    async fn reflect(
        &self,
        subtask_title: &str,
        agent_response: &str,
    ) -> Result<String> {
        tracing::info!(
            "[TaskMode/Reflector] Agent returned text for '{}', redirecting to tool usage",
            subtask_title
        );
        let system = prompts::reflector_system_prompt();
        let user = prompts::reflector_user_prompt(subtask_title, agent_response);
        self.simple_completion_for_phase(system, &user, Some("pipeline_reflector"))
            .await
            .context("Reflector LLM call failed")
    }

    // plan_subtask and enrich removed: PentAGI-style flow delegates knowledge
    // retrieval to agents themselves via their memory/search tools.
}
