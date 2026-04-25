//! `zai_sdk_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// Z.AI SDK model definitions.
pub fn zai_sdk_models() -> Vec<ModelDefinition> {
    vec![
        ModelDefinition {
            id: "glm-5",
            display_name: "GLM 5",
            provider: AiProvider::ZaiSdk,
            capabilities: ModelCapabilities::zai_thinking_defaults(),
            aliases: &["GLM-5"],
        },
        ModelDefinition {
            id: "glm-4.7",
            display_name: "GLM 4.7",
            provider: AiProvider::ZaiSdk,
            capabilities: ModelCapabilities::zai_thinking_defaults(),
            aliases: &["GLM-4.7"],
        },
        ModelDefinition {
            id: "glm-4.6v",
            display_name: "GLM 4.6v",
            provider: AiProvider::ZaiSdk,
            capabilities: ModelCapabilities::zai_vision_defaults(),
            aliases: &["GLM-4.6v"],
        },
        ModelDefinition {
            id: "glm-4.5-air",
            display_name: "GLM 4.5 Air",
            provider: AiProvider::ZaiSdk,
            capabilities: ModelCapabilities::zai_defaults(),
            aliases: &["GLM-4.5-air"],
        },
        ModelDefinition {
            id: "glm-4-flash",
            display_name: "GLM 4 Flash",
            provider: AiProvider::ZaiSdk,
            capabilities: ModelCapabilities::zai_defaults(),
            aliases: &["GLM-4-flash"],
        },
    ]
}
