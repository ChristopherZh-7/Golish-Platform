//! `vertex_ai_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Vertex AI (Anthropic Claude) model definitions.
pub fn vertex_ai_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "claude-opus-4-6@default",
            display_name: "Claude Opus 4.6",
            provider: AiProvider::VertexAi,
            capabilities: ModelCapabilities::anthropic_opus_4_6(),
            aliases: &[],
        },
        ModelDefinition {
            id: "claude-sonnet-4-6@default",
            display_name: "Claude Sonnet 4.6",
            provider: AiProvider::VertexAi,
            capabilities: ModelCapabilities::anthropic_sonnet_4_6(),
            aliases: &[],
        },
        ModelDefinition {
            id: "claude-opus-4-5@20251101",
            display_name: "Claude Opus 4.5",
            provider: AiProvider::VertexAi,
            capabilities: ModelCapabilities::anthropic_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "claude-sonnet-4-5@20250929",
            display_name: "Claude Sonnet 4.5",
            provider: AiProvider::VertexAi,
            capabilities: ModelCapabilities::anthropic_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "claude-haiku-4-5@20251001",
            display_name: "Claude Haiku 4.5",
            provider: AiProvider::VertexAi,
            capabilities: ModelCapabilities {
                max_output_tokens: 4_096, // Haiku has smaller output
                ..ModelCapabilities::anthropic_defaults()
            },
            aliases: &[],
        },
    ]
}
