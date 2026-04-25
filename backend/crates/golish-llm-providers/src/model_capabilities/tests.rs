//! Capability detection tests.

use super::*;
use super::*;

// ========== ModelCapabilities::detect() tests ==========

#[test]
fn test_model_capabilities_anthropic() {
    // All Anthropic models support both temperature and thinking history
    let caps = ModelCapabilities::detect("anthropic", "claude-3-opus");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("anthropic", "claude-3-sonnet");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("vertex_ai_anthropic", "claude-3-5-sonnet");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("vertex_ai", "claude-opus-4-5");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_openai_reasoning_models() {
    // OpenAI reasoning models: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai", "o1");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "o1-preview");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "o3");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "o3-mini");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "o4-mini");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai_responses", "o3");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_openai_regular_models() {
    // Regular OpenAI Chat Completions models: yes temperature, no thinking history
    let caps = ModelCapabilities::detect("openai", "gpt-4o");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "gpt-4o-mini");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "gpt-4.1");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_openai_gpt5_models() {
    // GPT-5 series are reasoning models: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai", "gpt-5");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "gpt-5.2");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai", "gpt-5-mini");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_openai_responses_api() {
    // OpenAI Responses API: ALWAYS supports thinking history regardless of model
    // This is because the Responses API generates internal reasoning IDs that
    // function calls reference, and these must be preserved across turns.
    let caps = ModelCapabilities::detect("openai_responses", "gpt-4.1");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openai_responses", "gpt-4o");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // GPT-5 series are reasoning models - no temperature, yes thinking
    let caps = ModelCapabilities::detect("openai_responses", "gpt-5.2");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // o-series reasoning models don't support temperature
    let caps = ModelCapabilities::detect("openai_responses", "o3-mini");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // Codex models don't support temperature
    let caps = ModelCapabilities::detect("openai_responses", "gpt-5.1-codex-max");
    assert!(!caps.supports_temperature);
    assert!(caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_openai_reasoning_provider() {
    // "openai_reasoning" is the runtime provider_name for reasoning models
    // (gpt-5.2, gpt-5.2-codex, o-series) when routed through rig-openai-responses.
    // It must behave identically to "openai_responses" for thinking history,
    // and like "openai" for temperature (no temperature for reasoning/codex models).

    // GPT-5.2 via openai_reasoning: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai_reasoning", "gpt-5.2");
    assert!(
        !caps.supports_temperature,
        "gpt-5.2 via openai_reasoning should not support temperature"
    );
    assert!(
        caps.supports_thinking_history,
        "gpt-5.2 via openai_reasoning must support thinking history"
    );

    // GPT-5.2 Codex via openai_reasoning: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai_reasoning", "gpt-5.2-codex");
    assert!(
        !caps.supports_temperature,
        "gpt-5.2-codex via openai_reasoning should not support temperature"
    );
    assert!(
        caps.supports_thinking_history,
        "gpt-5.2-codex via openai_reasoning must support thinking history"
    );

    // o3 via openai_reasoning: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai_reasoning", "o3");
    assert!(
        !caps.supports_temperature,
        "o3 via openai_reasoning should not support temperature"
    );
    assert!(
        caps.supports_thinking_history,
        "o3 via openai_reasoning must support thinking history"
    );

    // o4-mini via openai_reasoning: no temperature, yes thinking history
    let caps = ModelCapabilities::detect("openai_reasoning", "o4-mini");
    assert!(
        !caps.supports_temperature,
        "o4-mini via openai_reasoning should not support temperature"
    );
    assert!(
        caps.supports_thinking_history,
        "o4-mini via openai_reasoning must support thinking history"
    );
}

#[test]
fn test_openai_reasoning_provider_temperature_support() {
    // "openai_reasoning" provider must apply the same temperature rules as "openai"
    assert!(!model_supports_temperature("openai_reasoning", "gpt-5.2"));
    assert!(!model_supports_temperature(
        "openai_reasoning",
        "gpt-5.2-codex"
    ));
    assert!(!model_supports_temperature("openai_reasoning", "o3"));
    assert!(!model_supports_temperature("openai_reasoning", "o4-mini"));
    assert!(!model_supports_temperature("openai_reasoning", "o1"));
    assert!(!model_supports_temperature(
        "openai_reasoning",
        "gpt-5.1-codex-max"
    ));
}

#[test]
fn test_model_capabilities_gemini() {
    // Gemini thinking model: yes temperature, yes thinking history
    let caps = ModelCapabilities::detect("gemini", "gemini-2.0-flash-thinking-exp");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // Regular Gemini: yes temperature, no thinking history
    let caps = ModelCapabilities::detect("gemini", "gemini-2.5-pro");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("gemini", "gemini-1.5-flash");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_zai() {
    // Z.AI GLM-4.7: yes temperature, yes thinking history (preserved thinking)
    let caps = ModelCapabilities::detect("zai", "GLM-4.7");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // Case insensitive
    let caps = ModelCapabilities::detect("zai", "glm-4.7");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // GLM-4.5-air: yes temperature, no explicit thinking config
    let caps = ModelCapabilities::detect("zai", "GLM-4.5-air");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_zai_sdk() {
    // zai_sdk provider (same behavior as zai)
    // GLM-4.7: yes temperature, yes thinking history (preserved thinking)
    let caps = ModelCapabilities::detect("zai_sdk", "GLM-4.7");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("zai_sdk", "glm-4.7");
    assert!(caps.supports_temperature);
    assert!(caps.supports_thinking_history);

    // GLM-4.5-air: yes temperature, no explicit thinking config
    let caps = ModelCapabilities::detect("zai_sdk", "GLM-4.5-air");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    // GLM-4-assistant: yes temperature, no thinking history
    let caps = ModelCapabilities::detect("zai_sdk", "glm-4-assistant");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_other_providers() {
    // Other providers: yes temperature, no thinking history
    let caps = ModelCapabilities::detect("groq", "llama-3.3-70b");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("ollama", "llama3.2");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("xai", "grok-2");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    // Note: Z.AI GLM-4.7 does support thinking history - tested separately below
    let caps = ModelCapabilities::detect("zai", "glm-4.5-air");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);

    let caps = ModelCapabilities::detect("openrouter", "anthropic/claude-3-opus");
    assert!(caps.supports_temperature);
    assert!(!caps.supports_thinking_history);
}

#[test]
fn test_model_capabilities_defaults() {
    let conservative = ModelCapabilities::conservative_defaults();
    assert!(conservative.supports_temperature);
    assert!(!conservative.supports_thinking_history);

    let anthropic = ModelCapabilities::anthropic_defaults();
    assert!(anthropic.supports_temperature);
    assert!(anthropic.supports_thinking_history);

    let default = ModelCapabilities::default();
    assert!(!default.supports_temperature);
    assert!(!default.supports_thinking_history);
}

// ========== Legacy function tests ==========

#[test]
fn test_openai_temperature_support() {
    // Models that DO support temperature
    assert!(model_supports_temperature("openai", "gpt-4o"));
    assert!(model_supports_temperature("openai", "gpt-4o-mini"));
    assert!(model_supports_temperature("openai", "gpt-4.1"));
    assert!(model_supports_temperature("openai", "gpt-4.1-mini"));
    assert!(model_supports_temperature("openai", "chatgpt-4o-latest"));

    // Models that do NOT support temperature (reasoning models)
    // o-series
    assert!(!model_supports_temperature("openai", "o1"));
    assert!(!model_supports_temperature("openai", "o3"));
    assert!(!model_supports_temperature("openai", "o3-mini"));
    assert!(!model_supports_temperature("openai", "o4-mini"));
    // GPT-5 series (all are reasoning models)
    assert!(!model_supports_temperature("openai", "gpt-5"));
    assert!(!model_supports_temperature("openai", "gpt-5.1"));
    assert!(!model_supports_temperature("openai", "gpt-5.2"));
    assert!(!model_supports_temperature("openai", "gpt-5-mini"));
    assert!(!model_supports_temperature("openai", "gpt-5-nano"));

    // Codex models - any variant should NOT support temperature
    assert!(!model_supports_temperature("openai", "gpt-5.1-codex"));
    assert!(!model_supports_temperature("openai", "gpt-5.1-codex-max"));
    assert!(!model_supports_temperature("openai", "codex-mini-latest"));
    assert!(!model_supports_temperature("openai", "codex-mini"));
    assert!(!model_supports_temperature("openai", "codex"));
    assert!(!model_supports_temperature("openai", "CODEX-MINI")); // case insensitive
    assert!(!model_supports_temperature(
        "openai_responses",
        "gpt-5.1-codex-max"
    )); // responses API variant
}

#[test]
fn test_other_providers_always_support_temperature() {
    assert!(model_supports_temperature("anthropic", "claude-3-opus"));
    assert!(model_supports_temperature("vertex_ai", "claude-opus-4-5"));
    assert!(model_supports_temperature("gemini", "gemini-2.5-pro"));
    assert!(model_supports_temperature("groq", "llama-3.3-70b"));
    assert!(model_supports_temperature("ollama", "llama3.2"));
    assert!(model_supports_temperature("xai", "grok-2"));
    assert!(model_supports_temperature("zai", "glm-4.7"));
    assert!(model_supports_temperature("zai_sdk", "glm-4.7"));
    assert!(model_supports_temperature("zai_sdk", "glm-4.6v"));
}

#[test]
fn test_openai_web_search_support() {
    // Models that DO support web search
    assert!(openai_supports_web_search("gpt-4o"));
    assert!(openai_supports_web_search("gpt-4o-mini"));
    assert!(openai_supports_web_search("chatgpt-4o-latest"));
    assert!(openai_supports_web_search("gpt-4.1"));
    assert!(openai_supports_web_search("gpt-4.1-mini"));
    assert!(openai_supports_web_search("gpt-5"));
    assert!(openai_supports_web_search("gpt-5.1"));
    assert!(openai_supports_web_search("gpt-5.2"));

    // Models that do NOT support web search (reasoning models, etc.)
    assert!(!openai_supports_web_search("o1"));
    assert!(!openai_supports_web_search("o3"));
    assert!(!openai_supports_web_search("o3-mini"));
    assert!(!openai_supports_web_search("o4-mini"));
    assert!(!openai_supports_web_search("codex-mini"));
    assert!(!openai_supports_web_search("gpt-3.5-turbo"));
}

// ========== VisionCapabilities::detect() tests ==========

#[test]
fn test_vision_capabilities_zai() {
    // Vision models have "v" suffix
    let caps = VisionCapabilities::detect("zai", "glm-4v");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 10 * 1024 * 1024);
    assert!(!caps.supported_formats.is_empty());

    let caps = VisionCapabilities::detect("zai", "glm-4.6v");
    assert!(caps.supports_vision);

    let caps = VisionCapabilities::detect("zai", "GLM-4V"); // case insensitive
    assert!(caps.supports_vision);

    // Non-vision models
    let caps = VisionCapabilities::detect("zai", "glm-4.7");
    assert!(!caps.supports_vision);

    let caps = VisionCapabilities::detect("zai", "glm-4-assistant");
    assert!(!caps.supports_vision);

    let caps = VisionCapabilities::detect("zai", "GLM-4.5-air");
    assert!(!caps.supports_vision);
}

#[test]
fn test_vision_capabilities_zai_sdk() {
    // zai_sdk provider (same behavior as zai)
    // Vision models have "v" suffix
    let caps = VisionCapabilities::detect("zai_sdk", "glm-4v");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 10 * 1024 * 1024);

    let caps = VisionCapabilities::detect("zai_sdk", "glm-4.6v");
    assert!(caps.supports_vision);

    // Non-vision models
    let caps = VisionCapabilities::detect("zai_sdk", "glm-4.7");
    assert!(!caps.supports_vision);

    let caps = VisionCapabilities::detect("zai_sdk", "glm-4-assistant");
    assert!(!caps.supports_vision);
}

#[test]
fn test_vision_capabilities_anthropic() {
    // Claude 3+ models support vision
    let caps = VisionCapabilities::detect("anthropic", "claude-3-opus");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 5 * 1024 * 1024);

    let caps = VisionCapabilities::detect("anthropic", "claude-sonnet-4-5");
    assert!(caps.supports_vision);

    let caps = VisionCapabilities::detect("vertex_ai", "claude-opus-4-5");
    assert!(caps.supports_vision);
}

#[test]
fn test_vision_capabilities_openai() {
    // GPT-4+ and o-series support vision
    let caps = VisionCapabilities::detect("openai", "gpt-4o");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 20 * 1024 * 1024);

    let caps = VisionCapabilities::detect("openai", "o3-mini");
    assert!(caps.supports_vision);

    // GPT-3.5 doesn't support vision
    let caps = VisionCapabilities::detect("openai", "gpt-3.5-turbo");
    assert!(!caps.supports_vision);

    // openai_reasoning provider (used for o-series models)
    let caps = VisionCapabilities::detect("openai_reasoning", "o3-mini");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 20 * 1024 * 1024);

    let caps = VisionCapabilities::detect("openai_reasoning", "o1");
    assert!(caps.supports_vision);
}

