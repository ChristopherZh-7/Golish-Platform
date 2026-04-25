//! `gemini_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Gemini model definitions.
pub fn gemini_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "gemini-3-pro-preview",
            display_name: "Gemini 3 Pro Preview",
            provider: AiProvider::Gemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-pro",
            display_name: "Gemini 2.5 Pro",
            provider: AiProvider::Gemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-flash",
            display_name: "Gemini 2.5 Flash",
            provider: AiProvider::Gemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.5-flash-lite",
            display_name: "Gemini 2.5 Flash Lite",
            provider: AiProvider::Gemini,
            capabilities: ModelCapabilities::gemini_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gemini-2.0-flash-thinking-exp",
            display_name: "Gemini 2.0 Flash Thinking",
            provider: AiProvider::Gemini,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::gemini_defaults()
            },
            aliases: &[],
        },
    ]
}
