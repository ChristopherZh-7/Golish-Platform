//! `nvidia_models` model definitions.

use golish_settings::schema::AiProvider;

use crate::capabilities::ModelCapabilities;
use crate::registry::ModelDefinition;


/// NVIDIA NIM model definitions.
pub fn nvidia_models() -> Vec<ModelDefinition> {
    vec![
        // NVIDIA Nemotron family
        ModelDefinition {
            id: "nvidia/nemotron-3-super-120b-a12b",
            display_name: "Nemotron 3 Super 120B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                context_window: 1_000_000,
                max_output_tokens: 8_192,
                ..ModelCapabilities::nvidia_large_defaults()
            },
            aliases: &["nemotron-120b", "nemotron-super"],
        },
        ModelDefinition {
            id: "nvidia/nemotron-3-nano-30b-a3b",
            display_name: "Nemotron 3 Nano 30B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                context_window: 1_000_000,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["nemotron-nano", "nemotron-30b"],
        },
        // Qwen family
        ModelDefinition {
            id: "qwen/qwen3-coder-480b-a35b-instruct",
            display_name: "Qwen3 Coder 480B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                context_window: 256_000,
                max_output_tokens: 8_192,
                ..ModelCapabilities::nvidia_large_defaults()
            },
            aliases: &["qwen3-coder"],
        },
        ModelDefinition {
            id: "qwen/qwen3.5-397b-a17b",
            display_name: "Qwen 3.5 397B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_vision: true,
                ..ModelCapabilities::nvidia_large_defaults()
            },
            aliases: &["qwen3.5-397b"],
        },
        ModelDefinition {
            id: "qwen/qwen3.5-122b-a10b",
            display_name: "Qwen 3.5 122B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["qwen3.5-122b"],
        },
        // Mistral family
        ModelDefinition {
            id: "mistralai/mistral-large-3-675b-instruct-2512",
            display_name: "Mistral Large 3 675B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_vision: true,
                ..ModelCapabilities::nvidia_large_defaults()
            },
            aliases: &["mistral-large-3"],
        },
        ModelDefinition {
            id: "mistralai/mistral-small-4-119b-2603",
            display_name: "Mistral Small 4 119B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_vision: true,
                context_window: 256_000,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["mistral-small-4"],
        },
        // DeepSeek family
        ModelDefinition {
            id: "deepseek-ai/deepseek-v3.2",
            display_name: "DeepSeek V3.2",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::nvidia_large_defaults()
            },
            aliases: &["deepseek-v3.2"],
        },
        // Moonshot Kimi family
        ModelDefinition {
            id: "moonshotai/kimi-k2-thinking",
            display_name: "Kimi K2 Thinking",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                context_window: 256_000,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["kimi-k2-thinking"],
        },
        // Other notable models
        ModelDefinition {
            id: "stepfun-ai/step-3.5-flash",
            display_name: "Step 3.5 Flash",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["step-3.5-flash"],
        },
        ModelDefinition {
            id: "minimaxai/minimax-m2.5",
            display_name: "MiniMax M2.5",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_large_defaults(),
            aliases: &["minimax-m2.5"],
        },
        ModelDefinition {
            id: "meta/llama-3.1-405b-instruct",
            display_name: "Llama 3.1 405B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_large_defaults(),
            aliases: &["llama-405b"],
        },
        // Small / fast models
        ModelDefinition {
            id: "nvidia/llama-3.3-nemotron-super-49b-v1.5",
            display_name: "Nemotron Super 49B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["nemotron-49b", "nemotron-super-49b"],
        },
        ModelDefinition {
            id: "nvidia/llama-3.1-nemotron-ultra-253b-v1",
            display_name: "Nemotron Ultra 253B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_large_defaults(),
            aliases: &["nemotron-ultra", "nemotron-253b"],
        },
        ModelDefinition {
            id: "nvidia/nvidia-nemotron-nano-9b-v2",
            display_name: "Nemotron Nano 9B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_small_defaults(),
            aliases: &["nemotron-9b"],
        },
        ModelDefinition {
            id: "nvidia/llama-3.1-nemotron-nano-4b-v1.1",
            display_name: "Nemotron Nano 4B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_small_defaults(),
            aliases: &["nemotron-4b"],
        },
        ModelDefinition {
            id: "qwen/qwen3-next-80b-a3b-instruct",
            display_name: "Qwen3 Next 80B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["qwen3-next-80b"],
        },
        ModelDefinition {
            id: "mistralai/mistral-small-3.1-24b-instruct-2503",
            display_name: "Mistral Small 3.1 24B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_vision: true,
                context_window: 128_000,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["mistral-small-3.1"],
        },
        ModelDefinition {
            id: "mistralai/mistral-nemotron",
            display_name: "Mistral Nemotron",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["mistral-nemotron"],
        },
        ModelDefinition {
            id: "mistralai/magistral-small-2506",
            display_name: "Magistral Small",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["magistral-small"],
        },
        ModelDefinition {
            id: "google/gemma-4-31b-it",
            display_name: "Gemma 4 31B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities::nvidia_defaults(),
            aliases: &["gemma-4-31b"],
        },
        ModelDefinition {
            id: "microsoft/phi-4-mini-flash-reasoning",
            display_name: "Phi-4 Mini Flash",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_thinking_history: true,
                ..ModelCapabilities::nvidia_small_defaults()
            },
            aliases: &["phi-4-mini"],
        },
        ModelDefinition {
            id: "meta/llama-4-maverick-17b-128e-instruct",
            display_name: "Llama 4 Maverick 17B",
            provider: AiProvider::Nvidia,
            capabilities: ModelCapabilities {
                supports_vision: true,
                ..ModelCapabilities::nvidia_defaults()
            },
            aliases: &["llama-4-maverick"],
        },
    ]
}
