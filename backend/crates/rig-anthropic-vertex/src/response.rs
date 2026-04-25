//! Response parsing for the Anthropic Vertex provider.
//!
//! Owns the **non-streaming** response → rig conversion plus the
//! `StreamingCompletionResponseData` accumulator that the streaming path
//! emits as its `FinalResponse`.  The non-streaming and streaming paths
//! share a single response shape (`types::CompletionResponse`), but their
//! lifecycle is different enough to warrant separate plumbing.

use rig::completion::{AssistantContent, CompletionResponse, Usage};
use rig::message::{Reasoning, Text, ToolCall, ToolFunction};
use rig::one_or_many::OneOrMany;
use serde::{Deserialize, Serialize};

use crate::types::{self, ContentBlock};

/// Convert a non-streaming Anthropic response into rig's `CompletionResponse`.
///
/// Reasoning (`Thinking`) blocks are emitted **before** any other content so
/// the resulting message round-trips correctly when sent back to Anthropic
/// (the API requires thinking-first ordering when extended thinking is on).
pub(crate) fn convert_response(
    response: types::CompletionResponse,
) -> CompletionResponse<types::CompletionResponse> {
    // IMPORTANT: When thinking is enabled, thinking blocks MUST come first
    // Separate thinking from other content to ensure correct ordering
    let mut thinking_content: Vec<AssistantContent> = vec![];
    let mut other_content: Vec<AssistantContent> = vec![];

    for block in response.content.iter() {
        match block {
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                // Convert to AssistantContent::Reasoning with signature
                thinking_content.push(AssistantContent::Reasoning(
                    Reasoning::new_with_signature(thinking, Some(signature.clone())),
                ));
            }
            ContentBlock::Text { text, .. } => {
                other_content.push(AssistantContent::Text(Text { text: text.clone() }));
            }
            ContentBlock::ToolUse { id, name, input } => {
                other_content.push(AssistantContent::ToolCall(ToolCall {
                    id: id.clone(),
                    call_id: None,
                    function: ToolFunction {
                        name: name.clone(),
                        arguments: input.clone(),
                    },
                    signature: None,
                    additional_params: None,
                }));
            }
            _ => {}
        }
    }

    // Combine: thinking first, then other content
    thinking_content.append(&mut other_content);
    let choice = thinking_content;

    CompletionResponse {
        choice: OneOrMany::many(choice).unwrap_or_else(|_| {
            OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            }))
        }),
        usage: Usage {
            input_tokens: response.usage.input_tokens as u64,
            output_tokens: response.usage.output_tokens as u64,
            total_tokens: (response.usage.input_tokens + response.usage.output_tokens) as u64,
            cached_input_tokens: 0,
        },
        raw_response: response,
        message_id: None,
    }
}

/// Response type for streaming (wraps the streaming response).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StreamingCompletionResponseData {
    /// Accumulated text
    pub text: String,
    /// Token usage (filled at end)
    pub usage: Option<types::Usage>,
}

impl rig::completion::GetTokenUsage for StreamingCompletionResponseData {
    fn token_usage(&self) -> Option<Usage> {
        self.usage.as_ref().map(|u| Usage {
            input_tokens: u.input_tokens as u64,
            output_tokens: u.output_tokens as u64,
            total_tokens: (u.input_tokens + u.output_tokens) as u64,
            cached_input_tokens: 0,
        })
    }
}
