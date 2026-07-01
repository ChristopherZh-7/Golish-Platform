//! `LlmOneShot` — inbound port giving bridge tools a single non-streaming LLM
//! completion without coupling `golish-app-core` / `golish-pentest-app` to a
//! concrete LLM provider crate.
//!
//! The implementation side (`golish-agent-app`) wraps the configured DeepSeek
//! client (`golish_llm_providers::create_client_for_model` +
//! `LlmClient::one_shot_completion`). Tools hold `Option<Arc<dyn LlmOneShot>>`
//! and degrade to deterministic behaviour when it is `None` or unavailable.

use anyhow::Result;

/// A single bounded LLM text completion. Implementations must be cheap to share
/// (`Arc`) and safe to call from any tool's `execute`.
#[async_trait::async_trait]
pub trait LlmOneShot: Send + Sync {
    /// Run one completion. `temperature` / `max_tokens` use the implementation's
    /// defaults when `None`. Returns the model's raw text; the caller parses it
    /// (e.g. extracts a JSON object).
    async fn complete(
        &self,
        system_prompt: &str,
        user_message: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String>;

    /// Whether a usable provider/credential is configured. Callers use this to
    /// skip the LLM path and fall back to deterministic output.
    async fn is_available(&self) -> bool;
}
