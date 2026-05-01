//! Eval execution with custom tool definitions.

use std::path::Path;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;

use golish_ai::eval_support::EvalConfig as AiEvalConfig;

use crate::config::{EvalConfig, EvalProvider};
use crate::runner::{AgentOutput, ToolCall as EvalToolCall, VerboseConfig};

use super::super::build_production_system_prompt;

/// Execute a prompt with custom tools for specialized benchmarks.
#[allow(clippy::too_many_arguments)]
pub async fn execute_eval_prompt_with_tools(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    provider: EvalProvider,
    model_override: Option<&str>,
    additional_tools: Vec<rig::completion::ToolDefinition>,
    custom_executor: Option<golish_ai::eval_support::CustomToolExecutor>,
) -> Result<AgentOutput> {
    let config = EvalConfig::load_for_provider(provider)
        .await?
        .with_model(model_override.map(|s| s.to_string()));

    match provider {
        EvalProvider::VertexClaude => {
            execute_with_vertex_claude_and_tools(
                workspace, prompt, system_prompt, verbose_config, &config,
                additional_tools, custom_executor,
            )
            .await
        }
        EvalProvider::Zai => {
            execute_with_zai_and_tools(
                workspace, prompt, system_prompt, verbose_config, &config,
                additional_tools, custom_executor,
            )
            .await
        }
        EvalProvider::OpenAi => {
            execute_with_openai_and_tools(
                workspace, prompt, system_prompt, verbose_config, &config,
                additional_tools, custom_executor,
            )
            .await
        }
    }
}

async fn execute_with_vertex_claude_and_tools(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
    additional_tools: Vec<rig::completion::ToolDefinition>,
    custom_executor: Option<golish_ai::eval_support::CustomToolExecutor>,
) -> Result<AgentOutput> {
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

    let model_id = config
        .model_override
        .as_deref()
        .unwrap_or(models::CLAUDE_SONNET_4_5);
    let model_name = config
        .model_override
        .as_deref()
        .unwrap_or("Claude Sonnet 4.5");

    let model = client.completion_model(model_id).with_web_search();

    execute_with_model_and_tools(
        workspace, prompt, system_prompt, verbose_config, model,
        model_name, EvalProvider::VertexClaude, additional_tools, custom_executor,
    )
    .await
}

async fn execute_with_zai_and_tools(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
    additional_tools: Vec<rig::completion::ToolDefinition>,
    custom_executor: Option<golish_ai::eval_support::CustomToolExecutor>,
) -> Result<AgentOutput> {
    let zai_config = config
        .zai
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Z.AI configuration not available"))?;

    let model_id = config
        .model_override
        .as_deref()
        .unwrap_or(rig_zai_sdk::models::GLM_4);
    let model_name = config.model_override.as_deref().unwrap_or("GLM-4");

    let client = rig_zai_sdk::Client::new(&zai_config.api_key);
    let model = client.completion_model(model_id);

    execute_with_model_and_tools(
        workspace, prompt, system_prompt, verbose_config, model,
        model_name, EvalProvider::Zai, additional_tools, custom_executor,
    )
    .await
}

async fn execute_with_openai_and_tools(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
    additional_tools: Vec<rig::completion::ToolDefinition>,
    custom_executor: Option<golish_ai::eval_support::CustomToolExecutor>,
) -> Result<AgentOutput> {
    use rig::client::CompletionClient;
    use rig::providers::openai as rig_openai;

    let openai_config = config
        .openai
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("OpenAI configuration not available"))?;

    let model_id = config.model_override.as_deref().unwrap_or("gpt-5.1");
    let model_name = config.model_override.as_deref().unwrap_or("GPT-5.1");

    let client: rig_openai::Client = rig_openai::Client::new(&openai_config.api_key)
        .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;
    let model = client.completion_model(model_id);

    execute_with_model_and_tools(
        workspace, prompt, system_prompt, verbose_config, model,
        model_name, EvalProvider::OpenAi, additional_tools, custom_executor,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_with_model_and_tools<M>(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    model: M,
    model_name: &str,
    provider: EvalProvider,
    additional_tools: Vec<rig::completion::ToolDefinition>,
    custom_executor: Option<golish_ai::eval_support::CustomToolExecutor>,
) -> Result<AgentOutput>
where
    M: RigCompletionModel + Sync,
{
    let provider_name = match provider {
        EvalProvider::VertexClaude => "anthropic",
        EvalProvider::Zai => "zai",
        EvalProvider::OpenAi => "openai_responses",
    };

    let ai_config = AiEvalConfig {
        provider_name: provider_name.to_string(),
        model_name: model_name.to_string(),
        require_hitl: false,
        workspace: workspace.to_path_buf(),
        verbose: verbose_config.enabled,
    };

    let effective_system_prompt = match system_prompt {
        Some(custom) => custom.to_string(),
        None => build_production_system_prompt(workspace, provider),
    };

    let eval_output = golish_ai::eval_support::run_eval_agentic_loop_with_tools(
        &model,
        &effective_system_prompt,
        prompt,
        ai_config,
        additional_tools,
        custom_executor,
    )
    .await?;

    tracing::info!(
        "Eval with custom tools completed with {} tool calls, {} files modified",
        eval_output.tool_calls.len(),
        eval_output.files_modified.len()
    );

    let tool_calls = eval_output
        .tool_calls
        .into_iter()
        .map(|tc| EvalToolCall {
            name: tc.name,
            input: tc.input,
            output: tc.output,
            success: tc.success,
        })
        .collect();

    Ok(AgentOutput {
        response: eval_output.response,
        tool_calls,
        files_modified: eval_output.files_modified,
        duration_ms: eval_output.duration_ms,
        tokens_used: eval_output.tokens_used,
    })
}
