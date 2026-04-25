//! Per-provider model catalogs and provider metadata helpers.

use golish_settings::schema::AiProvider;
use serde::{Deserialize, Serialize};

mod anthropic;
mod gemini;
mod groq;
mod nvidia;
mod ollama;
mod openai;
mod openrouter;
mod vertex_ai;
mod vertex_gemini;
mod xai;
mod zai_sdk;

pub use anthropic::anthropic_models;
pub use gemini::gemini_models;
pub use groq::groq_models;
pub use nvidia::nvidia_models;
pub use ollama::ollama_default_models;
pub use openai::openai_models;
pub use openrouter::openrouter_models;
pub use vertex_ai::vertex_ai_models;
pub use vertex_gemini::vertex_gemini_models;
pub use xai::xai_models;
pub use zai_sdk::zai_sdk_models;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub provider: AiProvider,
    pub name: &'static str,
    pub icon: &'static str,
    pub description: &'static str,
}

/// Get provider metadata.
pub fn get_provider_info(provider: AiProvider) -> ProviderInfo {
    match provider {
        AiProvider::VertexAi => ProviderInfo {
            provider,
            name: "Vertex AI",
            icon: "🔷",
            description: "Claude on Google Cloud",
        },
        AiProvider::Anthropic => ProviderInfo {
            provider,
            name: "Anthropic",
            icon: "🔶",
            description: "Direct Claude API access",
        },
        AiProvider::Openai => ProviderInfo {
            provider,
            name: "OpenAI",
            icon: "⚪",
            description: "GPT and o-series models",
        },
        AiProvider::Gemini => ProviderInfo {
            provider,
            name: "Gemini",
            icon: "💎",
            description: "Google Gemini models",
        },
        AiProvider::Groq => ProviderInfo {
            provider,
            name: "Groq",
            icon: "⚡",
            description: "Fast inference for open models",
        },
        AiProvider::Xai => ProviderInfo {
            provider,
            name: "xAI",
            icon: "𝕏",
            description: "Grok models",
        },
        AiProvider::ZaiSdk => ProviderInfo {
            provider,
            name: "Z.AI SDK",
            icon: "🤖",
            description: "Z.AI GLM models",
        },
        AiProvider::Ollama => ProviderInfo {
            provider,
            name: "Ollama",
            icon: "🦙",
            description: "Local inference",
        },
        AiProvider::Openrouter => ProviderInfo {
            provider,
            name: "OpenRouter",
            icon: "🔀",
            description: "Access multiple providers",
        },
        AiProvider::VertexGemini => ProviderInfo {
            provider,
            name: "Vertex Gemini",
            icon: "🔷",
            description: "Gemini on Google Cloud",
        },
        AiProvider::Nvidia => ProviderInfo {
            provider,
            name: "NVIDIA NIM",
            icon: "🟢",
            description: "NVIDIA NIM inference",
        },
    }
}

/// Get all provider metadata.
pub fn get_all_provider_info() -> Vec<ProviderInfo> {
    vec![
        get_provider_info(AiProvider::VertexAi),
        get_provider_info(AiProvider::VertexGemini),
        get_provider_info(AiProvider::Anthropic),
        get_provider_info(AiProvider::Openai),
        get_provider_info(AiProvider::Gemini),
        get_provider_info(AiProvider::Groq),
        get_provider_info(AiProvider::Xai),
        get_provider_info(AiProvider::ZaiSdk),
        get_provider_info(AiProvider::Ollama),
        get_provider_info(AiProvider::Openrouter),
        get_provider_info(AiProvider::Nvidia),
    ]
}
