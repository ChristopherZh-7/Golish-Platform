//! Loader that materialises a `ProviderModelsFile` (either from disk or from
//! the embedded fallback) into `Vec<ModelDefinition>`.

use std::path::Path;

use golish_settings::schema::AiProvider;

use crate::descriptors::capabilities_base::{merge_capabilities, resolve_capabilities_base};
use crate::descriptors::{ModelDescriptor, ProviderModelsFile};
use crate::registry::ModelDefinition;

/// Slug used to look up `resources/llm-models/<slug>.json`.
pub(crate) fn provider_slug(provider: AiProvider) -> &'static str {
    match provider {
        AiProvider::Nvidia => "nvidia",
        AiProvider::Openai => "openai",
        AiProvider::Anthropic => "anthropic",
        AiProvider::Gemini => "gemini",
        AiProvider::VertexAi => "vertex_ai",
        AiProvider::VertexGemini => "vertex_gemini",
        AiProvider::Groq => "groq",
        AiProvider::Xai => "xai",
        AiProvider::ZaiSdk => "zai_sdk",
        AiProvider::Ollama => "ollama",
        AiProvider::Openrouter => "openrouter",
        AiProvider::Deepseek => "deepseek",
    }
}

/// Try to load a provider's models from `resource_dir/<provider>.json`,
/// falling back to the embedded defaults on any error.
pub fn load_provider_models(provider: AiProvider, resource_dir: &Path) -> Vec<ModelDefinition> {
    let slug = provider_slug(provider);
    let path = resource_dir.join(format!("{slug}.json"));
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<ProviderModelsFile>(&text) {
            Ok(file) => file_into_definitions(file, provider),
            Err(err) => {
                tracing::warn!(
                    target = "golish_models",
                    provider = ?provider,
                    path = %path.display(),
                    error = %err,
                    "LLM models JSON parse error; falling back to embedded defaults"
                );
                embedded_defaults_for(provider)
            }
        },
        Err(err) => {
            tracing::debug!(
                target = "golish_models",
                provider = ?provider,
                path = %path.display(),
                error = %err,
                "LLM models JSON not found; using embedded defaults"
            );
            embedded_defaults_for(provider)
        }
    }
}

/// Embedded fallback. Returns the same model set as if the resource file were
/// present (the resource file is `include_str!`-baked into the binary).
///
/// Each provider's JSON lives at `resources/llm-models/<slug>.json` and is
/// baked into the binary so the application never starts with an empty
/// registry, even when the on-disk resource file is missing or corrupt.
pub fn embedded_defaults_for(provider: AiProvider) -> Vec<ModelDefinition> {
    let raw: &str = match provider {
        AiProvider::Nvidia => include_str!("../../../../../resources/llm-models/nvidia.json"),
        AiProvider::Anthropic => {
            include_str!("../../../../../resources/llm-models/anthropic.json")
        }
        AiProvider::Openai => include_str!("../../../../../resources/llm-models/openai.json"),
        AiProvider::Gemini => include_str!("../../../../../resources/llm-models/gemini.json"),
        AiProvider::VertexAi => {
            include_str!("../../../../../resources/llm-models/vertex_ai.json")
        }
        AiProvider::VertexGemini => {
            include_str!("../../../../../resources/llm-models/vertex_gemini.json")
        }
        AiProvider::Groq => include_str!("../../../../../resources/llm-models/groq.json"),
        AiProvider::Xai => include_str!("../../../../../resources/llm-models/xai.json"),
        AiProvider::ZaiSdk => include_str!("../../../../../resources/llm-models/zai_sdk.json"),
        AiProvider::Ollama => include_str!("../../../../../resources/llm-models/ollama.json"),
        AiProvider::Openrouter => {
            include_str!("../../../../../resources/llm-models/openrouter.json")
        }
        AiProvider::Deepseek => {
            include_str!("../../../../../resources/llm-models/deepseek.json")
        }
    };
    let file: ProviderModelsFile = serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("embedded {provider:?}.json must parse: {e}"));
    file_into_definitions(file, provider)
}

fn file_into_definitions(
    file: ProviderModelsFile,
    provider: AiProvider,
) -> Vec<ModelDefinition> {
    let default_base = file.default_capabilities_base.clone();
    file.models
        .into_iter()
        .map(|m| descriptor_into_definition(m, provider, default_base.as_deref()))
        .collect()
}

