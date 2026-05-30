//! LLM provider abstraction for Golish.
//!
//! This crate provides a unified interface for interacting with different LLM providers:
//! - OpenRouter via rig-core (supports tools and system prompts)
//! - Anthropic on Vertex AI via rig-anthropic-vertex
//! - OpenAI via rig-core
//! - Ollama local inference via rig-core
//! - Gemini via rig-core
//! - Groq via rig-core
//! - xAI (Grok) via rig-core
//! - Direct Anthropic API via rig-core
//! - Z.AI (GLM models) via rig-zai-sdk (native SDK implementation)
//! - DeepSeek via OpenAI-compatible API
//!
//! # Architecture
//!
//! This is a **Layer 2 (Infrastructure)** crate:
//! - Depends on: rig-core, rig-anthropic-vertex
//! - Used by: golish-ai (agent orchestration)

pub mod deepseek;
mod model_capabilities;
mod openai_config;
mod provider_config;
mod provider_trait;
mod reasoning_models;
pub mod xiaomi;

pub use deepseek::*;
pub use model_capabilities::*;
pub use openai_config::*;
pub use provider_config::*;
pub use provider_trait::*;
pub use reasoning_models::*;
pub use xiaomi::*;

use rig::providers::anthropic as rig_anthropic;
use rig::providers::gemini as rig_gemini;
use rig::providers::groq as rig_groq;
use rig::providers::ollama as rig_ollama;
use rig::providers::openai as rig_openai;
use rig::providers::openrouter as rig_openrouter;
use rig::providers::xai as rig_xai;

/// Convert settings-level [`OpenRouterProviderPreferences`](golish_settings::OpenRouterProviderPreferences)
/// into the JSON value expected by the OpenRouter API, using rig-core's native
/// [`ProviderPreferences`](rig_openrouter::ProviderPreferences) types for type-safe serialization.
///
/// The settings struct stores values as flat strings/numbers for TOML ergonomics.
/// This function maps those values into rig's typed enums (`DataCollection`,
/// `ProviderSortStrategy`, `Quantization`, `MaxPrice`, etc.) and delegates
/// JSON serialization to `ProviderPreferences::to_json()`.
pub fn openrouter_preferences_to_json(
    prefs: &golish_settings::schema::OpenRouterProviderPreferences,
) -> serde_json::Value {
    use rig_openrouter::{
        DataCollection, LatencyThreshold, MaxPrice, ProviderPreferences, ProviderSortStrategy,
        Quantization, ThroughputThreshold,
    };

    let mut rig_prefs = ProviderPreferences::new();

    if let Some(ref order) = prefs.order {
        rig_prefs = rig_prefs.order(order.iter().cloned());
    }
    if let Some(ref only) = prefs.only {
        rig_prefs = rig_prefs.only(only.iter().cloned());
    }
    if let Some(ref ignore) = prefs.ignore {
        rig_prefs = rig_prefs.ignore(ignore.iter().cloned());
    }
    if let Some(allow_fallbacks) = prefs.allow_fallbacks {
        rig_prefs = rig_prefs.allow_fallbacks(allow_fallbacks);
    }
    if let Some(require_parameters) = prefs.require_parameters {
        rig_prefs = rig_prefs.require_parameters(require_parameters);
    }
    if let Some(ref data_collection) = prefs.data_collection {
        let dc = match data_collection.to_lowercase().as_str() {
            "deny" => DataCollection::Deny,
            _ => DataCollection::Allow,
        };
        rig_prefs = rig_prefs.data_collection(dc);
    }
    if let Some(zdr) = prefs.zdr {
        rig_prefs = rig_prefs.zdr(zdr);
    }
    if let Some(ref sort) = prefs.sort {
        if let Some(strategy) = match sort.to_lowercase().as_str() {
            "price" => Some(ProviderSortStrategy::Price),
            "throughput" => Some(ProviderSortStrategy::Throughput),
            "latency" => Some(ProviderSortStrategy::Latency),
            _ => None,
        } {
            rig_prefs = rig_prefs.sort(strategy);
        }
    }
    if let Some(throughput) = prefs.preferred_min_throughput {
        rig_prefs = rig_prefs.preferred_min_throughput(ThroughputThreshold::Simple(throughput));
    }
    if let Some(latency) = prefs.preferred_max_latency {
        rig_prefs = rig_prefs.preferred_max_latency(LatencyThreshold::Simple(latency));
    }

    if prefs.max_price_prompt.is_some() || prefs.max_price_completion.is_some() {
        let mut max_price = MaxPrice::new();
        if let Some(prompt) = prefs.max_price_prompt {
            max_price = max_price.prompt(prompt);
        }
        if let Some(completion) = prefs.max_price_completion {
            max_price = max_price.completion(completion);
        }
        rig_prefs = rig_prefs.max_price(max_price);
    }

    if let Some(ref quantizations) = prefs.quantizations {
        let quants: Vec<Quantization> = quantizations
            .iter()
            .filter_map(|q: &String| match q.to_lowercase().as_str() {
                "int4" => Some(Quantization::Int4),
                "int8" => Some(Quantization::Int8),
                "fp8" => Some(Quantization::Fp8),
                "fp16" => Some(Quantization::Fp16),
                "bf16" => Some(Quantization::Bf16),
                "fp32" => Some(Quantization::Fp32),
                "unknown" => Some(Quantization::Unknown),
                _ => None,
            })
            .collect();
        if !quants.is_empty() {
            rig_prefs = rig_prefs.quantizations(quants);
        }
    }

    rig_prefs.to_json()
}

