//! `CompletionModel` for Gemini on Vertex AI.
//!
//! `mod.rs` owns the public [`CompletionModel`] struct and its constructor /
//! builder methods, plus the [`StreamingCompletionResponseData`] used by the
//! streaming impl. Heavy lifting lives in sibling modules:
//!
//! - [`convert`] — pure rig ↔ Gemini-types conversions
//! - [`model_impl`] — the `rig::completion::CompletionModel` trait impl
//!   (HTTP calls, SSE → rig stream translation)

use rig::completion::{GetTokenUsage, Usage};
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::types::{self, ThinkingConfig};

mod convert;
mod model_impl;

/// Completion model for Gemini on Vertex AI.
#[derive(Clone)]
pub struct CompletionModel {
    client: Client,
    model: String,
    /// Optional thinking configuration for reasoning models.
    thinking: Option<ThinkingConfig>,
}

impl CompletionModel {
    /// Create a new completion model.
    pub fn new(client: Client, model: String) -> Self {
        Self {
            client,
            model,
            thinking: None,
        }
    }

    /// Enable thinking mode with the specified token budget.
    pub fn with_thinking_budget(mut self, budget: i32) -> Self {
        self.thinking = Some(ThinkingConfig::with_budget(budget));
        self
    }

    /// Enable thinking mode with the specified level (`"LOW"` or `"HIGH"`).
    pub fn with_thinking_level(mut self, level: impl Into<String>) -> Self {
        self.thinking = Some(ThinkingConfig::with_level(level));
        self
    }

    /// Enable including thoughts in the response.
    pub fn with_include_thoughts(mut self, include: bool) -> Self {
        if let Some(ref mut thinking) = self.thinking {
            thinking.include_thoughts = Some(include);
        } else {
            self.thinking = Some(ThinkingConfig {
                thinking_budget: None,
                thinking_level: None,
                include_thoughts: Some(include),
            });
        }
        self
    }

    /// Get the model identifier.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Internal: shared by sibling modules to access the underlying client.
    pub(super) fn client(&self) -> &Client {
        &self.client
    }

    /// Internal: shared by sibling modules to read the thinking configuration.
    pub(super) fn thinking(&self) -> &Option<ThinkingConfig> {
        &self.thinking
    }
}

impl std::fmt::Debug for CompletionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionModel")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

/// Response type for streaming.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamingCompletionResponseData {
    /// Accumulated text.
    pub text: String,
    /// Token usage (filled at end).
    pub usage: Option<types::UsageMetadata>,
}

impl GetTokenUsage for StreamingCompletionResponseData {
    fn token_usage(&self) -> Option<Usage> {
        self.usage.as_ref().map(|u| Usage {
            input_tokens: u.prompt_token_count as u64,
            output_tokens: u.candidates_token_count as u64,
            total_tokens: u.total_token_count as u64,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}
