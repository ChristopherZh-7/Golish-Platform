//! Restore and persist the sub-agent's conversation chain via the
//! [`SubAgentChainPersistence`] trait.
//!
//! This is the PentAGI-style persistent message-chain pattern: the chain
//! survives across invocations of the same agent within a session/task so
//! cross-invocation context (memory) can be injected later via the briefing
//! system.

use rig::completion::Message;

use crate::definition::SubAgentContext;
use crate::executor_helpers::serialize_chat_history;
use crate::executor_types::SubAgentExecutorContext;

pub(super) async fn maybe_restore_chain(
    ctx: &SubAgentExecutorContext<'_>,
    parent_context: &SubAgentContext,
    agent_id: &str,
) -> Option<uuid::Uuid> {
    let persistence = ctx.chain_persistence?;
    let session_uuid = ctx.session_id.and_then(|s| uuid::Uuid::parse_str(s).ok())?;
    let task_uuid = parent_context
        .task_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    match persistence
        .chain_create(session_uuid, task_uuid, None, agent_id, None, None)
        .await
    {
        Ok(cid) => {
            tracing::info!(
                "[sub-agent:{}] Created/restored persistent chain {}",
                agent_id,
                cid
            );
            Some(cid)
        }
        Err(e) => {
            tracing::warn!("[sub-agent:{}] Failed to restore chain: {}", agent_id, e);
            None
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
