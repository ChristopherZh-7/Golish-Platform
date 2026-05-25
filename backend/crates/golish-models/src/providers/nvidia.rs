//! NVIDIA NIM model definitions.
//!
//! The catalog is now loaded from `resources/llm-models/nvidia.json` via the
//! `descriptors` module. See `docs/design/2026-05-25-llm-models-json-driven.md`
//! for the rationale and migration plan.
//!
//! To add / remove / update NVIDIA NIM models, edit the JSON file. No Rust
//! code changes are required, and the loader will validate the JSON against
//! `ProviderModelsFile` at startup. If the JSON is missing or invalid, the
//! loader falls back to the embedded copy compiled into the binary so the
//! application never starts in a degraded state.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// NVIDIA NIM model definitions, sourced from `resources/llm-models/nvidia.json`
/// (with embedded fallback baked into the binary).
pub fn nvidia_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::Nvidia)
}