#[test]
fn test_vision_capabilities_gemini() {
    // All Gemini models support vision (direct API)
    let caps = VisionCapabilities::detect("gemini", "gemini-2.5-pro");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 20 * 1024 * 1024);

    // Vertex AI Gemini also supports vision (both naming conventions)
    let caps = VisionCapabilities::detect("vertex_ai_gemini", "gemini-2.5-pro");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 20 * 1024 * 1024);

    let caps = VisionCapabilities::detect("vertex_ai_gemini", "gemini-2.0-flash");
    assert!(caps.supports_vision);

    // vertex_gemini is the actual provider name used in llm_client.rs
    let caps = VisionCapabilities::detect("vertex_gemini", "gemini-2.5-pro");
    assert!(caps.supports_vision);
    assert_eq!(caps.max_image_size_bytes, 20 * 1024 * 1024);

    let caps = VisionCapabilities::detect("vertex_gemini", "gemini-2.0-flash");
    assert!(caps.supports_vision);
}

#[test]
fn test_vision_capabilities_no_support() {
    // Providers without vision support
    let caps = VisionCapabilities::detect("ollama", "llama3.2");
    assert!(!caps.supports_vision);
    assert!(caps.supported_formats.is_empty());

    let caps = VisionCapabilities::detect("groq", "llama-3.3-70b");
    assert!(!caps.supports_vision);

    let caps = VisionCapabilities::detect("xai", "grok-2");
    assert!(!caps.supports_vision);
}
