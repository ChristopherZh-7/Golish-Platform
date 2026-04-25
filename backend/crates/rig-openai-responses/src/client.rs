//! Client and reasoning-effort wrappers around `async-openai`.
//!
//! `Client` is a thin wrapper that owns the `async-openai` client and
//! constructs `CompletionModel` instances.  `ReasoningEffort` is our
//! local enum mirroring async-openai's `ReasoningEffort`, so callers
//! don't have to depend on async-openai's types directly.

use async_openai::config::OpenAIConfig;
use async_openai::types::responses::ReasoningEffort as OAReasoningEffort;
use async_openai::Client as OpenAIClient;

use crate::completion::CompletionModel;

/// Wrapper around async-openai client for creating completion models.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: OpenAIClient<OpenAIConfig>,
}

impl Client {
    /// Create a new client with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            inner: OpenAIClient::with_config(config),
        }
    }

    /// Create a new client with a custom base URL (e.g., for Azure OpenAI).
    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);
        Self {
            inner: OpenAIClient::with_config(config),
        }
    }

    /// Create a completion model for the given model name.
    pub fn completion_model(&self, model: impl Into<String>) -> CompletionModel {
        CompletionModel::new(self.clone(), model.into())
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

/// Reasoning effort level for OpenAI reasoning models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReasoningEffort {
    /// Low reasoning effort - faster but less thorough.
    Low,
    /// Medium reasoning effort - balanced.
    #[default]
    Medium,
    /// High reasoning effort - slower but more thorough.
    High,
    /// Extra high reasoning effort - maximum thoroughness (maps to OpenAI's `xhigh`).
    ExtraHigh,
}

impl From<ReasoningEffort> for OAReasoningEffort {
    fn from(effort: ReasoningEffort) -> Self {
        match effort {
            ReasoningEffort::Low => OAReasoningEffort::Low,
            ReasoningEffort::Medium => OAReasoningEffort::Medium,
            ReasoningEffort::High => OAReasoningEffort::High,
            ReasoningEffort::ExtraHigh => OAReasoningEffort::Xhigh,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_effort_conversion() {
        assert!(matches!(
            OAReasoningEffort::from(ReasoningEffort::Low),
            OAReasoningEffort::Low
        ));
        assert!(matches!(
            OAReasoningEffort::from(ReasoningEffort::Medium),
            OAReasoningEffort::Medium
        ));
        assert!(matches!(
            OAReasoningEffort::from(ReasoningEffort::High),
            OAReasoningEffort::High
        ));
        assert!(matches!(
            OAReasoningEffort::from(ReasoningEffort::ExtraHigh),
            OAReasoningEffort::Xhigh
        ));
    }
}
