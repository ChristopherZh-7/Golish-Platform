use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

// =============================================================================
// Enums for type-safe settings
// =============================================================================

/// AI provider selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "generated/")]
pub enum AiProvider {
    #[default]
    VertexAi,
    /// Google Gemini on Vertex AI (native Gemini models)
    VertexGemini,
    Openrouter,
    Anthropic,
    Openai,
    Ollama,
    Gemini,
    Groq,
    Xai,
    /// Z.AI via native SDK implementation
    ZaiSdk,
    /// NVIDIA NIM (OpenAI-compatible API)
    Nvidia,
}

impl fmt::Display for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AiProvider::VertexAi => "vertex_ai",
            AiProvider::VertexGemini => "vertex_gemini",
            AiProvider::Openrouter => "openrouter",
            AiProvider::Anthropic => "anthropic",
            AiProvider::Openai => "openai",
            AiProvider::Ollama => "ollama",
            AiProvider::Gemini => "gemini",
            AiProvider::Groq => "groq",
            AiProvider::Xai => "xai",
            AiProvider::ZaiSdk => "zai_sdk",
            AiProvider::Nvidia => "nvidia",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for AiProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "vertex_ai" | "vertex" => Ok(AiProvider::VertexAi),
            "vertex_gemini" => Ok(AiProvider::VertexGemini),
            "openrouter" => Ok(AiProvider::Openrouter),
            "anthropic" => Ok(AiProvider::Anthropic),
            "openai" => Ok(AiProvider::Openai),
            "ollama" => Ok(AiProvider::Ollama),
            "gemini" => Ok(AiProvider::Gemini),
            "groq" => Ok(AiProvider::Groq),
            "xai" => Ok(AiProvider::Xai),
            "z_ai_sdk" | "zai_sdk" | "zai" | "z_ai" | "zhipu" => Ok(AiProvider::ZaiSdk),
            "nvidia" | "nvidia_nim" | "nim" => Ok(AiProvider::Nvidia),
            _ => Err(format!("Invalid AI provider: {}", s)),
        }
    }
}

/// UI theme selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::System => "system",
        };
        write!(f, "{}", s)
    }
}

/// Logging level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

/// Index storage location configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndexLocation {
    /// Store indexes globally in ~/.golish/<codebase-name>/index (new default)
    #[default]
    Global,
    /// Store indexes locally in <workspace>/.golish/index (legacy behavior)
    Local,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        };
        write!(f, "{}", s)
    }
}

/// Reasoning effort level for models that support it (e.g., OpenAI o-series, GPT-5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    ExtraHigh,
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::ExtraHigh => "extra_high",
        };
        write!(f, "{}", s)
    }
}
