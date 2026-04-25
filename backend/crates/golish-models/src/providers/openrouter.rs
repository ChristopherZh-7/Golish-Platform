//! `openrouter_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// OpenRouter model definitions.
///
/// Note: OpenRouter provides access to many models. These are curated defaults.
pub fn openrouter_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "mistralai/devstral-2512",
            display_name: "Devstral 2512",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "deepseek/deepseek-v3.2",
            display_name: "Deepseek v3.2",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "z-ai/glm-4.6",
            display_name: "GLM 4.6",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "x-ai/grok-code-fast-1",
            display_name: "Grok Code Fast 1",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "openai/gpt-oss-20b",
            display_name: "GPT OSS 20B",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "openai/gpt-oss-120b",
            display_name: "GPT OSS 120B",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::conservative_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "openai/gpt-5.2",
            display_name: "GPT 5.2",
            provider: AiProvider::Openrouter,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
    ]
}
