//! Restore and persist the sub-agent's conversation chain via the
//! [`SubAgentChainPersistence`] trait.
//!
//! This is the PentAGI-style persistent message-chain pattern: the chain
//! survives across invocations of the same agent within a session/task so
//! cross-invocation context (memory) can be injected later via the briefing
//! system.

use rig::completion::Message;

use crate::definition::SubAgentContext;
use crate::executor_helpers::{deserialize_chat_history, serialize_chat_history};
use crate::executor_types::SubAgentExecutorContext;

/// Resolve the chain for this delegation, honoring the AI-controlled
/// `ctx.resume`:
/// - `Some("<uuid>")` → continue THAT exact prior chain (replay its messages).
/// - `Some("latest")`/other non-uuid → continue this agent's most recent chain.
/// - `None` → create a fresh chain (legacy behavior).
///
/// Returns `(chain_id, prior_messages)`. `prior_messages` is empty for a fresh
/// chain; for a resume it is the full-fidelity replay (incl. tool results /
/// evidence ids) so the worker remembers what it already did.
pub(super) async fn maybe_restore_chain(
    ctx: &SubAgentExecutorContext<'_>,
    parent_context: &SubAgentContext,
    agent_id: &str,
) -> (Option<uuid::Uuid>, Vec<Message>) {
    let Some(persistence) = ctx.chain_persistence else {
        return (None, Vec::new());
    };
    let Some(session_uuid) = ctx.session_id.and_then(|s| uuid::Uuid::parse_str(s).ok()) else {
        return (None, Vec::new());
    };
    let task_uuid = parent_context
        .task_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    // Resume: continue a prior worker the AI named.
    if let Some(resume) = ctx.resume.as_deref() {
        if let Ok(chain_id) = uuid::Uuid::parse_str(resume) {
            // Precise: continue this exact chain id.
            if let Ok(Some(json)) = persistence.chain_load_by_id(chain_id).await {
                let msgs = deserialize_chat_history(&json);
                tracing::info!(
                    "[sub-agent:{}] Resumed chain {} ({} prior messages)",
                    agent_id,
                    chain_id,
                    msgs.len()
                );
                return (Some(chain_id), msgs);
            }
            tracing::warn!(
                "[sub-agent:{}] resume id {} not found; starting fresh",
                agent_id,
                chain_id
            );
        } else if let Ok(Some((chain_id, json))) = persistence
            .chain_load_latest(session_uuid, task_uuid, agent_id)
            .await
        {
            // "latest": continue this agent's most recent chain.
            let msgs = deserialize_chat_history(&json);
            tracing::info!(
                "[sub-agent:{}] Resumed latest chain {} ({} prior messages)",
                agent_id,
                chain_id,
                msgs.len()
            );
            return (Some(chain_id), msgs);
        } else {
            tracing::info!(
                "[sub-agent:{}] no prior chain to resume; starting fresh",
                agent_id
            );
        }
    }

    // Fresh chain (default / resume miss).
    match persistence
        .chain_create(session_uuid, task_uuid, None, agent_id, None, None)
        .await
    {
        Ok(cid) => {
            tracing::info!("[sub-agent:{}] Created persistent chain {}", agent_id, cid);
            (Some(cid), Vec::new())
        }
        Err(e) => {
            tracing::warn!("[sub-agent:{}] Failed to create chain: {}", agent_id, e);
            (None, Vec::new())
        }
    }
}

pub(super) async fn persist_chain(
    ctx: &SubAgentExecutorContext<'_>,
    chain_id: Option<uuid::Uuid>,
    chat_history: &[Message],
    duration_ms: u64,
    agent_id: &str,
) {
    let (Some(persistence), Some(cid)) = (ctx.chain_persistence, chain_id) else {
        return;
    };

    let chain_json = serialize_chat_history(chat_history);
    match persistence.chain_update(cid, &chain_json).await {
        Ok(_) => {
            if let Err(e) = persistence
                .chain_update_usage(cid, 0, 0, 0, 0.0, 0.0, duration_ms as i32)
                .await
            {
                tracing::warn!(
                    "[sub-agent:{}] Failed to update chain usage {}: {}",
                    agent_id,
                    cid,
                    e
                );
            }
            tracing::info!(
                "[sub-agent:{}] Persisted {} messages to chain {}",
                agent_id,
                chat_history.len(),
                cid
            );
        }
        Err(e) => {
            tracing::warn!(
                "[sub-agent:{}] Failed to persist chain {}: {}",
                agent_id,
                cid,
                e
            );
        }
    }
}
