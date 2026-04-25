//! Single-turn eval execution: dispatch to the right provider and run the
//! unified agentic loop once.
//!
//! `execute_eval_prompt*` are the public entry points; the per-provider
//! `execute_with_*` and `execute_with_*_and_tools` helpers live here too,
//! along with the generic `execute_with_model_and_tools<M>` template they all
//! reduce to.

use std::path::Path;

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;

use golish_ai::eval_support::EvalConfig as AiEvalConfig;

use crate::config::{EvalConfig, EvalProvider};
use crate::runner::{AgentOutput, ToolCall as EvalToolCall, VerboseConfig};

use super::build_production_system_prompt;


/// Execute a prompt against the agent in the given workspace using the default provider.
///
/// This is a lightweight executor that:
/// - Uses the configured LLM provider (default: Vertex Claude Sonnet)
/// - Has a minimal set of tools
/// - Runs an agentic loop until completion
/// - Auto-approves all tool calls (no HITL)
///
/// If `verbose_config.enabled` is true, outputs real-time conversation.
/// If `verbose_config.log_file` is set, writes to that file instead of stdout.
pub async fn execute_eval_prompt(
    workspace: &Path,
    prompt: &str,
    verbose_config: &VerboseConfig,
) -> Result<AgentOutput> {
    execute_eval_prompt_with_options(
        workspace,
        prompt,
        None,
        verbose_config,
        EvalProvider::default(),
    )
    .await
}

/// Execute a prompt with a custom system prompt.
///
/// This variant allows testing how different system prompts affect agent behavior.
/// If `system_prompt` is `None`, uses the default eval system prompt.
pub async fn execute_eval_prompt_with_system(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
) -> Result<AgentOutput> {
    execute_eval_prompt_with_options(
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        EvalProvider::default(),
    )
    .await
}

/// Execute a prompt against the agent using a specific provider.
pub async fn execute_eval_prompt_with_provider(
    workspace: &Path,
    prompt: &str,
    verbose_config: &VerboseConfig,
    provider: EvalProvider,
) -> Result<AgentOutput> {
    execute_eval_prompt_with_options(workspace, prompt, None, verbose_config, provider).await
}

/// Execute a prompt with all options: custom system prompt and provider.
pub async fn execute_eval_prompt_with_options(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    provider: EvalProvider,
) -> Result<AgentOutput> {
    execute_eval_prompt_with_model(
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        provider,
        None,
    )
    .await
}

/// Execute a prompt with all options including model override.
pub async fn execute_eval_prompt_with_model(
    workspace: &Path,
    prompt: &str,
    system_prompt: Option<&str>,
    verbose_config: &VerboseConfig,
    provider: EvalProvider,
    model_override: Option<&str>,
) -> Result<AgentOutput> {
    // Load configuration for the specified provider
    let config = EvalConfig::load_for_provider(provider)
        .await?
        .with_model(model_override.map(|s| s.to_string()));

    match provider {
        EvalProvider::VertexClaude => {
            execute_with_vertex_claude(workspace, prompt, system_prompt, verbose_config, &config)
                .await
        }
        EvalProvider::Zai => {
            execute_with_zai(workspace, prompt, system_prompt, verbose_config, &config).await
        }
        EvalProvider::OpenAi => {
            execute_with_openai(workspace, prompt, system_prompt, verbose_config, &config).await
        }
    }
}

/// Execute a prompt with custom tools for specialized benchmarks.
///
/// This variant allows injecting custom tool definitions and executors,
/// which is needed for specialized benchmarks like SWE-bench.
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
    // Load configuration for the specified provider
    let config = EvalConfig::load_for_provider(provider)
        .await?
        .with_model(model_override.map(|s| s.to_string()));

    match provider {
        EvalProvider::VertexClaude => {
            execute_with_vertex_claude_and_tools(
                workspace,
                prompt,
                system_prompt,
                verbose_config,
                &config,
                additional_tools,
                custom_executor,
            )
            .await
        }
        EvalProvider::Zai => {
            execute_with_zai_and_tools(
                workspace,
                prompt,
                system_prompt,
                verbose_config,
                &config,
                additional_tools,
                custom_executor,
            )
            .await
        }
        EvalProvider::OpenAi => {
            execute_with_openai_and_tools(
                workspace,
                prompt,
                system_prompt,
                verbose_config,
                &config,
                additional_tools,
                custom_executor,
            )
            .await
        }
    }
}

/// Execute with Vertex AI Claude and custom tools.
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
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::VertexClaude,
        additional_tools,
        custom_executor,
    )
    .await
}

/// Execute with Z.AI GLM and custom tools.
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
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::Zai,
        additional_tools,
        custom_executor,
    )
    .await
}

/// Execute with OpenAI and custom tools.
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
        workspace,
        prompt,
        system_prompt,
        verbose_config,
        model,
        model_name,
        EvalProvider::OpenAi,
        additional_tools,
        custom_executor,
    )
    .await
}

/// Generic execution with any model and custom tools.
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

    // Run with custom tools
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

/// Execute with Vertex AI Claude.
async fn execute_with_vertex_claude(
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

    // Create client using service account credentials if available, otherwise fall back to ADC
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

    // Use model override if provided, otherwise use default
    let model_id = config
        .model_override
        .as_deref()
        .unwrap_or(models::CLAUDE_SONNET_4_5);
    let model_name = config
        .model_override
        .as_deref()
        .unwrap_or("Claude Sonnet 4.5");

    // Enable native web search (web_search_20250305)
    // Note: web_fetch_20250910 requires a beta header not yet supported on Vertex AI
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
async fn execute_with_zai(
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

    // Use model override if provided, otherwise use default
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
async fn execute_with_openai(
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

    // Use model override if provided, otherwise use default
    let model_id = config.model_override.as_deref().unwrap_or("gpt-5.1");
    let model_name = config.model_override.as_deref().unwrap_or("GPT-5.1");

    let client: rig_openai::Client = rig_openai::Client::new(&openai_config.api_key)
        .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {}", e))?;
    // Use completion_model which returns Responses API model (same as main app)
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
///
/// This function now delegates to the unified agentic loop from golish-ai,
/// ensuring evals test the same code path as the main application.
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
    // Map eval provider to provider name for capabilities detection
    let provider_name = match provider {
        EvalProvider::VertexClaude => "anthropic",
        EvalProvider::Zai => "zai",
        // Use openai_responses because evals use the Responses API (completion_model returns ResponsesCompletionModel)
        EvalProvider::OpenAi => "openai_responses",
    };

    // Create eval config for the unified loop
    let ai_config = AiEvalConfig {
        provider_name: provider_name.to_string(),
        model_name: model_name.to_string(),
        require_hitl: false,
        workspace: workspace.to_path_buf(),
        verbose: verbose_config.enabled,
    };

    // Build the effective system prompt:
    // - If a custom system prompt is provided (for scenario-specific tests), use it
    // - Otherwise, use the production prompt with contributions (same as main agent)
    let effective_system_prompt = match system_prompt {
        Some(custom) => custom.to_string(),
        None => build_production_system_prompt(workspace, provider),
    };

    // Run the unified agentic loop
    let eval_output = golish_ai::eval_support::run_eval_agentic_loop(
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

    // Convert from golish_ai's EvalToolCall to golish_evals' ToolCall
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
