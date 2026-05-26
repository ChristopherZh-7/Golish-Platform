//! JSON-driven model registry descriptors.
//!
//! See `docs/design/2026-05-25-llm-models-json-driven.md` for the motivation
//! and architecture. This module owns the deserialization types that map a
//! `resources/llm-models/<provider>.json` file into runtime `ModelDefinition`
//! values, with optional `capabilities.base` references and field overrides.

mod capabilities_base;
mod loader;

pub use capabilities_base::{merge_capabilities, resolve_capabilities_base};
pub use loader::{embedded_defaults_for, load_provider_models};

use serde::{Deserialize, Serialize};

/// Top-level shape of a `resources/llm-models/<provider>.json` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelsFile {
    /// Provider slug (must match `AiProvider` variants like `"nvidia"`).
    pub provider: String,
    /// Optional default capabilities base used when a model does not specify one.
    #[serde(default)]
    pub default_capabilities_base: Option<String>,
    /// Model descriptors.
    pub models: Vec<ModelDescriptor>,
}

/// One model entry inside a `ProviderModelsFile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Unique model identifier (e.g. `"moonshotai/kimi-k2.6"`).
    pub id: String,
    /// Human readable display name.
    pub display_name: String,
    /// Capability overrides.
    #[serde(default)]
    pub capabilities: CapabilitiesDescriptor,
    /// Alternative IDs that resolve to this model.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Optional thinking-quirk classification (currently informational).
    #[serde(default)]
    pub thinking_quirks: Option<String>,
}

/// Capability override description. Any `None` field falls through to the
/// resolved base capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilitiesDescriptor {
    /// Name of a capability base (e.g. `"nvidia_large_defaults"`).
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub supports_temperature: Option<bool>,
    #[serde(default)]
    pub supports_thinking_history: Option<bool>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    #[serde(default)]
    pub supports_web_search: Option<bool>,
    #[serde(default)]
    pub is_reasoning_model: Option<bool>,
    #[serde(default)]
    pub is_codex_model: Option<bool>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_provider_models_file() {
        let raw = r#"{
            "provider": "nvidia",
            "default_capabilities_base": "nvidia_defaults",
            "models": [
                {
                    "id": "moonshotai/kimi-k2.6",
                    "display_name": "Kimi K2.6",
                    "capabilities": {
                        "base": "nvidia_large_defaults",
                        "context_window": 256000,
                        "supports_thinking_history": true
                    },
                    "aliases": ["kimi-k2.6", "kimi-k2-6"],
                    "thinking_quirks": "explicit_thinking"
                }
            ]
        }"#;
        let parsed: ProviderModelsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.provider, "nvidia");
        assert_eq!(
            parsed.default_capabilities_base.as_deref(),
            Some("nvidia_defaults")
        );
        assert_eq!(parsed.models.len(), 1);
        let m = &parsed.models[0];
        assert_eq!(m.id, "moonshotai/kimi-k2.6");
        assert_eq!(m.display_name, "Kimi K2.6");
        assert_eq!(
            m.capabilities.base.as_deref(),
            Some("nvidia_large_defaults")
        );
        assert_eq!(m.capabilities.context_window, Some(256_000));
        assert_eq!(m.capabilities.supports_thinking_history, Some(true));
        assert_eq!(m.aliases, vec!["kimi-k2.6", "kimi-k2-6"]);
        assert_eq!(m.thinking_quirks.as_deref(), Some("explicit_thinking"));
    }

    #[test]
    fn rejects_missing_id() {
        let raw = r#"{
            "provider": "nvidia",
            "models": [{ "display_name": "X" }]
        }"#;
        let parsed = serde_json::from_str::<ProviderModelsFile>(raw);
        assert!(parsed.is_err(), "must reject model entry without `id`");
    }

    #[test]
    fn default_capabilities_descriptor_is_all_none() {
        let desc = CapabilitiesDescriptor::default();
        assert!(desc.base.is_none());
        assert!(desc.context_window.is_none());
        assert!(desc.supports_temperature.is_none());
        assert!(desc.supports_thinking_history.is_none());
    }
}
