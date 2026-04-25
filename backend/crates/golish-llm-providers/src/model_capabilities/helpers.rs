//! Per-provider helpers for narrow capability questions:
//! [`model_supports_temperature`] and [`openai_supports_web_search`].

use crate::reasoning_models::is_reasoning_model;


/// Check if a model supports the temperature parameter.
///
/// # Arguments
/// * `provider` - The provider name (e.g., "openai", "anthropic", "vertex_ai")
/// * `model` - The model identifier
///
/// # Returns
/// `true` if the model supports temperature, `false` otherwise.
///
/// # Examples
/// ```
/// use golish_llm_providers::model_supports_temperature;
///
/// assert!(model_supports_temperature("openai", "gpt-4o"));
/// assert!(model_supports_temperature("openai", "gpt-4.1"));
/// assert!(!model_supports_temperature("openai", "o3"));
/// assert!(!model_supports_temperature("openai", "gpt-5"));
/// assert!(!model_supports_temperature("openai", "gpt-5.2"));
/// assert!(!model_supports_temperature("openai", "codex-mini"));
/// assert!(model_supports_temperature("anthropic", "claude-3-opus"));
/// ```
pub fn model_supports_temperature(provider: &str, model: &str) -> bool {
    match provider {
        "openai" | "openai_responses" | "openai_reasoning" => {
            let model_lower = model.to_lowercase();

            // Codex models don't support temperature (any variant)
            if model_lower.contains("codex") {
                return false;
            }

            // Reasoning models (o-series and gpt-5 series) don't support temperature
            if is_reasoning_model(model) {
                return false;
            }

            // All other OpenAI models support temperature
            true
        }
        // All other providers support temperature
        _ => true,
    }
}

/// OpenAI models that support the web_search_preview tool.
///
/// Based on OpenAI's documentation, web search is available for:
/// - GPT-4o series (gpt-4o, gpt-4o-mini, chatgpt-4o-latest)
/// - GPT-4.1 series (gpt-4.1, gpt-4.1-mini, gpt-4.1-nano)
/// - GPT-5 series (gpt-5, gpt-5.1, gpt-5.2, gpt-5-mini, gpt-5-nano)
const OPENAI_WEB_SEARCH_MODELS: &[&str] = &[
    // GPT-4o series
    "gpt-4o",
    "gpt-4o-mini",
    "chatgpt-4o-latest",
    // GPT-4.1 series
    "gpt-4.1",
    "gpt-4.1-mini",
    "gpt-4.1-nano",
    // GPT-5 series
    "gpt-5",
    "gpt-5.1",
    "gpt-5.2",
    "gpt-5-mini",
    "gpt-5-nano",
];

/// Check if an OpenAI model supports the native web search tool.
///
/// # Arguments
/// * `model` - The model identifier
///
/// # Returns
/// `true` if the model supports web search, `false` otherwise.
///
/// # Examples
/// ```
/// use golish_llm_providers::openai_supports_web_search;
///
/// assert!(openai_supports_web_search("gpt-4o"));
/// assert!(openai_supports_web_search("gpt-5.1"));
/// assert!(!openai_supports_web_search("o3"));
/// ```
pub fn openai_supports_web_search(model: &str) -> bool {
    OPENAI_WEB_SEARCH_MODELS
        .iter()
        .any(|m| model.to_lowercase().contains(&m.to_lowercase()))
}

