//! DeepSeek model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;

/// DeepSeek direct API model definitions.
pub fn deepseek_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "deepseek-v4-flash",
            display_name: "DeepSeek V4 Flash",
            provider: AiProvider::Deepseek,
            capabilities: ModelCapabilities::deepseek_defaults(),
            aliases: &["deepseek-chat", "deepseek-reasoner"],
        },
        ModelDefinition {
            id: "deepseek-v4-pro",
            display_name: "DeepSeek V4 Pro",
            provider: AiProvider::Deepseek,
            capabilities: ModelCapabilities::deepseek_defaults(),
            aliases: &[],
        },
    ]
}
