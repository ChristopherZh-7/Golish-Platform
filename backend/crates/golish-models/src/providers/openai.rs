//! `openai_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// OpenAI model definitions.
pub fn openai_models() -> Vec<ModelDefinition> {
    vec![
        // GPT-5 series (reasoning models) - 400k context, 128k output
        ModelDefinition {
            id: "gpt-5.4",
            display_name: "GPT 5.4",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5.2",
            display_name: "GPT 5.2",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5.1",
            display_name: "GPT 5.1",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5",
            display_name: "GPT 5",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5-mini",
            display_name: "GPT 5 Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5-nano",
            display_name: "GPT 5 Nano",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt5_defaults(),
            aliases: &[],
        },
        // GPT-4.1 series
        ModelDefinition {
            id: "gpt-4.1",
            display_name: "GPT 4.1",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-4.1-mini",
            display_name: "GPT 4.1 Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-4.1-nano",
            display_name: "GPT 4.1 Nano",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        // GPT-4o series
        ModelDefinition {
            id: "gpt-4o",
            display_name: "GPT 4o",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-4o-mini",
            display_name: "GPT 4o Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "chatgpt-4o-latest",
            display_name: "ChatGPT 4o Latest",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_gpt4_defaults(),
            aliases: &[],
        },
        // o-series reasoning models - 200k context, 100k output
        ModelDefinition {
            id: "o4-mini",
            display_name: "o4 Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_o_series_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "o3",
            display_name: "o3",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_o_series_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "o3-mini",
            display_name: "o3 Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_o_series_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "o1",
            display_name: "o1",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_o_series_defaults(),
            aliases: &["o1-preview"],
        },
        // Codex models
        ModelDefinition {
            id: "gpt-5.2-codex",
            display_name: "GPT 5.2 Codex",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_codex_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5.1-codex",
            display_name: "GPT 5.1 Codex",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_codex_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5.1-codex-max",
            display_name: "GPT 5.1 Codex Max",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_codex_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "gpt-5.1-codex-mini",
            display_name: "GPT 5.1 Codex Mini",
            provider: AiProvider::Openai,
            capabilities: ModelCapabilities::openai_codex_defaults(),
            aliases: &[],
        },
    ]
}
