//! Multi-turn eval execution: drive the agentic loop across a list of user
//! prompts, threading conversation history between turns.

use std::path::Path;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;

use golish_ai::eval_support::EvalConfig as AiEvalConfig;

use crate::config::{EvalConfig, EvalProvider};
use crate::runner::{AgentOutput, ToolCall as EvalToolCall, VerboseConfig};

use super::build_production_system_prompt;


/// Output from a multi-turn evaluation.
#[derive(Debug)]
pub struct MultiTurnAgentOutput {
    /// Outputs from each turn in order.
    pub turns: Vec<AgentOutput>,
    /// Total duration of all turns in milliseconds.
    pub total_duration_ms: u64,
}

/// Execute a multi-turn conversation to test reasoning ID preservation.
///
/// This is critical for testing OpenAI Responses API compatibility,
/// as reasoning item errors only manifest across multiple turns.
pub async fn execute_multi_turn_eval(
    workspace: &Path,
    prompts: &[&str],
    verbose_config: &VerboseConfig,
    provider: EvalProvider,
) -> Result<MultiTurnAgentOutput> {
    let config = EvalConfig::load_for_provider(provider).await?;

    match provider {
        EvalProvider::VertexClaude => {
            execute_multi_turn_with_vertex_claude(workspace, prompts, verbose_config, &config).await
        }
        EvalProvider::Zai => {
            execute_multi_turn_with_zai(workspace, prompts, verbose_config, &config).await
        }
        EvalProvider::OpenAi => {
            execute_multi_turn_with_openai(workspace, prompts, verbose_config, &config).await
        }
    }
}

/// Execute multi-turn with Vertex AI Claude.
async fn execute_multi_turn_with_vertex_claude(
    workspace: &Path,
    prompts: &[&str],
    _verbose_config: &VerboseConfig,
    config: &EvalConfig,
) -> Result<MultiTurnAgentOutput> {
    use rig_anthropic_vertex::{models, Client};

    let vertex_config = config
        .vertex
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Vertex AI configuration not available"))?;

    let client = if let Some(ref creds_path) = vertex_config.credentials_path {
        Client::from_service_account(
            creds_path,
            &vertex_config.project_id,
            &vertex_config.location,
        )
        .await?
    } else {
        Client::from_env(&vertex_config.project_id, &vertex_config.location).await?
    };
    let model = client
        .completion_model(models::CLAUDE_SONNET_4_5)
        .with_web_search();

    execute_multi_turn_with_model(
        workspace,
        prompts,
        model,
        "Claude Sonnet 4.5",
        EvalProvider::VertexClaude,
    )
    .await
}

/// Execute multi-turn with Z.AI GLM.
async fn execute_multi_turn_with_zai(
    workspace: &Path,
    prompts: &[&str],
    _verbose_config: &VerboseConfig,
    config: &EvalConfig,
) -> Result<MultiTurnAgentOutput> {
    let zai_config = config
        .zai
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Z.AI configuration not available"))?;

    let client = rig_zai_sdk::Client::new(&zai_config.api_key);
    let model = client.completion_model(rig_zai_sdk::models::GLM_4);

    execute_multi_turn_with_model(workspace, prompts, model, "GLM-4", EvalProvider::Zai).await
}

/// Execute multi-turn with OpenAI.
async fn execute_multi_turn_with_openai(
    workspace: &Path,
    prompts: &[&str],
    _verbose_config: &VerboseConfig,
    config: &EvalConfig,
) -> Result<MultiTurnAgentOutput> {
    use rig::client::CompletionClient;
    use rig::providers::openai as rig_openai;

    let openai_config = config
        .openai
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("OpenAI configuration not available"))?;

    let client: rig_openai::Client = rig_openai::Client::new(&openai_config.api_key)
        .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;
    let model = client.completion_model("gpt-5.1");

    execute_multi_turn_with_model(workspace, prompts, model, "GPT-5.1", EvalProvider::OpenAi).await
}

/// Generic multi-turn execution with any model.
async fn execute_multi_turn_with_model<M>(
    workspace: &Path,
    prompts: &[&str],
    model: M,
    model_name: &str,
    provider: EvalProvider,
) -> Result<MultiTurnAgentOutput>
where
    M: RigCompletionModel + Sync,
{
    let provider_name = match provider {
        EvalProvider::VertexClaude => "anthropic",
        EvalProvider::Zai => "zai",
        // Use openai_responses because evals use the Responses API (completion_model returns ResponsesCompletionModel)
        EvalProvider::OpenAi => "openai_responses",
    };

    let ai_config = AiEvalConfig {
        provider_name: provider_name.to_string(),
        model_name: model_name.to_string(),
        require_hitl: false,
        workspace: workspace.to_path_buf(),
        verbose: false, // Multi-turn evals don't need verbose output
    };

    // Build the production system prompt with contributions (same as main agent)
    let system_prompt = build_production_system_prompt(workspace, provider);

    // Run multi-turn evaluation
    let multi_output =
        golish_ai::eval_support::run_multi_turn_eval(&model, &system_prompt, prompts, ai_config)
            .await?;

    tracing::info!(
        "Multi-turn eval completed: {} turns in {}ms",
        multi_output.turns.len(),
        multi_output.total_duration_ms
    );

    // Convert outputs
    let turns = multi_output
        .turns
        .into_iter()
        .map(|turn| {
            let tool_calls = turn
                .tool_calls
                .into_iter()
                .map(|tc| EvalToolCall {
                    name: tc.name,
                    input: tc.input,
                    output: tc.output,
                    success: tc.success,
                })
                .collect();

            AgentOutput {
                response: turn.response,
                tool_calls,
                files_modified: turn.files_modified,
                duration_ms: turn.duration_ms,
                tokens_used: turn.tokens_used,
            }
        })
        .collect();

    Ok(MultiTurnAgentOutput {
        turns,
        total_duration_ms: multi_output.total_duration_ms,
    })
}
