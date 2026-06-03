//! Tauri commands for frontend conversation & timeline persistence.
//! Replaces workspace.json read/write with PostgreSQL-backed storage.

use crate::error::GolishError;
use serde::{Deserialize, Serialize};

use crate::state::DbState;

/// Raw row shape returned by `repo::conversation_store::list_by_project`
/// (mirrors the SELECT column order). Aliased to keep the call site readable.
type ConvListRow = (
    String,
    String,
    String,
    Option<String>,
    i32,
    chrono::DateTime<chrono::Utc>,
);

/// Raw row shape returned by `repo::conversation_store::load_preferences`.
type WorkspacePrefsRow = (
    Option<String>,
    Option<serde_json::Value>,
    Option<String>,
    Option<serde_json::Value>,
);

// ─── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: String,
    pub title: String,
    pub ai_session_id: String,
    pub project_path: Option<String>,
    pub sort_order: i32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageRow {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub error: Option<String>,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_calls_content_offset: Option<i32>,
    pub tool_call_offsets: Option<serde_json::Value>,
    /// Time-ordered reasoning bursts (JSONB array of ThinkingSegment) so the UI
    /// can restore interleaved Thought blocks instead of one merged block.
    #[serde(default)]
    pub thinking_segments: Option<serde_json::Value>,
    pub sort_order: i32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBlockRow {
    pub id: String,
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub block_type: String,
    pub data: serde_json::Value,
    pub batch_id: Option<String>,
    pub sort_order: i32,
    #[serde(default)]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStateRow {
    pub session_id: String,
    pub conversation_id: Option<String>,
    pub working_directory: String,
    pub scrollback: String,
    pub custom_name: Option<String>,
    pub plan_json: Option<serde_json::Value>,
    pub execution_mode: Option<String>,
    pub retired_plans_json: Option<serde_json::Value>,
    pub plan_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePreferences {
    pub active_conversation_id: Option<String>,
    pub ai_model: Option<serde_json::Value>,
    pub approval_mode: Option<String>,
    pub approval_patterns: Option<serde_json::Value>,
}

// ─── Conversations ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save(
    state: tauri::State<'_, DbState>,
    conversation: ConversationRow,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query(
        r#"INSERT INTO conversations (id, title, ai_session_id, project_path, sort_order, created_at)
           VALUES ($1, $2, $3, $4, $5, to_timestamp($6::double precision / 1000))
           ON CONFLICT (id) DO UPDATE SET
             title = EXCLUDED.title,
             ai_session_id = EXCLUDED.ai_session_id,
             sort_order = EXCLUDED.sort_order,
             updated_at = NOW()"#,
    )
    .bind(&conversation.id)
    .bind(&conversation.title)
    .bind(&conversation.ai_session_id)
    .bind(&conversation.project_path)
    .bind(conversation.sort_order)
    .bind(conversation.created_at as f64)
    .execute(pool)
    .await
?;
    Ok(())
}

#[tauri::command]
pub async fn conv_delete(
    state: tauri::State<'_, DbState>,
    conversation_id: String,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query("DELETE FROM conversations WHERE id = $1")
        .bind(&conversation_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn conv_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<ConversationRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<ConvListRow> =
        golish_db::repo::conversation_store::list_by_project(pool, project_path.as_deref()).await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, title, ai_session_id, project_path, sort_order, created_at)| ConversationRow {
                id,
                title,
                ai_session_id,
                project_path,
                sort_order,
                created_at: created_at.timestamp_millis(),
            },
        )
        .collect())
}

// ─── Chat Messages ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_messages(
    state: tauri::State<'_, DbState>,
    conversation_id: String,
    messages: Vec<ChatMessageRow>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;

    // Delete existing messages for this conversation and re-insert
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM chat_messages WHERE conversation_id = $1")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await?;

    for msg in &messages {
        sqlx::query(
            r#"INSERT INTO chat_messages
               (id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, tool_call_offsets, thinking_segments, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, to_timestamp($12::double precision / 1000))"#,
        )
        .bind(&msg.id)
        .bind(&conversation_id)
        .bind(&msg.role)
        .bind(&msg.content)
        .bind(&msg.thinking)
        .bind(&msg.error)
        .bind(&msg.tool_calls)
        .bind(msg.tool_calls_content_offset)
        .bind(&msg.tool_call_offsets)
        .bind(&msg.thinking_segments)
        .bind(msg.sort_order)
        .bind(msg.created_at as f64)
        .execute(&mut *tx)
        .await
?;
    }

    tx.commit().await?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_messages(
    state: tauri::State<'_, DbState>,
    conversation_id: String,
) -> Result<Vec<ChatMessageRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<i32>, Option<serde_json::Value>, Option<serde_json::Value>, i32, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, tool_call_offsets, thinking_segments, sort_order, created_at
           FROM chat_messages
           WHERE conversation_id = $1
           ORDER BY sort_order ASC, created_at ASC"#,
    )
    .bind(&conversation_id)
    .fetch_all(pool)
    .await
