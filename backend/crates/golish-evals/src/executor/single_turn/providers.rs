//! Per-provider eval execution implementations.

use std::path::Path;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;

use golish_agent_runtime::eval_support::EvalConfig as AiEvalConfig;

use crate::config::{EvalConfig, EvalProvider};
use crate::runner::{AgentOutput, ToolCall as EvalToolCall, VerboseConfig};

use super::super::build_production_system_prompt;

/// Execute with Vertex AI Claude.
pub(super) async fn execute_with_vertex_claude(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
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

    execute_with_model(
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::VertexClaude,
    )
    .await
}

/// Execute with Z.AI GLM.
pub(super) async fn execute_with_zai(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
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

    execute_with_model(
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::Zai,
    )
    .await
}

/// Execute with OpenAI.
pub(super) async fn execute_with_openai(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    config: &EvalConfig,
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

    execute_with_model(
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::OpenAi,
    )
    .await
}

/// Generic execution with any model implementing CompletionModel.
pub(super) async fn execute_with_model<M>(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    model: M,
    model_name: &str,
    provider: EvalProvider,
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

    let eval_output = golish_agent_runtime::eval_support::run_eval_agentic_loop(
        &model,
        &effective_system_prompt,
        prompt,
        ai_config,
    )
    .await?;

    tracing::info!(
        "Eval completed with {} tool calls, {} files modified",
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
