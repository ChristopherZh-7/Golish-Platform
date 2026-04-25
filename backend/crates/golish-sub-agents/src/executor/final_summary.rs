//! Run the final tool-less LLM call when the iteration cap is exceeded.
//!
//! When `iteration > agent_def.max_iterations` and the agent never called the
//! barrier tool, we still want to give the user *something* to read. This
//! helper streams a final completion with **no tools** so the model is forced
//! to summarise its work as plain text.

use futures::StreamExt;
use rig::completion::{CompletionModel as RigCompletionModel, Message};
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;
use rig::streaming::StreamedAssistantContent;

use crate::definition::SubAgentDefinition;
use crate::executor_types::SubAgentExecutorContext;
use golish_core::events::AiEvent;
use golish_llm_providers::ModelCapabilities;

/// Stream a final tool-less call to coax the model into emitting a summary.
///
/// Appends streamed text directly to `accumulated_response` and emits
/// [`AiEvent::SubAgentTextDelta`] events. Failures are logged and ignored —
/// any pre-existing `accumulated_response` is preserved.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_final_summary<M>(
    agent_def: &SubAgentDefinition,
    chat_history: &[Message],
    ctx: &SubAgentExecutorContext<'_>,
    agent_id: &str,
    parent_request_id: &str,
    accumulated_response: &mut String,
    model: &M,
) where
    M: RigCompletionModel + Sync,
{
    tracing::info!(
        "[sub-agent] Max iterations ({}) reached, making final toolless call for summary",
        agent_def.max_iterations
    );

    let caps = ModelCapabilities::detect(ctx.provider_name, ctx.model_name);
    let temperature = if caps.supports_temperature {
        Some(ctx.temperature_override.unwrap_or(0.3) as f64)
    } else {
        None
    };
    let max_tokens = ctx.max_tokens_override.unwrap_or(8192) as u64;
    let additional_params = ctx.top_p_override.map(|tp| serde_json::json!({ "top_p": tp }));

    // NVIDIA NIM workaround: provider serialises system content as an array of
    // text parts but only accepts plain strings. Reroute the system prompt as
    // the first user message.
    let is_nvidia = ctx.provider_name == "nvidia";
    let (preamble, effective_history) = if is_nvidia {
        let mut h = vec![Message::User {
            content: OneOrMany::one(UserContent::text(&agent_def.system_prompt)),
        }];
        h.extend(chat_history.to_vec());
        (None, h)
    } else {
        (Some(agent_def.system_prompt.clone()), chat_history.to_vec())
    };

    let final_request = rig::completion::CompletionRequest {
        preamble,
        chat_history: OneOrMany::many(effective_history.clone())
            .unwrap_or_else(|_| OneOrMany::one(effective_history[0].clone())),
        documents: vec![],
        tools: vec![],
        temperature,
        max_tokens: Some(max_tokens),
        tool_choice: None,
        additional_params,
        model: None,
        output_schema: None,
    };

    if let Some(stats) = ctx.api_request_stats {
        stats.record_sent(ctx.provider_name).await;
    }

    match model.stream(final_request).await {
        Ok(mut final_stream) => {
            if let Some(stats) = ctx.api_request_stats {
                stats.record_received(ctx.provider_name).await;
            }
            while let Some(chunk_result) = final_stream.next().await {
                if let Ok(StreamedAssistantContent::Text(text_msg)) = chunk_result {
                    accumulated_response.push_str(&text_msg.text);
                    let _ = ctx.event_tx.send(AiEvent::SubAgentTextDelta {
                        agent_id: agent_id.to_string(),
                        delta: text_msg.text,
                        accumulated: accumulated_response.clone(),
                        parent_request_id: parent_request_id.to_string(),
                    });
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                "[sub-agent] Final summary call failed: {}, returning accumulated response",
                e
            );
        }
    }
}