// Re-export for external use
pub use rig_gemini_vertex;
pub use rig_openai_responses;
pub use rig_zai_sdk;

/// LLM client abstraction for different providers
#[derive(Clone)]
pub enum LlmClient {
    /// Anthropic on Vertex AI via rig-anthropic-vertex
    VertexAnthropic(rig_anthropic_vertex::CompletionModel),
    /// Gemini on Vertex AI via rig-gemini-vertex
    VertexGemini(rig_gemini_vertex::CompletionModel),
    /// OpenRouter via rig-core (supports tools and system prompts)
    RigOpenRouter(rig_openrouter::CompletionModel),
    /// OpenAI via rig-core (uses Chat Completions API - may have tool issues)
    RigOpenAi(rig_openai::completion::CompletionModel),
    /// OpenAI via rig-core (uses Responses API - better tool support)
    RigOpenAiResponses(rig_openai::responses_api::ResponsesCompletionModel),
    /// OpenAI reasoning models via custom provider with explicit streaming event separation.
    /// Used for o1, o3, o4, gpt-5.x models where reasoning deltas must be kept separate from text.
    OpenAiReasoning(rig_openai_responses::CompletionModel),
    /// Direct Anthropic API via rig-core
    RigAnthropic(rig_anthropic::completion::CompletionModel),
    /// Ollama local inference via rig-core
    RigOllama(rig_ollama::CompletionModel<reqwest::Client>),
    /// Gemini via rig-core
    RigGemini(rig_gemini::completion::CompletionModel),
    /// Groq via rig-core
    RigGroq(rig_groq::CompletionModel<reqwest::Client>),
    /// xAI (Grok) via rig-core
    RigXai(rig_xai::completion::CompletionModel<reqwest::Client>),
    /// Z.AI via native SDK implementation
    RigZaiSdk(rig_zai_sdk::CompletionModel),
    /// NVIDIA NIM via OpenAI-compatible Chat Completions API
    RigNvidia(rig_openai::completion::CompletionModel),
    /// DeepSeek via OpenAI-compatible Chat Completions API
    RigDeepSeek(rig_openai::completion::CompletionModel),
    /// Xiaomi MiMo via OpenAI-compatible Chat Completions API
    RigXiaomi(rig_openai::completion::CompletionModel),
    /// Xiaomi MiMo via Anthropic-compatible Messages API
    RigXiaomiAnthropic(rig_anthropic::completion::CompletionModel),
    /// Mock client for testing (doesn't require credentials)
    /// This variant is always available for integration testing across crates.
    Mock,
}

