//! Compaction integration for the agentic loop.
//!
//! Two flavours are exposed:
//!
//! - [`pre_turn_compaction`] runs at iteration 1 using token usage from the
//!   *previous* turn. This catches cases where a single-iteration agent run
//!   would otherwise exit before any compaction check could fire.
//! - [`inter_turn_compaction`] runs at iteration > 1. Unlike the pre-turn
//!   variant, when compaction fails AND the context is still over budget it
//!   bubbles up a [`TerminalErrorEmitted`] so the bridge can surface a clean
//!   error to the user instead of looping forever.

use anyhow::Result;
use rig::completion::Message;

use golish_core::events::AiEvent;

use super::compaction::maybe_compact;
use super::context::{AgenticLoopContext, TerminalErrorEmitted};

/// Run a compaction check before the first iteration's LLM call.
///
/// Errors during compaction are logged but not propagated — the turn proceeds
/// regardless. Used at iteration 1 so single-iteration runs still get a chance
/// to compact.
pub(super) async fn pre_turn_compaction(
    ctx: &AgenticLoopContext<'_>,
    chat_history: &mut Vec<Message>,
) {
    {
        let compaction_state = ctx.compaction_state.read().await;
        if compaction_state.last_input_tokens.is_some() {
            tracing::info!(
                "[compaction] Pre-turn check - tokens: {:?}, using_heuristic: {}",
                compaction_state.last_input_tokens,
                compaction_state.using_heuristic
            );
        }
    }

    let Some(session_id) = ctx.session_id else {
        return;
    };

    match maybe_compact(ctx, session_id, chat_history).await {
        Ok(Some(result)) => {
            if result.success {
                let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                    tokens_before: result.tokens_before,
                    messages_before: result.messages_before,
                    messages_after: chat_history.len(),
                    summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                    summary: result.summary.clone(),
                    summarizer_input: result.summarizer_input.clone(),
                });
                ctx.context_manager.update_from_messages(chat_history).await;
            } else {
                let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                    tokens_before: result.tokens_before,
                    messages_before: result.messages_before,
                    error: result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string()),
                    summarizer_input: result.summarizer_input.clone(),
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("[compaction] Pre-turn compaction error: {}", e);
        }
    }
}

/// Run a compaction check between iterations (iteration > 1).
///
/// On a failed compaction that leaves the context still over budget, this
/// emits a terminal `AiEvent::Error` and returns a [`TerminalErrorEmitted`]
/// preserving the partial response and history.
pub(super) async fn inter_turn_compaction(
    ctx: &AgenticLoopContext<'_>,
    chat_history: &mut Vec<Message>,
    iteration: usize,
    accumulated_response: &str,
) -> Result<()> {
    {
        let compaction_state = ctx.compaction_state.read().await;
        tracing::info!(
            "[compaction] Iteration {} - tokens: {:?}, using_heuristic: {}, attempted: {}",
            iteration,
            compaction_state.last_input_tokens,
            compaction_state.using_heuristic,
            compaction_state.attempted_this_turn
        );
    }

    let Some(session_id) = ctx.session_id else {
        return Ok(());
    };

    match maybe_compact(ctx, session_id, chat_history).await {
        Ok(Some(result)) => {
            if result.success {
                let _ = ctx.event_tx.send(AiEvent::CompactionCompleted {
                    tokens_before: result.tokens_before,
                    messages_before: result.messages_before,
                    messages_after: chat_history.len(),
                    summary_length: result.summary.as_ref().map(|s| s.len()).unwrap_or(0),
                    summary: result.summary.clone(),
                    summarizer_input: result.summarizer_input.clone(),
                });

                ctx.context_manager.update_from_messages(chat_history).await;
                Ok(())
            } else {
                let _ = ctx.event_tx.send(AiEvent::CompactionFailed {
                    tokens_before: result.tokens_before,
                    messages_before: result.messages_before,
                    error: result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string()),
                    summarizer_input: result.summarizer_input.clone(),
                });

                // Re-check whether we're still over the limit after the failed compaction.
                let check = {
                    let compaction_state = ctx.compaction_state.read().await;
                    ctx.context_manager
                        .should_compact(&compaction_state, ctx.model_name)
                };

                if check.should_compact {
                    tracing::error!(
                        "[compaction] Failed and context still exceeded: {} tokens",
                        check.current_tokens
                    );
                    let _ = ctx.event_tx.send(AiEvent::Error {
                        message: format!(
                            "Context compaction failed and limit exceeded ({} tokens). {}",
                            check.current_tokens,
                            result.error.unwrap_or_else(|| "Unknown error".to_string())
                        ),
                        error_type: "compaction_failed".to_string(),
                    });
                    Err(TerminalErrorEmitted::with_partial_state(
                        "Context compaction failed and limit exceeded",
                        (!accumulated_response.is_empty())
                            .then(|| accumulated_response.to_string()),
                        Some(chat_history.clone()),
                    )
                    .into())
                } else {
                    Ok(())
                }
            }
        }
        Ok(None) => Ok(()),
        Err(e) => {
            // Error checking compaction (non-fatal, log and continue).
            tracing::warn!("[compaction] Error during compaction check: {}", e);
            Ok(())
        }
    }
}
