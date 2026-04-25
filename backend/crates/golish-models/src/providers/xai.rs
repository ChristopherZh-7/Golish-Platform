//! `xai_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// xAI (Grok) model definitions.
pub fn xai_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "grok-4-1-fast-reasoning",
            display_name: "Grok 4.1 Fast (Reasoning)",
            provider: AiProvider::Xai,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::xai_defaults()
            },
            aliases: &[],
        },
        ModelDefinition {
            id: "grok-4-1-fast-non-reasoning",
            display_name: "Grok 4.1 Fast",
            provider: AiProvider::Xai,
            capabilities: ModelCapabilities::xai_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "grok-4-fast-reasoning",
            display_name: "Grok 4 (Reasoning)",
            provider: AiProvider::Xai,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::xai_defaults()
            },
            aliases: &[],
        },
        ModelDefinition {
            id: "grok-4-fast-non-reasoning",
            display_name: "Grok 4",
            provider: AiProvider::Xai,
            capabilities: ModelCapabilities::xai_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "grok-code-fast-1",
            display_name: "Grok Code",
            provider: AiProvider::Xai,
            capabilities: ModelCapabilities::xai_defaults(),
            aliases: &[],
        },
    ]
}
