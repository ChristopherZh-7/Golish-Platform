//! Stream-event mapping for the OpenAI Responses API.
//!
//! This is the heart of streaming support: every `ResponseStreamEvent` from
//! `async-openai` is translated into at most one rig `RawStreamingChoice`.
//!
//! Two invariants matter here:
//! 1. **Reasoning is never mixed with text.**  Reasoning summary/text deltas
//!    map to `RawStreamingChoice::ReasoningDelta`, never `Message`.
//! 2. **`encrypted_content` is captured on `ResponseCompleted`** and surfaced
//!    via `StreamingResponseData::reasoning_encrypted_content`, so the
//!    agentic loop can inject it into accumulated reasoning items for the
//!    next turn (required for stateless multi-turn with reasoning models).

use async_openai::types::responses::{OutputItem, ResponseStreamEvent};
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, ToolCallDeltaContent};

use crate::response::{StreamingResponseData, Usage};

/// Map an async-openai `ResponseStreamEvent` to a rig-core `RawStreamingChoice`.
///
/// This is the core function that ensures reasoning events are explicitly
/// separated from text events.
pub(crate) fn map_stream_event(
    event: ResponseStreamEvent,
) -> Option<RawStreamingChoice<StreamingResponseData>> {
    match event {
        // Text deltas → Message
        ResponseStreamEvent::ResponseOutputTextDelta(e) => {
            tracing::trace!("Text delta: {} chars", e.delta.len());
            Some(RawStreamingChoice::Message(e.delta))
        }

        // Reasoning summary deltas → ReasoningDelta (EXPLICIT separation!)
        ResponseStreamEvent::ResponseReasoningSummaryTextDelta(e) => {
            tracing::trace!("Reasoning summary delta: {} chars", e.delta.len());
            Some(RawStreamingChoice::ReasoningDelta {
                id: Some(e.item_id),
                reasoning: e.delta,
            })
        }

        // Reasoning text deltas → ReasoningDelta
        ResponseStreamEvent::ResponseReasoningTextDelta(e) => {
            tracing::trace!("Reasoning text delta: {} chars", e.delta.len());
            Some(RawStreamingChoice::ReasoningDelta {
                id: Some(e.item_id),
                reasoning: e.delta,
            })
        }

        // Function call argument deltas → ToolCallDelta
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(e) => {
            tracing::trace!("Function call args delta: {} chars", e.delta.len());
            Some(RawStreamingChoice::ToolCallDelta {
                id: e.item_id,
                internal_call_id: nanoid::nanoid!(),
                content: ToolCallDeltaContent::Delta(e.delta),
            })
        }

        // Output item added - check for function calls
        ResponseStreamEvent::ResponseOutputItemAdded(e) => {
            if let OutputItem::FunctionCall(fc) = e.item {
                tracing::info!("Function call started: {}", fc.name);
                // fc.id is Option<String>, use empty string as fallback
                let id = fc.id.clone().unwrap_or_default();
                Some(RawStreamingChoice::ToolCall(RawStreamingToolCall {
                    id,
                    internal_call_id: nanoid::nanoid!(),
                    call_id: Some(fc.call_id),
                    name: fc.name,
                    arguments: serde_json::json!({}),
                    signature: None,
                    additional_params: None,
                }))
            } else {
                None
            }
        }

        // Response completed → FinalResponse with usage and reasoning encrypted_content
        ResponseStreamEvent::ResponseCompleted(e) => {
            tracing::info!("Response completed");
            let usage = e.response.usage.map(|u| Usage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                total_tokens: u.total_tokens,
            });

            // Extract reasoning items' encrypted_content for stateless multi-turn support.
            // This allows the agentic loop to inject encrypted_content into accumulated
            // reasoning items, which is required for subsequent turns with reasoning models.
            let mut reasoning_encrypted_content = std::collections::HashMap::new();
            let reasoning_item_count = e
                .response
                .output
                .iter()
                .filter(|item| matches!(item, OutputItem::Reasoning(_)))
                .count();

            for output in &e.response.output {
                if let OutputItem::Reasoning(reasoning) = output {
                    if let Some(encrypted) = &reasoning.encrypted_content {
                        reasoning_encrypted_content.insert(reasoning.id.clone(), encrypted.clone());
                        tracing::debug!(
                            "[OpenAI] Captured encrypted_content for reasoning {}: {} bytes",
                            reasoning.id,
                            encrypted.len()
                        );
                    } else {
                        tracing::warn!(
                            "[OpenAI] Reasoning item {} has NO encrypted_content! This will cause multi-turn failures. \
                             Make sure 'include: [reasoning.encrypted_content]' is in the request.",
                            reasoning.id
                        );
                    }
                }
            }

            if reasoning_item_count > 0 && reasoning_encrypted_content.is_empty() {
                tracing::error!(
                    "[OpenAI] Found {} reasoning items but captured 0 encrypted_content values! \
                     The 'include' parameter may not be working.",
                    reasoning_item_count
                );
            }

            Some(RawStreamingChoice::FinalResponse(StreamingResponseData {
                usage,
                reasoning_encrypted_content,
            }))
        }

        // Errors - ResponseErrorEvent has code, message, param fields
        ResponseStreamEvent::ResponseError(e) => {
            tracing::error!(
                "OpenAI response error: code={:?}, message={:?}",
                e.code,
                e.message
            );
            Some(RawStreamingChoice::Message(format!(
                "[Error: {:?} - {:?}]",
                e.code, e.message
            )))
        }

        // Response failed
        ResponseStreamEvent::ResponseFailed(e) => {
            tracing::error!("OpenAI response failed: {:?}", e.response.status);
            Some(RawStreamingChoice::Message(format!(
                "[Response failed: {:?}]",
                e.response.status
            )))
        }

        // Refusal deltas
        ResponseStreamEvent::ResponseRefusalDelta(e) => {
            tracing::warn!("Refusal delta received");
            Some(RawStreamingChoice::Message(format!(
                "[Refusal] {}",
                e.delta
            )))
        }

        // Lifecycle events we don't need to emit as content
        ResponseStreamEvent::ResponseCreated(_)
        | ResponseStreamEvent::ResponseInProgress(_)
        | ResponseStreamEvent::ResponseIncomplete(_)
        | ResponseStreamEvent::ResponseQueued(_)
        | ResponseStreamEvent::ResponseOutputItemDone(_)
        | ResponseStreamEvent::ResponseContentPartAdded(_)
        | ResponseStreamEvent::ResponseContentPartDone(_)
        | ResponseStreamEvent::ResponseOutputTextDone(_)
        | ResponseStreamEvent::ResponseRefusalDone(_)
        | ResponseStreamEvent::ResponseReasoningSummaryPartAdded(_)
        | ResponseStreamEvent::ResponseReasoningSummaryPartDone(_)
        | ResponseStreamEvent::ResponseReasoningSummaryTextDone(_)
        | ResponseStreamEvent::ResponseReasoningTextDone(_)
        | ResponseStreamEvent::ResponseFunctionCallArgumentsDone(_) => None,

        // Other events (web search, file search, MCP, etc.) - log and skip
        other => {
            tracing::debug!("Unhandled OpenAI stream event: {:?}", other);
            None
        }
    }
}
