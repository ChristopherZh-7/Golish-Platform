//! LLM-based evaluation metrics.
//!
//! Uses Vertex Claude Sonnet to evaluate agent outputs against criteria.
//!
//! ## Layout
//!
//! - [`tools`]: Tool-call wiring for the judge (read_file / list_files +
//!   their workspace-scoped executors).
//! - [`judge`]: [`LlmJudgeMetric`] — pass/fail evaluation with optional
//!   workspace-exploration tools.
//! - [`score`]: [`LlmScoreMetric`] — numeric scoring on a 0–N scale.
//!
//! Common pieces (system prompt, Vertex client constructor,
//! `extract_last_number` helper) live in this `mod.rs`.

use anyhow::Result;
use rig_anthropic_vertex::{models, Client};

use crate::config::EvalConfig;

mod judge;
mod score;
mod tools;

#[cfg(test)]
mod tests;

pub use judge::LlmJudgeMetric;
pub use score::LlmScoreMetric;

/// System prompt for LLM judge evaluations.
pub(super) const JUDGE_SYSTEM_PROMPT: &str = r#"You are an expert code reviewer evaluating AI assistant outputs.
You will be given:
1. The original task/prompt given to the assistant
2. The assistant's response
3. Evaluation criteria to judge against

Evaluate strictly and objectively. Focus on whether the criteria are met, not on style preferences.
"#;

/// Create a Vertex AI client for LLM judge evaluations.
pub(super) async fn create_judge_client() -> Result<rig_anthropic_vertex::CompletionModel> {
    // Load configuration from settings.toml with env var fallback.
    let config = EvalConfig::load().await?;

    let vertex_config = config
        .vertex
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Vertex AI configuration not available for LLM judge"))?;

    // Create client using service account credentials if available, otherwise
    // fall back to ADC.
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
    Ok(client.completion_model(models::CLAUDE_SONNET_4_5))
}

/// Extract the last number from a text response.
///
/// Handles cases where the LLM includes reasoning before the final score.
pub(super) fn extract_last_number(text: &str) -> Option<f64> {
    // Find all numbers in the text (including decimals).
    let mut last_number = None;
    let mut current_num = String::new();
    let mut in_number = false;

    for c in text.chars() {
        if c.is_ascii_digit() || (c == '.' && in_number && !current_num.contains('.')) {
            current_num.push(c);
            in_number = true;
        } else if in_number {
            if let Ok(n) = current_num.parse::<f64>() {
                last_number = Some(n);
            }
            current_num.clear();
            in_number = false;
        }
    }

    // Check the last number if we ended while still in a number.
    if in_number {
        if let Ok(n) = current_num.parse::<f64>() {
            last_number = Some(n);
        }
    }

    last_number
}
