//! Restore and persist the sub-agent's conversation chain in PostgreSQL.
//!
//! This is the PentAGI-style persistent message-chain pattern: the chain
//! survives across invocations of the same agent within a session/task so
//! cross-invocation context (memory) can be injected later via the briefing
//! system.

use rig::completion::Message;

use crate::definition::SubAgentContext;
use crate::executor_helpers::{restore_or_create_chain, serialize_chat_history};
use crate::executor_types::SubAgentExecutorContext;

/// Restore (or create) the persistent chain row for this sub-agent invocation.
///
/// Returns `Some(chain_id)` when persistence is enabled and a row exists or was
/// created; `None` when no DB pool is configured or no session id is available.
///
/// We deliberately do **not** pre-populate `chat_history` from the restored
/// rows — the new task prompt is fresh each invocation. The existing chain row
/// is updated by [`persist_chain`] so future briefings can read prior context.
pub(super) async fn maybe_restore_chain(
    ctx: &SubAgentExecutorContext<'_>,
    parent_context: &SubAgentContext,
    agent_id: &str,
) -> Option<uuid::Uuid> {
    let pool = ctx.db_pool?;
    let session_uuid = ctx.session_id.and_then(|s| uuid::Uuid::parse_str(s).ok())?;
    let task_uuid = parent_context
        .task_id
        .as_deref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    match restore_or_create_chain(pool, session_uuid, task_uuid, agent_id).await {
        Ok((cid, restored_history)) => {
            if !restored_history.is_empty() {
                tracing::info!(
                    "[sub-agent:{}] Restored {} messages from persistent chain {}",
                    agent_id,
                    restored_history.len(),
                    cid
                );
            }
            Some(cid)
        }
        Err(e) => {
            tracing::warn!(
                "[sub-agent:{}] Failed to restore chain: {}",
                agent_id,
                e
            );
            None
        }
    }
}

/// Persist the final chat history into the chain row at the end of an
/// invocation. Increments `duration_ms` and bumps `updated_at`.
pub(super) async fn persist_chain(
    ctx: &SubAgentExecutorContext<'_>,
    chain_id: Option<uuid::Uuid>,
    chat_history: &[Message],
    duration_ms: u64,
    agent_id: &str,
) {
    let (Some(pool), Some(cid)) = (ctx.db_pool, chain_id) else {
        return;
    };

    let chain_json = serialize_chat_history(chat_history);
    let result = sqlx::query(
        "UPDATE message_chains SET chain = $1, duration_ms = duration_ms + $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(&chain_json)
    .bind(duration_ms as i32)
    .bind(cid)
    .execute(pool.as_ref())
    .await;

    match result {
        Ok(_) => {
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
