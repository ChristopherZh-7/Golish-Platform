//! Single-turn eval execution: dispatch to the right provider and run the
//! unified agentic loop once.

mod providers;
mod with_tools;

use std::path::Path;

use anyhow::Result;

use crate::config::{EvalConfig, EvalProvider};
use crate::runner::{AgentOutput, VerboseConfig};

pub use with_tools::execute_eval_prompt_with_tools;

/// Execute a prompt against the agent in the given workspace using the default provider.
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
    let config = EvalConfig::load_for_provider(provider)
        .await?
        .with_model(model_override.map(|s| s.to_string()));

    match provider {
        EvalProvider::VertexClaude => {
            providers::execute_with_vertex_claude(workspace, prompt, system_prompt, verbose_config, &config)
                .await
        }
        EvalProvider::Zai => {
            providers::execute_with_zai(workspace, prompt, system_prompt, verbose_config, &config).await
        }
        EvalProvider::OpenAi => {
            providers::execute_with_openai(workspace, prompt, system_prompt, verbose_config, &config).await
        }
    }
}
