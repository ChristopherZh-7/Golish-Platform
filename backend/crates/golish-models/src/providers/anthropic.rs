//! `anthropic_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Direct Anthropic API model definitions.
pub fn anthropic_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "claude-sonnet-4-6-20260217",
            display_name: "Claude Sonnet 4.6",
            provider: AiProvider::Anthropic,
            capabilities: ModelCapabilities::anthropic_sonnet_4_6(),
            aliases: &["claude-sonnet-4-6"],
        },
        ModelDefinition {
            id: "claude-opus-4-5-20251101",
            display_name: "Claude Opus 4.5",
            provider: AiProvider::Anthropic,
            capabilities: ModelCapabilities::anthropic_defaults(),
            aliases: &["claude-opus-4-5"],
        },
        ModelDefinition {
            id: "claude-sonnet-4-5-20250929",
            display_name: "Claude Sonnet 4.5",
            provider: AiProvider::Anthropic,
            capabilities: ModelCapabilities::anthropic_defaults(),
            aliases: &["claude-sonnet-4-5"],
        },
        ModelDefinition {
            id: "claude-haiku-4-5-20251001",
            display_name: "Claude Haiku 4.5",
            provider: AiProvider::Anthropic,
            capabilities: ModelCapabilities {
                max_output_tokens: 4_096,
                ..ModelCapabilities::anthropic_defaults()
            },
            aliases: &["claude-haiku-4-5"],
        },
    ]
}
