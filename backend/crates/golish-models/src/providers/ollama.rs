//! Ollama default model definitions, loaded from
//! `resources/llm-models/ollama.json`.
//!
//! Note: Ollama models vary by installation. These are common defaults.
//! Use `discover_ollama_models()` in the registry module for dynamic
//! discovery.
//!
//! See `docs/design/2026-05-25-llm-models-json-driven.md` for the migration
//! rationale. To add / remove / update models, edit the JSON file.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// Ollama default model definitions.
pub fn ollama_default_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::Ollama)
}
