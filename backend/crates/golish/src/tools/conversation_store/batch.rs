//! `conv_save_batch` — atomic multi-conversation upsert.
//!
//! Lifted out of the parent module so the per-command file stays under the
//! 500-line module budget. Re-exported via `super::`.

use crate::error::GolishError;
use serde::{Deserialize, Serialize};

use crate::state::DbState;
use super::{ChatMessageRow, ConversationRow, TerminalStateRow, TimelineBlockRow, WorkspacePreferences};

// ─── Batch Save ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTimelineEntry {
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub blocks: Vec<TimelineBlockRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvBatchItem {
    pub conversation: ConversationRow,
    pub messages: Vec<ChatMessageRow>,
    pub terminal_states: Vec<TerminalStateRow>,
    pub timelines: Vec<BatchTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvBatchSavePayload {
    pub project_path: String,
    /// All conversation IDs that should survive for this project.
    /// Any DB rows for the same project_path whose ID is absent will be deleted.
    pub surviving_ids: Vec<String>,
    /// Changed conversations to upsert (may be a subset of surviving_ids).
    pub items: Vec<ConvBatchItem>,
    pub preferences: WorkspacePreferences,
}

/// Save all changed conversations + preferences in one transaction.
///
/// Compared to calling `conv_save`, `conv_save_messages`, `conv_save_terminal_state`,
/// and `conv_save_timeline` individually, this command:
///   1. Reduces IPC round-trips from O(N*4) to 1.
///   2. Guarantees atomicity — partial writes are impossible.
///   3. Cleans up stale conversations in the same transaction.
#[tauri::command]
pub async fn conv_save_batch(
    state: tauri::State<'_, DbState>,
    payload: ConvBatchSavePayload,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let mut tx = pool.begin().await?;

    // ── 1. Delete stale conversations not in `surviving_ids` ──
    if payload.surviving_ids.is_empty() {
        sqlx::query("DELETE FROM conversations WHERE project_path = $1")
            .bind(&payload.project_path)
            .execute(&mut *tx)
            .await
?;
    } else {
        // Build a ($2, $3, ...) placeholder list for the surviving IDs
        let placeholders: Vec<String> = (0..payload.surviving_ids.len())
            .map(|i| format!("${}", i + 2))
            .collect();
        let query_str = format!(
            "DELETE FROM conversations WHERE project_path = $1 AND id NOT IN ({})",
            placeholders.join(", ")
        );
        let mut q = sqlx::query(&query_str).bind(&payload.project_path);
        for id in &payload.surviving_ids {
            q = q.bind(id);
        }
        q.execute(&mut *tx).await?;
    }

    // ── 2. Upsert each changed conversation ──
    for item in &payload.items {
        let conv = &item.conversation;

        sqlx::query(
            r#"INSERT INTO conversations (id, title, ai_session_id, project_path, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5, to_timestamp($6::double precision / 1000))
               ON CONFLICT (id) DO UPDATE SET
                 title = EXCLUDED.title,
                 ai_session_id = EXCLUDED.ai_session_id,
                 sort_order = EXCLUDED.sort_order,
                 updated_at = NOW()"#,
        )
        .bind(&conv.id)
        .bind(&conv.title)
        .bind(&conv.ai_session_id)
        .bind(&conv.project_path)
        .bind(conv.sort_order)
        .bind(conv.created_at as f64)
        .execute(&mut *tx)
        .await
?;

        // Messages: delete + re-insert
        sqlx::query("DELETE FROM chat_messages WHERE conversation_id = $1")
            .bind(&conv.id)
            .execute(&mut *tx)
            .await
?;

        for msg in &item.messages {
            sqlx::query(
                r#"INSERT INTO chat_messages
                   (id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, tool_call_offsets, sort_order, created_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, to_timestamp($11::double precision / 1000))"#,
            )
            .bind(&msg.id)
            .bind(&conv.id)
            .bind(&msg.role)
            .bind(&msg.content)
            .bind(&msg.thinking)
            .bind(&msg.error)
            .bind(&msg.tool_calls)
            .bind(msg.tool_calls_content_offset)
            .bind(&msg.tool_call_offsets)
            .bind(msg.sort_order)
            .bind(msg.created_at as f64)
            .execute(&mut *tx)
            .await
?;
        }

        // Terminal states
        for ts in &item.terminal_states {
            if let Some(ref conv_id) = ts.conversation_id {
                sqlx::query(
                    "DELETE FROM terminal_state WHERE conversation_id = $1 AND session_id != $2",
                )
                .bind(conv_id)
                .bind(&ts.session_id)
                .execute(&mut *tx)
                .await
?;
            }

            sqlx::query(
                r#"INSERT INTO terminal_state (session_id, conversation_id, working_directory, scrollback, custom_name, plan_json, execution_mode, retired_plans_json, plan_message_id)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                   ON CONFLICT (session_id) DO UPDATE SET
                     working_directory = EXCLUDED.working_directory,
                     scrollback = EXCLUDED.scrollback,
                     custom_name = EXCLUDED.custom_name,
                     plan_json = EXCLUDED.plan_json,
                     execution_mode = EXCLUDED.execution_mode,
                     retired_plans_json = EXCLUDED.retired_plans_json,
                     plan_message_id = EXCLUDED.plan_message_id,
                     updated_at = NOW()"#,
            )
            .bind(&ts.session_id)
            .bind(&ts.conversation_id)
            .bind(&ts.working_directory)
            .bind(&ts.scrollback)
            .bind(&ts.custom_name)
            .bind(&ts.plan_json)
            .bind(&ts.execution_mode)
            .bind(&ts.retired_plans_json)
            .bind(&ts.plan_message_id)
            .execute(&mut *tx)
            .await
?;
        }

        // Timeline blocks per terminal
        for entry in &item.timelines {
            sqlx::query("DELETE FROM timeline_blocks WHERE session_id = $1")
                .bind(&entry.session_id)
                .execute(&mut *tx)
                .await
?;

            for block in &entry.blocks {
                sqlx::query(
                    r#"INSERT INTO timeline_blocks
                       (id, session_id, conversation_id, block_type, data, batch_id, sort_order, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, COALESCE($8::timestamptz, NOW()))"#,
                )
                .bind(&block.id)
                .bind(&entry.session_id)
                .bind(entry.conversation_id.as_deref())
                .bind(&block.block_type)
                .bind(&block.data)
                .bind(&block.batch_id)
                .bind(block.sort_order)
                .bind(&block.timestamp)
                .execute(&mut *tx)
                .await
?;
            }
        }
    }

    // ── 3. Save workspace preferences ──
    sqlx::query(
        r#"INSERT INTO workspace_preferences
           (project_path, active_conversation_id, ai_model, approval_mode, approval_patterns)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (project_path) DO UPDATE SET
             active_conversation_id = EXCLUDED.active_conversation_id,
             ai_model = EXCLUDED.ai_model,
             approval_mode = EXCLUDED.approval_mode,
             approval_patterns = EXCLUDED.approval_patterns,
             updated_at = NOW()"#,
    )
    .bind(&payload.project_path)
    .bind(&payload.preferences.active_conversation_id)
    .bind(&payload.preferences.ai_model)
    .bind(&payload.preferences.approval_mode)
    .bind(&payload.preferences.approval_patterns)
    .execute(&mut *tx)
    .await
?;

    tx.commit().await?;
    Ok(())
}

// ─── Workspace Preferences ───────────────────────────────────────────────────