/// Dispatch a closure-like body over all [`LlmClient`] variants uniformly.
///
/// Every variant except `Mock` binds the inner model to `$model` and evaluates `$body`.
/// The `Mock` arm evaluates `$mock_body` instead.
///
/// Because only one match arm executes, captured variables are moved exactly once.
///
/// ```ignore
/// dispatch_llm_client!(&*client, |m| {
///     m.completion(request).await
/// }, mock => Err(anyhow!("no mock")));
/// ```
#[macro_export]
macro_rules! dispatch_llm_client {
    ($client:expr, |$model:ident| $body:expr, mock => $mock_body:expr) => {
        match $client {
            $crate::LlmClient::VertexAnthropic($model) => $body,
            $crate::LlmClient::VertexGemini($model) => $body,
            $crate::LlmClient::RigOpenRouter($model) => $body,
            $crate::LlmClient::RigOpenAi($model) => $body,
            $crate::LlmClient::RigOpenAiResponses($model) => $body,
            $crate::LlmClient::OpenAiReasoning($model) => $body,
            $crate::LlmClient::RigAnthropic($model) => $body,
            $crate::LlmClient::RigOllama($model) => $body,
            $crate::LlmClient::RigGemini($model) => $body,
            $crate::LlmClient::RigGroq($model) => $body,
            $crate::LlmClient::RigXai($model) => $body,
            $crate::LlmClient::RigZaiSdk($model) => $body,
            $crate::LlmClient::RigNvidia($model) => $body,
            $crate::LlmClient::RigDeepSeek($model) => $body,
            $crate::LlmClient::RigXiaomi($model) => $body,
            $crate::LlmClient::RigXiaomiAnthropic($model) => $body,
            $crate::LlmClient::Mock => $mock_body,
        }
    };
}

/// Like [`dispatch_llm_client!`] but splits Vertex Anthropic (extended thinking)
/// from the generic path.
///
/// Use when Vertex Anthropic requires special handling (e.g. the thinking-enabled
/// agentic loop) while all other providers share the same body.
///
/// ```ignore
/// dispatch_llm_client_split!(&*client,
///     vertex_anthropic(va) => { self.run_thinking_turn(&va, ...).await },
///     generic(m)           => { self.run_generic_turn(&m, ...).await },
///     mock                 => Err(anyhow!("mock")),
/// );
/// ```
#[macro_export]
macro_rules! dispatch_llm_client_split {
    ($client:expr,
     vertex_anthropic($va:ident) => $va_body:expr,
     generic($g:ident) => $g_body:expr,
     mock => $mock_body:expr $(,)?
    ) => {
        match $client {
            $crate::LlmClient::VertexAnthropic($va) => $va_body,
            $crate::LlmClient::VertexGemini($g) => $g_body,
            $crate::LlmClient::RigOpenRouter($g) => $g_body,
            $crate::LlmClient::RigOpenAi($g) => $g_body,
            $crate::LlmClient::RigOpenAiResponses($g) => $g_body,
            $crate::LlmClient::OpenAiReasoning($g) => $g_body,
            $crate::LlmClient::RigAnthropic($g) => $g_body,
            $crate::LlmClient::RigOllama($g) => $g_body,
            $crate::LlmClient::RigGemini($g) => $g_body,
            $crate::LlmClient::RigGroq($g) => $g_body,
            $crate::LlmClient::RigXai($g) => $g_body,
            $crate::LlmClient::RigZaiSdk($g) => $g_body,
            $crate::LlmClient::RigNvidia($g) => $g_body,
            $crate::LlmClient::RigDeepSeek($g) => $g_body,
            $crate::LlmClient::RigXiaomi($g) => $g_body,
            $crate::LlmClient::RigXiaomiAnthropic($g) => $g_body,
            $crate::LlmClient::Mock => $mock_body,
        }
    };
}

