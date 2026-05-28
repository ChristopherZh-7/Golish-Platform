//! Resolve a capabilities base name to a `ModelCapabilities` value and merge
//! overrides on top.

use crate::capabilities::ModelCapabilities;
use crate::descriptors::CapabilitiesDescriptor;

/// Resolve a `capabilities.base` string to the corresponding base
/// `ModelCapabilities`. Unknown / `None` falls back to
/// `ModelCapabilities::conservative_defaults()`.
///
/// The complete set of registered bases matches the public functions on
/// `ModelCapabilities`. Edit both sides if new bases are introduced.
pub fn resolve_capabilities_base(name: Option<&str>) -> ModelCapabilities {
    match name {
        // Generic
        Some("conservative_defaults") => ModelCapabilities::conservative_defaults(),
        // Anthropic family
        Some("anthropic_defaults") => ModelCapabilities::anthropic_defaults(),
        Some("anthropic_sonnet_4_6") => ModelCapabilities::anthropic_sonnet_4_6(),
        Some("anthropic_opus_4_6") => ModelCapabilities::anthropic_opus_4_6(),
        // OpenAI family
        Some("openai_gpt4_defaults") => ModelCapabilities::openai_gpt4_defaults(),
        Some("openai_gpt5_defaults") => ModelCapabilities::openai_gpt5_defaults(),
        Some("openai_o_series_defaults") => ModelCapabilities::openai_o_series_defaults(),
        Some("openai_codex_defaults") => ModelCapabilities::openai_codex_defaults(),
        // Gemini family
        Some("gemini_defaults") => ModelCapabilities::gemini_defaults(),
        Some("gemini_2_0_flash_lite_defaults") => {
            ModelCapabilities::gemini_2_0_flash_lite_defaults()
        }
        // Groq
        Some("groq_defaults") => ModelCapabilities::groq_defaults(),
        // xAI
        Some("xai_defaults") => ModelCapabilities::xai_defaults(),
        // Z.AI
        Some("zai_defaults") => ModelCapabilities::zai_defaults(),
        Some("zai_thinking_defaults") => ModelCapabilities::zai_thinking_defaults(),
        Some("zai_vision_defaults") => ModelCapabilities::zai_vision_defaults(),
        // NVIDIA NIM
        Some("nvidia_defaults") => ModelCapabilities::nvidia_defaults(),
        Some("nvidia_large_defaults") => ModelCapabilities::nvidia_large_defaults(),
        Some("nvidia_small_defaults") => ModelCapabilities::nvidia_small_defaults(),
        // DeepSeek direct
        Some("deepseek_defaults") => ModelCapabilities::deepseek_defaults(),
        // Xiaomi MiMo
        Some("xiaomi_defaults") => ModelCapabilities::xiaomi_defaults(),
        // Ollama
        Some("ollama_defaults") => ModelCapabilities::ollama_defaults(),
        // Anything else: warn-and-fallback.
        _ => ModelCapabilities::conservative_defaults(),
    }
}

/// Apply field-level overrides from `desc` on top of `base` and return the
/// merged capability set.
pub fn merge_capabilities(
    mut base: ModelCapabilities,
    desc: &CapabilitiesDescriptor,
) -> ModelCapabilities {
    if let Some(v) = desc.supports_temperature {
        base.supports_temperature = v;
    }
    if let Some(v) = desc.supports_thinking_history {
        base.supports_thinking_history = v;
    }
    if let Some(v) = desc.supports_vision {
        base.supports_vision = v;
    }
    if let Some(v) = desc.supports_web_search {
        base.supports_web_search = v;
    }
    if let Some(v) = desc.is_reasoning_model {
        base.is_reasoning_model = v;
    }
    if let Some(v) = desc.is_codex_model {
        base.is_codex_model = v;
    }
    if let Some(v) = desc.context_window {
        base.context_window = v;
    }
    if let Some(v) = desc.max_output_tokens {
        base.max_output_tokens = v;
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_nvidia_large_defaults_base() {
        let caps = resolve_capabilities_base(Some("nvidia_large_defaults"));
        assert_eq!(caps, ModelCapabilities::nvidia_large_defaults());
    }

    #[test]
    fn resolves_nvidia_small_defaults_base() {
        let caps = resolve_capabilities_base(Some("nvidia_small_defaults"));
        assert_eq!(caps, ModelCapabilities::nvidia_small_defaults());
    }

    #[test]
    fn resolves_nvidia_defaults_base() {
        let caps = resolve_capabilities_base(Some("nvidia_defaults"));
        assert_eq!(caps, ModelCapabilities::nvidia_defaults());
    }

    #[test]
    fn unknown_base_falls_back_to_conservative() {
        let caps = resolve_capabilities_base(Some("zzz_unknown_base"));
        assert_eq!(caps, ModelCapabilities::conservative_defaults());
    }

    #[test]
    fn none_base_returns_conservative() {
        let caps = resolve_capabilities_base(None);
        assert_eq!(caps, ModelCapabilities::conservative_defaults());
    }

    #[test]
    fn merge_overrides_context_window() {
        let base = ModelCapabilities::nvidia_defaults();
        let desc = CapabilitiesDescriptor {
            context_window: Some(1_000_000),
            ..CapabilitiesDescriptor::default()
        };
        let merged = merge_capabilities(base, &desc);
        assert_eq!(merged.context_window, 1_000_000);
    }

    #[test]
    fn merge_overrides_vision_and_thinking() {
        let base = ModelCapabilities::nvidia_large_defaults();
        let desc = CapabilitiesDescriptor {
            supports_vision: Some(true),
            supports_thinking_history: Some(true),
            ..CapabilitiesDescriptor::default()
        };
        let merged = merge_capabilities(base, &desc);
        assert!(merged.supports_vision);
        assert!(merged.supports_thinking_history);
    }

    #[test]
    fn merge_empty_descriptor_returns_base() {
        let base = ModelCapabilities::nvidia_defaults();
        let merged = merge_capabilities(base.clone(), &CapabilitiesDescriptor::default());
        assert_eq!(merged, base);
    }
}