fn descriptor_into_definition(
    desc: ModelDescriptor,
    provider: AiProvider,
    default_base: Option<&str>,
) -> ModelDefinition {
    let base_name = desc
        .capabilities
        .base
        .as_deref()
        .or(default_base);
    let base = resolve_capabilities_base(base_name);
    let capabilities = merge_capabilities(base, &desc.capabilities);

    // ModelDefinition uses `&'static str`. JSON strings live for the program
    // lifetime once we leak them, so this is safe: the registry is initialised
    // once at startup and never freed.
    let id: &'static str = Box::leak(desc.id.into_boxed_str());
    let display_name: &'static str = Box::leak(desc.display_name.into_boxed_str());
    let aliases: &'static [&'static str] = leak_str_slice(desc.aliases);

    ModelDefinition {
        id,
        display_name,
        provider,
        capabilities,
        aliases,
    }
}

fn leak_str_slice(strings: Vec<String>) -> &'static [&'static str] {
    let static_strs: Vec<&'static str> = strings
        .into_iter()
        .map(|s| -> &'static str { Box::leak(s.into_boxed_str()) })
        .collect();
    Box::leak(static_strs.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loader_returns_embedded_defaults_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let models = load_provider_models(AiProvider::Nvidia, dir.path());
        assert!(
            !models.is_empty(),
            "embedded fallback must yield NVIDIA models"
        );
    }

    #[test]
    fn loader_prefers_resource_file_when_present() {
        let dir = TempDir::new().unwrap();
        let json = r#"{
            "provider": "nvidia",
            "models": [{
                "id": "test/synthetic",
                "display_name": "Synthetic",
                "capabilities": { "base": "nvidia_defaults" },
                "aliases": []
            }]
        }"#;
        std::fs::write(dir.path().join("nvidia.json"), json).unwrap();
        let models = load_provider_models(AiProvider::Nvidia, dir.path());
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "test/synthetic");
        assert_eq!(models[0].display_name, "Synthetic");
    }

    #[test]
    fn loader_falls_back_when_json_corrupt() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("nvidia.json"), b"{ not json").unwrap();
        let models = load_provider_models(AiProvider::Nvidia, dir.path());
        // Embedded fallback still returns the 28 NVIDIA models.
        assert!(!models.is_empty(), "should fall back on parse error");
    }

    #[test]
    fn embedded_nvidia_has_expected_count() {
        let models = embedded_defaults_for(AiProvider::Nvidia);
        assert!(
            models.len() >= 20,
            "embedded NVIDIA registry should be substantial; got {}",
            models.len()
        );
    }

    #[test]
    fn every_provider_has_embedded_json_after_phase_2() {
        // Phase 2 migrated all 12 providers to JSON-driven registries.
        // Every provider must ship a non-empty embedded fallback so the
        // application can start even with the on-disk resource files missing.
        for p in [
            AiProvider::VertexAi,
            AiProvider::VertexGemini,
            AiProvider::Anthropic,
            AiProvider::Openai,
            AiProvider::Gemini,
            AiProvider::Groq,
            AiProvider::Xai,
            AiProvider::ZaiSdk,
            AiProvider::Ollama,
            AiProvider::Openrouter,
            AiProvider::Nvidia,
            AiProvider::Deepseek,
        ] {
            let models = embedded_defaults_for(p);
            assert!(
                !models.is_empty(),
                "provider {p:?} has no embedded JSON registry"
            );
        }
    }

    /// Quality gate: the NVIDIA registry must contain key flagship models we
    /// committed to ship in `nvidia.json`. This catches accidental deletions
    /// when editors hand-edit the resource file. Models listed here are the
    /// ones documented in `docs/design/2026-05-25-llm-models-json-driven.md`.
    #[test]
    fn nvidia_registry_contains_required_flagship_models() {
        let models = embedded_defaults_for(AiProvider::Nvidia);
        let ids: std::collections::HashSet<&str> = models.iter().map(|m| m.id).collect();

        let required = [
            "moonshotai/kimi-k2.6",
            "deepseek-ai/deepseek-v4-flash",
            "deepseek-ai/deepseek-v3.1-terminus",
            "z-ai/glm-5.1",
            "minimaxai/minimax-m2.7",
            "mistralai/mistral-medium-3.5-128b",
            "nvidia/nemotron-3-super-120b-a12b",
            "qwen/qwen3-coder-480b-a35b-instruct",
            "google/gemma-4-31b-it",
        ];
        for id in required {
            assert!(
                ids.contains(id),
                "required flagship model `{id}` missing from nvidia.json"
            );
        }
    }

    /// Sanity check: the parsed NVIDIA registry must round-trip through the
    /// same code path that `providers::nvidia_models()` uses; if this fails,
    /// the loader is broken (not the JSON content).
    #[test]
    fn providers_nvidia_models_matches_loader_output() {
        let from_providers = crate::providers::nvidia_models();
        let from_loader = embedded_defaults_for(AiProvider::Nvidia);
        assert_eq!(
            from_providers.len(),
            from_loader.len(),
            "providers::nvidia_models() and loader output diverge"
        );
    }
}
