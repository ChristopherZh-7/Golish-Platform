//! [\]: aggregate capability flags for a (provider, model)
//! pair, plus the underlying detect_thinking_history_support helper.

use crate::reasoning_models::is_reasoning_model;

use super::helpers::model_supports_temperature;


// ============================================================================
// Model Capabilities (existing)
// ============================================================================

/// Capabilities that vary across LLM providers/models.
///
/// This struct provides a unified way to query model capabilities
/// that affect how the agent loop behaves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// Whether the model supports the temperature parameter.
    ///
    /// Most models support temperature, but OpenAI reasoning models (o1, o3)
    /// and some codex models do not.
    pub supports_temperature: bool,

    /// Whether thinking/reasoning should be tracked in message history.
    ///
    /// Some models produce reasoning traces that should be preserved in
    /// the conversation history:
    /// - Anthropic: All models (extended thinking feature)
    /// - OpenAI: Reasoning models (o1, o3 series)
    /// - Gemini: gemini-2.0-flash-thinking-exp
    pub supports_thinking_history: bool,
}

impl ModelCapabilities {
    /// Detect capabilities based on provider and model name.
    ///
    /// # Arguments
    /// * `provider_name` - The provider identifier (e.g., "openai", "anthropic", "vertex_ai_anthropic")
    /// * `model_name` - The model identifier (e.g., "gpt-4o", "claude-3-opus", "o3-mini")
    ///
    /// # Examples
    /// ```
    /// use golish_llm_providers::ModelCapabilities;
    ///
    /// // Anthropic models support thinking history
    /// let caps = ModelCapabilities::detect("anthropic", "claude-3-opus");
    /// assert!(caps.supports_temperature);
    /// assert!(caps.supports_thinking_history);
    ///
    /// // OpenAI reasoning models don't support temperature but do support thinking history
    /// let caps = ModelCapabilities::detect("openai", "o3-mini");
    /// assert!(!caps.supports_temperature);
    /// assert!(caps.supports_thinking_history);
    ///
    /// // Regular OpenAI models support temperature but not thinking history
    /// let caps = ModelCapabilities::detect("openai", "gpt-4o");
    /// assert!(caps.supports_temperature);
    /// assert!(!caps.supports_thinking_history);
    /// ```
    pub fn detect(provider_name: &str, model_name: &str) -> Self {
        let supports_temperature = model_supports_temperature(provider_name, model_name);
        let supports_thinking_history = detect_thinking_history_support(provider_name, model_name);

        Self {
            supports_temperature,
            supports_thinking_history,
        }
    }

    /// Create capabilities with conservative defaults.
    ///
    /// This is useful when the model name is not known at client creation time.
    /// Returns capabilities that are safe for most models.
    pub fn conservative_defaults() -> Self {
        Self {
            supports_temperature: true,
            supports_thinking_history: false,
        }
    }

    /// Create capabilities for Anthropic models.
    ///
    /// All Anthropic models support temperature and thinking history.
    pub fn anthropic_defaults() -> Self {
        Self {
            supports_temperature: true,
            supports_thinking_history: true,
        }
    }
}

/// Detect if a model supports thinking history based on provider and model name.
fn detect_thinking_history_support(provider_name: &str, model_name: &str) -> bool {
    let model_lower = model_name.to_lowercase();

    match provider_name {
        // All Anthropic models support extended thinking
        "anthropic" | "anthropic_vertex" | "vertex_ai_anthropic" | "vertex_ai" => true,

        // OpenAI Responses API: Enable thinking mode for reasoning capabilities.
        // Note: While the API generates reasoning IDs (rs_...), these should NOT be included
        // in conversation history across turns. Including them causes:
        // "Item 'rs_...' of type 'reasoning' was provided without its required following item."
        // The agentic_loop handles this by checking provider_name and excluding reasoning from
        // history for openai_responses. This flag enables thinking mode for the current turn only.
        "openai_responses" => true,

        // OpenAI Responses API for reasoning models (gpt-5.2, gpt-5.2-codex, o-series).
        // This is the runtime provider_name set by create_openai_components() when
        // is_reasoning_model() returns true. These models produce explicit reasoning items
        // (rs_... IDs) and their thinking content must be preserved across turns.
        "openai_reasoning" => true,

        // OpenAI Chat Completions API: Reasoning models (o-series and gpt-5 series)
        // produce reasoning items that need to be preserved.
        "openai" => is_reasoning_model(model_name),

        // Gemini: Only the thinking-exp model
        "gemini" => model_lower.contains("thinking"),

        // Z.AI: GLM-4.7 supports preserved thinking mode via reasoning_content field
        // The provider always sends thinking: true
        // GLM-4.5 supports interleaved thinking but not explicit thinking config
        "zai" | "zai_sdk" => model_lower.contains("glm-4.7"),

        // All other providers: no thinking history support
        _ => false,
    }
}
