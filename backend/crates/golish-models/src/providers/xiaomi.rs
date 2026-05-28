//! Xiaomi MiMo Token Plan model definitions, loaded from
//! `resources/llm-models/xiaomi.json`.
//!
//! See `docs/design/2026-05-27-add-xiaomi-mimo-provider.md` for the design.
//! To add / remove / update models, edit the JSON file.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// Xiaomi MiMo Token Plan model definitions.
pub fn xiaomi_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::Xiaomi)
}
