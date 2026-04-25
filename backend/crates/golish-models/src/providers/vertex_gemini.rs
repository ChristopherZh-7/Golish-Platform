//! `vertex_gemini_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Vertex AI Gemini model definitions.
///
/// These are Gemini models accessed via Google Cloud Vertex AI (using
/// service account or ADC authentication), as opposed to the `gemini_models()`
/// which use the AI Studio API.
pub fn vertex_gemini_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "gemini-3-pro-preview",
            display_name: "Gemini 3 Pro Preview",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-3-flash-preview",
            display_name: "Gemini 3 Flash Preview",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-pro",
            display_name: "Gemini 2.5 Pro",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-flash",
            display_name: "Gemini 2.5 Flash",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-flash-lite",
            display_name: "Gemini 2.5 Flash Lite",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.0-flash",
            display_name: "Gemini 2.0 Flash",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.0-flash-lite",
            display_name: "Gemini 2.0 Flash Lite",
            provider: AiProvider::VertexGemini,
            capabilities: ModelCapabilities::gemini_2_0_flash_lite_defaults(),
            aliases: &[],
        },
    ]
}