?;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                conversation_id,
                role,
                content,
                thinking,
                error,
                tool_calls,
                tool_calls_content_offset,
                tool_call_offsets,
                thinking_segments,
                sort_order,
                created_at,
            )| {
                ChatMessageRow {
                    id,
                    conversation_id,
                    role,
                    content,
                    thinking,
                    error,
                    tool_calls,
                    tool_calls_content_offset,
                    tool_call_offsets,
                    thinking_segments,
                    sort_order,
                    created_at: created_at.timestamp_millis(),
                }
            },
        )
        .collect())
}

// ─── Timeline Blocks ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_timeline(
    state: tauri::State<'_, DbState>,
    session_id: String,
    conversation_id: Option<String>,
    blocks: Vec<TimelineBlockRow>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM timeline_blocks WHERE session_id = $1")
        .bind(&session_id)
        .execute(&mut *tx)
        .await?;

    for block in &blocks {
        sqlx::query(
            r#"INSERT INTO timeline_blocks
               (id, session_id, conversation_id, block_type, data, batch_id, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, COALESCE($8::timestamptz, NOW()))"#,
        )
        .bind(&block.id)
        .bind(&session_id)
        .bind(conversation_id.as_deref())
        .bind(&block.block_type)
        .bind(&block.data)
        .bind(&block.batch_id)
        .bind(block.sort_order)
        .bind(&block.timestamp)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_timeline(
    state: tauri::State<'_, DbState>,
    session_id: String,
) -> Result<Vec<TimelineBlockRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String, serde_json::Value, Option<String>, i32, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, session_id, conversation_id, block_type, data, batch_id, sort_order, created_at
           FROM timeline_blocks
           WHERE session_id = $1
           ORDER BY sort_order ASC, created_at ASC"#,
    )
    .bind(&session_id)
    .fetch_all(pool)
    .await
?;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                session_id,
                conversation_id,
                block_type,
                data,
                batch_id,
                sort_order,
                created_at,
            )| {
                TimelineBlockRow {
                    id,
                    session_id,
                    conversation_id,
                    block_type,
                    data,
                    batch_id,
                    sort_order,
                    timestamp: Some(created_at.to_rfc3339()),
                }
            },
        )
        .collect())
}

// ─── Terminal State ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_terminal_state(
    state: tauri::State<'_, DbState>,
    terminal: TerminalStateRow,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let mut tx = pool.begin().await?;

    // Remove stale rows for this conversation (handles migration from
    // ephemeral PTY UUIDs to stable logical terminal IDs).
    if let Some(ref conv_id) = terminal.conversation_id {
        sqlx::query("DELETE FROM terminal_state WHERE conversation_id = $1 AND session_id != $2")
            .bind(conv_id)
            .bind(&terminal.session_id)
            .execute(&mut *tx)
            .await?;
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
    .bind(&terminal.session_id)
    .bind(&terminal.conversation_id)
    .bind(&terminal.working_directory)
    .bind(&terminal.scrollback)
    .bind(&terminal.custom_name)
    .bind(&terminal.plan_json)
    .bind(&terminal.execution_mode)
    .bind(&terminal.retired_plans_json)
    .bind(&terminal.plan_message_id)
    .execute(&mut *tx)
    .await
?;

    tx.commit().await?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_terminal_states(
    state: tauri::State<'_, DbState>,
    conversation_id: String,
) -> Result<Vec<TerminalStateRow>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, Option<String>, String, String, Option<String>, Option<serde_json::Value>, Option<String>, Option<serde_json::Value>, Option<String>)>(
        r#"SELECT session_id, conversation_id, working_directory, scrollback, custom_name, plan_json, execution_mode, retired_plans_json, plan_message_id
           FROM terminal_state
           WHERE conversation_id = $1"#,
    )
    .bind(&conversation_id)
    .fetch_all(pool)
    .await
?;

    Ok(rows
        .into_iter()
        .map(
            |(
                session_id,
                conversation_id,
                working_directory,
                scrollback,
                custom_name,
                plan_json,
                execution_mode,
                retired_plans_json,
                plan_message_id,
            )| {
                TerminalStateRow {
                    session_id,
                    conversation_id,
                    working_directory,
                    scrollback,
                    custom_name,
                    plan_json,
                    execution_mode,
                    retired_plans_json,
                    plan_message_id,
                }
            },
        )
        .collect())
}

// ─── Preferences ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_preferences(
    state: tauri::State<'_, DbState>,
    project_path: String,
    prefs: WorkspacePreferences,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
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
    .bind(&project_path)
    .bind(&prefs.active_conversation_id)
    .bind(&prefs.ai_model)
    .bind(&prefs.approval_mode)
    .bind(&prefs.approval_patterns)
    .execute(pool)
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_preferences(
    state: tauri::State<'_, DbState>,
    project_path: String,
) -> Result<Option<WorkspacePreferences>, GolishError> {
    let pool = state.pool_ready().await?;
    let row: Option<WorkspacePrefsRow> =
        golish_db::repo::conversation_store::load_preferences(pool, &project_path).await?;

    Ok(row.map(
        |(active_conversation_id, ai_model, approval_mode, approval_patterns)| {
            WorkspacePreferences {
                active_conversation_id,
                ai_model,
                approval_mode,
                approval_patterns,
            }
        },
    ))
}

pub mod batch;
pub use batch::{BatchTimelineEntry, ConvBatchItem, ConvBatchSavePayload};
