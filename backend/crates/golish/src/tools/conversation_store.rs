//! Tauri commands for frontend conversation & timeline persistence.
//! Replaces workspace.json read/write with PostgreSQL-backed storage.

use serde::{Deserialize, Serialize};

use crate::state::AppState;

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
    state: tauri::State<'_, AppState>,
    conversation: ConversationRow,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
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
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_delete(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query("DELETE FROM conversations WHERE id = $1")
        .bind(&conversation_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<ConversationRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, i32, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, title, ai_session_id, project_path, sort_order, created_at
           FROM conversations
           WHERE ($1::text IS NULL OR project_path = $1)
           ORDER BY sort_order ASC, created_at ASC"#,
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, title, ai_session_id, project_path, sort_order, created_at)| {
            ConversationRow {
                id,
                title,
                ai_session_id,
                project_path,
                sort_order,
                created_at: created_at.timestamp_millis(),
            }
        })
        .collect())
}

// ─── Chat Messages ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_messages(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    messages: Vec<ChatMessageRow>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;

    // Delete existing messages for this conversation and re-insert
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM chat_messages WHERE conversation_id = $1")
        .bind(&conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    for msg in &messages {
        sqlx::query(
            r#"INSERT INTO chat_messages
               (id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, to_timestamp($10::double precision / 1000))"#,
        )
        .bind(&msg.id)
        .bind(&conversation_id)
        .bind(&msg.role)
        .bind(&msg.content)
        .bind(&msg.thinking)
        .bind(&msg.error)
        .bind(&msg.tool_calls)
        .bind(msg.tool_calls_content_offset)
        .bind(msg.sort_order)
        .bind(msg.created_at as f64)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_messages(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<ChatMessageRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<i32>, i32, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, sort_order, created_at
           FROM chat_messages
           WHERE conversation_id = $1
           ORDER BY sort_order ASC, created_at ASC"#,
    )
    .bind(&conversation_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, conversation_id, role, content, thinking, error, tool_calls, tool_calls_content_offset, sort_order, created_at)| {
            ChatMessageRow {
                id,
                conversation_id,
                role,
                content,
                thinking,
                error,
                tool_calls,
                tool_calls_content_offset,
                sort_order,
                created_at: created_at.timestamp_millis(),
            }
        })
        .collect())
}

// ─── Timeline Blocks ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_timeline(
    state: tauri::State<'_, AppState>,
    session_id: String,
    conversation_id: Option<String>,
    blocks: Vec<TimelineBlockRow>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM timeline_blocks WHERE session_id = $1")
        .bind(&session_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

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
        .await
        .map_err(|e| e.to_string())?;
    }

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_timeline(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Vec<TimelineBlockRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, String, Option<String>, String, serde_json::Value, Option<String>, i32, chrono::DateTime<chrono::Utc>)>(
        r#"SELECT id, session_id, conversation_id, block_type, data, batch_id, sort_order, created_at
           FROM timeline_blocks
           WHERE session_id = $1
           ORDER BY sort_order ASC, created_at ASC"#,
    )
    .bind(&session_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(id, session_id, conversation_id, block_type, data, batch_id, sort_order, created_at)| {
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
        })
        .collect())
}

// ─── Terminal State ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_terminal_state(
    state: tauri::State<'_, AppState>,
    terminal: TerminalStateRow,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    sqlx::query(
        r#"INSERT INTO terminal_state (session_id, conversation_id, working_directory, scrollback, custom_name, plan_json)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (session_id) DO UPDATE SET
             working_directory = EXCLUDED.working_directory,
             scrollback = EXCLUDED.scrollback,
             custom_name = EXCLUDED.custom_name,
             plan_json = EXCLUDED.plan_json,
             updated_at = NOW()"#,
    )
    .bind(&terminal.session_id)
    .bind(&terminal.conversation_id)
    .bind(&terminal.working_directory)
    .bind(&terminal.scrollback)
    .bind(&terminal.custom_name)
    .bind(&terminal.plan_json)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_terminal_states(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<TerminalStateRow>, String> {
    let pool = state.db_pool_ready().await?;
    let rows = sqlx::query_as::<_, (String, Option<String>, String, String, Option<String>, Option<serde_json::Value>)>(
        r#"SELECT session_id, conversation_id, working_directory, scrollback, custom_name, plan_json
           FROM terminal_state
           WHERE conversation_id = $1"#,
    )
    .bind(&conversation_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(session_id, conversation_id, working_directory, scrollback, custom_name, plan_json)| {
            TerminalStateRow {
                session_id,
                conversation_id,
                working_directory,
                scrollback,
                custom_name,
                plan_json,
            }
        })
        .collect())
}

// ─── Workspace Preferences ───────────────────────────────────────────────────

#[tauri::command]
pub async fn conv_save_preferences(
    state: tauri::State<'_, AppState>,
    project_path: String,
    prefs: WorkspacePreferences,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
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
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn conv_load_preferences(
    state: tauri::State<'_, AppState>,
    project_path: String,
) -> Result<Option<WorkspacePreferences>, String> {
    let pool = state.db_pool_ready().await?;
    let row = sqlx::query_as::<_, (Option<String>, Option<serde_json::Value>, Option<String>, Option<serde_json::Value>)>(
        r#"SELECT active_conversation_id, ai_model, approval_mode, approval_patterns
           FROM workspace_preferences
           WHERE project_path = $1"#,
    )
    .bind(&project_path)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|(active_conversation_id, ai_model, approval_mode, approval_patterns)| {
        WorkspacePreferences {
            active_conversation_id,
            ai_model,
            approval_mode,
            approval_patterns,
        }
    }))
}
