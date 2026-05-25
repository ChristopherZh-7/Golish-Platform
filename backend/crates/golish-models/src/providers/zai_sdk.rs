//! Z.AI SDK model definitions, loaded from `resources/llm-models/zai_sdk.json`.
//!
//! See `docs/design/2026-05-25-llm-models-json-driven.md` for the migration
//! rationale. To add / remove / update models, edit the JSON file.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// Z.AI SDK model definitions.
pub fn zai_sdk_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::ZaiSdk)
}
