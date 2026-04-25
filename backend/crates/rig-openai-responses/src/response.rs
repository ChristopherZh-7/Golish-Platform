//! Response parsing for the OpenAI Responses API.
//!
//! This module owns the **non-streaming** response → rig conversion, plus the
//! shared `StreamingResponseData` accumulator that the streaming path emits as
//! its `FinalResponse`.  The two paths are intentionally split:
//!
//! - [`convert_response`] — synchronous decode of a `responses.create` body
//!   into rig's `CompletionResponse`, including reasoning and tool calls.
//! - [`StreamingResponseData`] / [`Usage`] — final-event payloads built by
//!   `stream_map::map_stream_event` on `ResponseCompleted`.

use async_openai::types::responses::{OutputItem, OutputMessageContent, Response, SummaryPart};
use rig::completion::{AssistantContent, CompletionResponse};
use rig::message::{Text, ToolCall, ToolFunction};
use rig::one_or_many::OneOrMany;
use serde::{Deserialize, Serialize};

/// Convert a non-streaming OpenAI `Response` into rig's `CompletionResponse`.
///
/// Walks the `output` list and turns each item into the corresponding
/// `AssistantContent` variant (Text / Reasoning / ToolCall).  Reasoning items
/// store their `encrypted_content` in the rig `Reasoning::signature` field so
/// it can be round-tripped on subsequent turns for stateless multi-turn
/// reasoning models.
pub(crate) fn convert_response(response: Response) -> CompletionResponse<Response> {
    let mut content: Vec<AssistantContent> = Vec::new();

    // Extract content from output items
    for output in &response.output {
        match output {
            OutputItem::Message(msg) => {
                for c in &msg.content {
                    match c {
                        OutputMessageContent::OutputText(text_output) => {
                            content.push(AssistantContent::Text(Text {
                                text: text_output.text.clone(),
                            }));
                        }
                        OutputMessageContent::Refusal(refusal) => {
                            content.push(AssistantContent::Text(Text {
                                text: format!("[Refusal]: {}", refusal.refusal),
                            }));
                        }
                    }
                }
            }
            OutputItem::Reasoning(reasoning) => {
                // Extract reasoning texts from summary, preserving each part separately
                // This ensures proper round-tripping when the reasoning is sent back to OpenAI
                let reasoning_parts: Vec<String> = reasoning
                    .summary
                    .iter()
                    .map(|SummaryPart::SummaryText(st)| st.text.clone())
                    .collect();

                // Also check the content field if present (populated with reasoning.encrypted_content include)
                let content_parts: Vec<String> = reasoning
                    .content
                    .as_ref()
                    .map(|c| c.iter().map(|rtc| rtc.text.clone()).collect())
                    .unwrap_or_default();

                // Combine: prefer content if available, otherwise use summary
                let all_parts = if !content_parts.is_empty() {
                    content_parts
                } else {
                    reasoning_parts
                };

                if !all_parts.is_empty() {
                    // Create Reasoning with multi() to preserve structure.
                    // Store encrypted_content in the signature field - this allows us to
                    // pass it back to OpenAI in subsequent turns for stateless operation.
                    // See: https://platform.openai.com/docs/guides/reasoning
                    content.push(AssistantContent::Reasoning({
                        let mut r = rig::message::Reasoning::multi(all_parts)
                            .with_id(reasoning.id.clone());
                        // Store encrypted_content as signature on the first text block
                        if let Some(sig) = &reasoning.encrypted_content {
                            if let Some(rig::message::ReasoningContent::Text { signature, .. }) =
                                r.content.first_mut()
                            {
                                *signature = Some(sig.clone());
                            }
                        }
                        r
                    }));
                }
            }
            OutputItem::FunctionCall(fc) => {
                let arguments = golish_json_repair::parse_tool_args(&fc.arguments);
                // fc.id is Option<String>, use empty string as fallback
                let id = fc.id.clone().unwrap_or_default();
                content.push(AssistantContent::ToolCall(ToolCall {
                    id,
                    call_id: Some(fc.call_id.clone()),
                    function: ToolFunction {
                        name: fc.name.clone(),
                        arguments,
                    },
                    signature: None,
                    additional_params: None,
                }));
            }
            _ => {}
        }
    }

    // Extract usage
    let usage = response.usage.as_ref().map(|u| rig::completion::Usage {
        input_tokens: u.input_tokens as u64,
        output_tokens: u.output_tokens as u64,
        total_tokens: u.total_tokens as u64,
        cached_input_tokens: 0,
    });

    CompletionResponse {
        choice: OneOrMany::many(content).unwrap_or_else(|_| {
            OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            }))
        }),
        usage: usage.unwrap_or(rig::completion::Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: 0,
        }),
        raw_response: response,
        message_id: None,
    }
}

/// Data accumulated during streaming, returned as the final response.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamingResponseData {
    /// Token usage statistics (populated at end of stream).
    pub usage: Option<Usage>,
    /// Map of reasoning item IDs to their encrypted_content (for stateless multi-turn).
    /// This is populated from ResponseCompleted and allows the agentic loop to
    /// inject encrypted_content into accumulated reasoning items.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub reasoning_encrypted_content: std::collections::HashMap<String, String>,
}

/// Token usage for streaming responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

impl rig::completion::GetTokenUsage for StreamingResponseData {
    fn token_usage(&self) -> Option<rig::completion::Usage> {
        self.usage.as_ref().map(|u| rig::completion::Usage {
            input_tokens: u.input_tokens as u64,
            output_tokens: u.output_tokens as u64,
            total_tokens: u.total_tokens as u64,
            cached_input_tokens: 0,
        })
    }
}
