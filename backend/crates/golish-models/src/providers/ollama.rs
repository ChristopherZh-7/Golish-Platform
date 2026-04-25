//! `ollama_default_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Ollama default model definitions.
///
/// Note: Ollama models vary by installation. These are common defaults.
/// Use `discover_ollama_models()` in the registry module for dynamic discovery.
pub fn ollama_default_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "llama3.2",
            display_name: "Llama 3.2",
            provider: AiProvider::Ollama,
            capabilities: ModelCapabilities::ollama_defaults(),
            aliases: &["llama3.2:latest"],
        },
        ModelDefinition {
            id: "llama3.1",
            display_name: "Llama 3.1",
            provider: AiProvider::Ollama,
            capabilities: ModelCapabilities::ollama_defaults(),
            aliases: &["llama3.1:latest"],
        },
        ModelDefinition {
            id: "qwen2.5",
            display_name: "Qwen 2.5",
            provider: AiProvider::Ollama,
            capabilities: ModelCapabilities::ollama_defaults(),
            aliases: &["qwen2.5:latest"],
        },
        ModelDefinition {
            id: "mistral",
            display_name: "Mistral",
            provider: AiProvider::Ollama,
            capabilities: ModelCapabilities::ollama_defaults(),
            aliases: &["mistral:latest"],
        },
        ModelDefinition {
            id: "codellama",
            display_name: "CodeLlama",
            provider: AiProvider::Ollama,
            capabilities: ModelCapabilities::ollama_defaults(),
            aliases: &["codellama:latest"],
        },
    ]
}