impl LlmClient {
    /// Execute a one-shot (non-streaming) completion request.
    ///
    /// Dispatches to the correct provider variant internally, extracting text
    /// from the response. This eliminates the need for callers to match on
    /// all 14 `LlmClient` variants.
    ///
    /// Automatically handles the NVIDIA NIM workaround (system prompt serialized
    /// as a leading user message instead of `preamble`).
    pub async fn one_shot_completion(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> anyhow::Result<String> {
        use rig::completion::{AssistantContent, CompletionModel as _, CompletionRequest, Message};
        use rig::message::{Text as RigText, UserContent};
        use rig::one_or_many::OneOrMany;

        let is_nvidia = matches!(self, Self::RigNvidia(_));

        let user_msg = Message::User {
            content: OneOrMany::one(UserContent::Text(RigText {
                text: user_message.to_string(),
            })),
        };

        let (preamble, chat_history) = if is_nvidia {
            let nvidia_history = vec![
                Message::User {
                    content: OneOrMany::one(UserContent::text(system_prompt)),
                },
                user_msg,
            ];
            (
                None,
                OneOrMany::many(nvidia_history).expect("nvidia_history always has 2 elements"),
            )
        } else {
            (Some(system_prompt.to_string()), OneOrMany::one(user_msg))
        };

        let request = CompletionRequest {
            preamble,
            chat_history,
            documents: vec![],
            tools: vec![],
            temperature,
            max_tokens,
            tool_choice: None,
            additional_params: None,
            model: None,
            output_schema: None,
        };

        dispatch_llm_client!(self, |m| {
            let response = m
                .completion(request)
                .await
                .map_err(|e| anyhow::anyhow!("LLM completion failed: {e}"))?;
            let text = response
                .choice
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            Ok(text)
        }, mock => Err(anyhow::anyhow!("Mock client cannot execute completions")))
    }

    /// Returns `true` if this variant supports Anthropic extended thinking.
    ///
    /// Only `VertexAnthropic` supports the thinking-enabled agentic loop.
    pub fn supports_thinking(&self) -> bool {
        matches!(self, Self::VertexAnthropic(_))
    }
    /// Check if this client uses an Anthropic model (Vertex AI, direct API, or Xiaomi Anthropic-compatible).
    ///
    /// Returns true for providers that support Anthropic-specific features
    /// like extended thinking and native web tools.
    pub fn is_anthropic(&self) -> bool {
        matches!(
            self,
            LlmClient::VertexAnthropic(_)
                | LlmClient::RigAnthropic(_)
                | LlmClient::RigXiaomiAnthropic(_)
        )
    }

    /// Check if this client supports Claude's native web tools.
    ///
    /// Native web tools (web_search_20250305, web_fetch_20250910) are server-side
    /// tools that Claude executes automatically. They're only supported on
    /// Vertex AI Anthropic for now (direct Anthropic API support may come later).
    pub fn supports_native_web_tools(&self) -> bool {
        match self {
            LlmClient::VertexAnthropic(_) => true,
            LlmClient::Mock => false,
            _ => false,
        }
    }

    /// Get the provider name for logging and debugging.
    pub fn provider_name(&self) -> &'static str {
        match self {
            LlmClient::VertexAnthropic(_) => "vertex_ai_anthropic",
            LlmClient::VertexGemini(_) => "vertex_ai_gemini",
            LlmClient::RigOpenRouter(_) => "openrouter",
            LlmClient::RigOpenAi(_) => "openai",
            LlmClient::RigOpenAiResponses(_) => "openai_responses",
            LlmClient::OpenAiReasoning(_) => "openai_reasoning",
            LlmClient::RigAnthropic(_) => "anthropic",
            LlmClient::RigOllama(_) => "ollama",
            LlmClient::RigGemini(_) => "gemini",
            LlmClient::RigGroq(_) => "groq",
            LlmClient::RigXai(_) => "xai",
            LlmClient::RigZaiSdk(_) => "zai_sdk",
            LlmClient::RigNvidia(_) => "nvidia",
            LlmClient::RigDeepSeek(_) => "deepseek",
            LlmClient::RigXiaomi(_) => "xiaomi",
            LlmClient::RigXiaomiAnthropic(_) => "xiaomi_anthropic",
            LlmClient::Mock => "mock",
        }
    }

    /// Check if this client uses a Gemini model on Vertex AI.
    pub fn is_vertex_gemini(&self) -> bool {
        matches!(self, LlmClient::VertexGemini(_))
    }

    /// Check if this client is an OpenAI provider.
    ///
    /// Returns true for Chat Completions API, Responses API, and reasoning model variants.
    /// `RigDeepSeek` and `RigXiaomi` count as OpenAI-compatible because they speak
    /// the same wire protocol; OpenAI server-side features (e.g. native web search)
    /// are gated separately by [`Self::supports_openai_web_search`].
    pub fn is_openai(&self) -> bool {
        matches!(
            self,
            LlmClient::RigOpenAi(_)
                | LlmClient::RigOpenAiResponses(_)
                | LlmClient::OpenAiReasoning(_)
                | LlmClient::RigDeepSeek(_)
                | LlmClient::RigXiaomi(_)
        )
    }

    /// Check if this client supports OpenAI's native web search tool.
    ///
    /// The web_search_preview tool is a server-side tool that OpenAI
    /// executes during inference, similar to Claude's native web tools.
    pub fn supports_openai_web_search(&self) -> bool {
        matches!(
            self,
            LlmClient::RigOpenAi(_)
                | LlmClient::RigOpenAiResponses(_)
                | LlmClient::OpenAiReasoning(_)
        )
    }

    /// Check if this client uses an OpenAI reasoning model (o1, o3, gpt-5.x).
    ///
    /// These models have explicit reasoning events that must be handled separately.
    pub fn is_reasoning_model(&self) -> bool {
        matches!(self, LlmClient::OpenAiReasoning(_))
    }
}

// Provider config structs and ProviderConfig enum are in provider_config.rs

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
