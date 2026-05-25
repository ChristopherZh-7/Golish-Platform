//! Vertex AI Gemini model definitions, loaded from
//! `resources/llm-models/vertex_gemini.json`.
//!
//! These are Gemini models accessed via Google Cloud Vertex AI (using service
//! account or ADC authentication), as opposed to the `gemini_models()` which
//! use the AI Studio API.
//!
//! See `docs/design/2026-05-25-llm-models-json-driven.md` for the migration
//! rationale. To add / remove / update models, edit the JSON file.

use golish_settings::schema::AiProvider;

use crate::descriptors::embedded_defaults_for;
use crate::registry::ModelDefinition;

/// Vertex AI Gemini model definitions.
pub fn vertex_gemini_models() -> Vec<ModelDefinition> {
    embedded_defaults_for(AiProvider::VertexGemini)
}
