use super::*;
use golish_llm_providers::ModelCapabilities;

#[test]
fn test_agentic_loop_config_main_agent_anthropic() {
    let config = AgenticLoopConfig::main_agent_anthropic();
    assert!(
        config.capabilities.supports_thinking_history,
        "Anthropic config should support thinking history"
    );
    assert!(
        config.capabilities.supports_temperature,
        "Anthropic config should support temperature"
    );
    assert!(config.require_hitl, "Main agent should require HITL");
    assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
}

#[test]
fn test_agentic_loop_config_main_agent_generic() {
    let config = AgenticLoopConfig::main_agent_generic();
    assert!(
        !config.capabilities.supports_thinking_history,
        "Generic config should not support thinking history"
    );
    assert!(
        config.capabilities.supports_temperature,
        "Generic config should support temperature"
    );
    assert!(config.require_hitl, "Main agent should require HITL");
    assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
}

#[test]
fn test_agentic_loop_config_sub_agent() {
    let config = AgenticLoopConfig::sub_agent(ModelCapabilities::conservative_defaults());
    assert!(
        !config.capabilities.supports_thinking_history,
        "Conservative defaults should not support thinking history"
    );
    assert!(!config.require_hitl, "Sub-agent should not require HITL");
    assert!(config.is_sub_agent, "Should be marked as sub-agent");
}

#[test]
fn test_agentic_loop_config_sub_agent_with_anthropic_capabilities() {
    let config = AgenticLoopConfig::sub_agent(ModelCapabilities::anthropic_defaults());
    assert!(
        config.capabilities.supports_thinking_history,
        "Anthropic sub-agent should support thinking history"
    );
    assert!(!config.require_hitl, "Sub-agent should not require HITL");
    assert!(config.is_sub_agent, "Should be marked as sub-agent");
}

#[test]
fn test_agentic_loop_config_with_detection_anthropic() {
    let config = AgenticLoopConfig::with_detection("anthropic", "claude-3-opus", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "Anthropic detection should enable thinking history"
    );
    assert!(
        config.capabilities.supports_temperature,
        "Anthropic detection should enable temperature"
    );
    assert!(config.require_hitl, "Non-sub-agent should require HITL");
    assert!(!config.is_sub_agent);
}

#[test]
fn test_agentic_loop_config_with_detection_openai_reasoning() {
    let config = AgenticLoopConfig::with_detection("openai", "o3-mini", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "OpenAI reasoning model should support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "OpenAI reasoning model should not support temperature"
    );
    assert!(config.require_hitl);
}

#[test]
fn test_agentic_loop_config_with_detection_openai_regular() {
    let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", false);
    assert!(
        !config.capabilities.supports_thinking_history,
        "Regular OpenAI model should not support thinking history"
    );
    assert!(
        config.capabilities.supports_temperature,
        "Regular OpenAI model should support temperature"
    );
}

#[test]
fn test_agentic_loop_config_with_detection_sub_agent() {
    let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", true);
    assert!(!config.require_hitl, "Sub-agent should not require HITL");
    assert!(config.is_sub_agent, "Should be marked as sub-agent");
}

#[test]
fn test_agentic_loop_config_with_detection_openai_gpt5_series() {
    // GPT-5 base model
    let config = AgenticLoopConfig::with_detection("openai", "gpt-5", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "GPT-5 should support thinking history (reasoning model)"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "GPT-5 should not support temperature (reasoning model)"
    );

    // GPT-5.1
    let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "GPT-5.1 should support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "GPT-5.1 should not support temperature"
    );

    // GPT-5.2
    let config = AgenticLoopConfig::with_detection("openai", "gpt-5.2", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "GPT-5.2 should support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "GPT-5.2 should not support temperature"
    );

    // GPT-5-mini
    let config = AgenticLoopConfig::with_detection("openai", "gpt-5-mini", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "GPT-5-mini should support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "GPT-5-mini should not support temperature"
    );
}

#[test]
fn test_agentic_loop_config_with_detection_openai_responses_gpt5() {
    // OpenAI Responses API with GPT-5.2
    let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "OpenAI Responses API should support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "GPT-5.2 via Responses API should not support temperature"
    );

    // Contrast with GPT-4.1 which DOES support temperature
    let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-4.1", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "OpenAI Responses API should support thinking history"
    );
    assert!(
        config.capabilities.supports_temperature,
        "GPT-4.1 via Responses API should support temperature"
    );
}

#[test]
fn test_agentic_loop_config_with_detection_openai_codex() {
    // Codex models don't support temperature
    let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1-codex-max", false);
    assert!(
        !config.capabilities.supports_temperature,
        "Codex models should not support temperature"
    );

    let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2-codex", false);
    assert!(
        !config.capabilities.supports_temperature,
        "Codex models via Responses API should not support temperature"
    );
}
