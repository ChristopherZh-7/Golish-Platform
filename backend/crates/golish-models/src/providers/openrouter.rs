//! OpenRouter model definitions, loaded from
//! `resources/llm-models/openrouter.json`.
//!
//! Note: OpenRouter provides access to many models. These are curated defaults.
//!
//! See `docs/design/2026-05-25-llm-models-json-driven.md` for the migration
//! rationale. To add / remove / update models, edit the JSON file.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// OpenRouter model definitions.
pub fn openrouter_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::Openrouter)
}
