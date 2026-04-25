//! `groq_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Groq model definitions.
pub fn groq_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "meta-llama/llama-4-scout-17b-16e-instruct",
            display_name: "Llama 4 Scout 17B",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &["llama-4-scout"],
        },
        ModelDefinition {
            id: "meta-llama/llama-4-maverick-17b-128e-instruct",
            display_name: "Llama 4 Maverick 17B",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &["llama-4-maverick"],
        },
        ModelDefinition {
            id: "llama-3.3-70b-versatile",
            display_name: "Llama 3.3 70B",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "llama-3.1-8b-instant",
            display_name: "Llama 3.1 8B Instant",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &[],
        },
        ModelDefinition {
            id: "openai/gpt-oss-120b",
            display_name: "GPT OSS 120B",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &["gpt-oss-120b"],
        },
        ModelDefinition {
            id: "openai/gpt-oss-20b",
            display_name: "GPT OSS 20B",
            provider: AiProvider::Groq,
            capabilities: ModelCapabilities::groq_defaults(),
            aliases: &["gpt-oss-20b"],
        },
    ]
}
