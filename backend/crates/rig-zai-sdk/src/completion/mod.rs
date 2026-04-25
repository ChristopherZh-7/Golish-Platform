//! `CompletionModel` implementation for the Z.AI API.
//!
//! Layout:
//! - [`CompletionModel`] struct + constructor + `Debug` impl in this `mod.rs`.
//! - [`StreamingResponseData`] / [`StreamingUsage`] streaming-final-payload
//!   types + their `GetTokenUsage` impl, also here.
//! - [`conversion`]: pure rig↔Z.AI message/tool conversion (impl block on
//!   `CompletionModel`).
//! - [`runtime`]: the actual `completion()` / `stream()` async methods
//!   that hit `/chat/completions` (impl block for the
//!   `rig::completion::CompletionModel` trait).

use rig::completion::Usage;
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;
use serde::{Deserialize, Serialize};

use crate::client::Client;

mod conversion;
mod runtime;

#[cfg(test)]
mod tests;

/// Default max tokens for Z.AI models.
pub(super) const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Completion model for Z.AI API.
#[derive(Clone)]
pub struct CompletionModel {
    pub(super) client: Client,
    pub(super) model: String,
}

impl CompletionModel {
    /// Create a new completion model.
    pub fn new(client: Client, model: String) -> Self {
        Self { client, model }
    }

    /// Get the model identifier.
    pub fn model(&self) -> &str {
        &self.model
    }
}

impl std::fmt::Debug for CompletionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionModel")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

// ============================================================================
// StreamingResponseData
// ============================================================================

/// Data accumulated during streaming, returned as the final response.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamingResponseData {
    /// Token usage statistics (populated at end of stream).
    pub usage: Option<StreamingUsage>,
}

/// Token usage for streaming responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamingUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl rig::completion::GetTokenUsage for StreamingResponseData {
    fn token_usage(&self) -> Option<Usage> {
        self.usage.as_ref().map(|u| Usage {
            input_tokens: u.prompt_tokens as u64,
            output_tokens: u.completion_tokens as u64,
            total_tokens: u.total_tokens as u64,
            cached_input_tokens: 0,
        })
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract text content from user message content.
pub(super) fn extract_user_text(content: &OneOrMany<UserContent>) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            UserContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
