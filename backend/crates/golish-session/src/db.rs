//! PostgreSQL persistence layer for session data.
//!
//! Provides dual-write alongside the existing file-based storage,
//! and DB-backed read operations for listing/loading sessions.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{QbitMessageRole, QbitSessionMessage, QbitSessionSnapshot, SessionListingInfo};

#[derive(sqlx::FromRow)]
struct SessionDataRow {
    id: Uuid,
    title: Option<String>,
    status: String,
    workspace_path: Option<String>,
    workspace_label: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    messages: Option<serde_json::Value>,
    transcript: Option<serde_json::Value>,
    distinct_tools: Option<serde_json::Value>,
    total_messages: Option<i32>,
    sidecar_session_id: Option<String>,
    agent_mode: Option<String>,
}

pub async fn save_session_to_db(
    pool: &PgPool,
    snapshot: &QbitSessionSnapshot,
    session_uuid: &Uuid,
) -> Result<()> {
    let messages_json = serde_json::to_value(&snapshot.messages)?;
    let transcript_json = serde_json::to_value(&snapshot.transcript)?;
    let tools_json = serde_json::to_value(&snapshot.distinct_tools)?;

    sqlx::query(
        r#"INSERT INTO sessions (id, title, status, workspace_path, workspace_label, model, provider, created_at, updated_at)
           VALUES ($1, $2, 'running'::session_status, $3, $4, $5, $6, $7, NOW())
           ON CONFLICT (id) DO UPDATE SET
             status = 'running'::session_status,
             updated_at = NOW()"#,
    )
    .bind(session_uuid)
    .bind(snapshot.messages.first().and_then(|m| {
        if m.role == QbitMessageRole::User {
            let preview: String = m.content.chars().take(100).collect();
            Some(preview)
        } else {
            None
        }
    }))
    .bind(&snapshot.workspace_path)
    .bind(&snapshot.workspace_label)
    .bind(&snapshot.model)
    .bind(&snapshot.provider)
    .bind(snapshot.started_at)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"INSERT INTO session_data (session_id, messages, transcript, distinct_tools, total_messages, sidecar_session_id, agent_mode)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (session_id) DO UPDATE SET
             messages = $2, transcript = $3, distinct_tools = $4,
             total_messages = $5, sidecar_session_id = $6, agent_mode = $7"#,
    )
    .bind(session_uuid)
    .bind(&messages_json)
    .bind(&transcript_json)
    .bind(&tools_json)
    .bind(snapshot.total_messages as i32)
    .bind(&snapshot.sidecar_session_id)
    .bind(&snapshot.agent_mode)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn finalize_session_in_db(
    pool: &PgPool,
    snapshot: &QbitSessionSnapshot,
    session_uuid: &Uuid,
) -> Result<()> {
    save_session_to_db(pool, snapshot, session_uuid).await?;

    sqlx::query("UPDATE sessions SET status = 'finished'::session_status, updated_at = NOW() WHERE id = $1")
        .bind(session_uuid)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn list_sessions_from_db(
    pool: &PgPool,
    limit: usize,
) -> Result<Vec<SessionListingInfo>> {
    let limit_val = if limit == 0 { 1000i64 } else { limit as i64 };
    let rows: Vec<SessionDataRow> = sqlx::query_as(
        r#"SELECT s.id, s.title, s.status::TEXT, s.workspace_path, s.workspace_label,
                  s.model, s.provider, s.created_at, s.updated_at,
                  d.messages, d.transcript, d.distinct_tools, d.total_messages,
                  d.sidecar_session_id, d.agent_mode
           FROM sessions s
           LEFT JOIN session_data d ON d.session_id = s.id
           ORDER BY s.created_at DESC
           LIMIT $1"#,
    )
    .bind(limit_val)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| row_to_listing(r)).collect())
}

pub async fn find_session_from_db(
    pool: &PgPool,
    identifier: &str,
) -> Result<Option<SessionListingInfo>> {
    let uid = Uuid::parse_str(identifier).ok();

    let row: Option<SessionDataRow> = if let Some(uid) = uid {
        sqlx::query_as(
            r#"SELECT s.id, s.title, s.status::TEXT, s.workspace_path, s.workspace_label,
                      s.model, s.provider, s.created_at, s.updated_at,
                      d.messages, d.transcript, d.distinct_tools, d.total_messages,
                      d.sidecar_session_id, d.agent_mode
               FROM sessions s
               LEFT JOIN session_data d ON d.session_id = s.id
               WHERE s.id = $1"#,
        )
        .bind(uid)
        .fetch_optional(pool)
        .await?
    } else {
        let pattern = format!("%{}%", identifier);
        sqlx::query_as(
            r#"SELECT s.id, s.title, s.status::TEXT, s.workspace_path, s.workspace_label,
                      s.model, s.provider, s.created_at, s.updated_at,
                      d.messages, d.transcript, d.distinct_tools, d.total_messages,
                      d.sidecar_session_id, d.agent_mode
               FROM sessions s
               LEFT JOIN session_data d ON d.session_id = s.id
               WHERE s.id::TEXT LIKE $1 OR s.workspace_label LIKE $1 OR s.title LIKE $1
               LIMIT 1"#,
        )
        .bind(&pattern)
        .fetch_optional(pool)
        .await?
    };

    Ok(row.map(row_to_listing))
}

pub async fn load_session_from_db(
    pool: &PgPool,
    identifier: &str,
) -> Result<Option<QbitSessionSnapshot>> {
    let uid = Uuid::parse_str(identifier).ok();

    let row: Option<SessionDataRow> = if let Some(uid) = uid {
        sqlx::query_as(
            r#"SELECT s.id, s.title, s.status::TEXT, s.workspace_path, s.workspace_label,
                      s.model, s.provider, s.created_at, s.updated_at,
                      d.messages, d.transcript, d.distinct_tools, d.total_messages,
                      d.sidecar_session_id, d.agent_mode
               FROM sessions s
               LEFT JOIN session_data d ON d.session_id = s.id
               WHERE s.id = $1"#,
        )
        .bind(uid)
        .fetch_optional(pool)
        .await?
    } else {
        let pattern = format!("%{}%", identifier);
        sqlx::query_as(
            r#"SELECT s.id, s.title, s.status::TEXT, s.workspace_path, s.workspace_label,
                      s.model, s.provider, s.created_at, s.updated_at,
                      d.messages, d.transcript, d.distinct_tools, d.total_messages,
                      d.sidecar_session_id, d.agent_mode
               FROM sessions s
               LEFT JOIN session_data d ON d.session_id = s.id
               WHERE s.id::TEXT LIKE $1 OR s.workspace_label LIKE $1 OR s.title LIKE $1
               LIMIT 1"#,
        )
        .bind(&pattern)
        .fetch_optional(pool)
        .await?
    };

    Ok(row.map(row_to_snapshot))
}

fn row_to_listing(r: SessionDataRow) -> SessionListingInfo {
    let messages: Vec<QbitSessionMessage> = r.messages
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let first_prompt = messages.iter().find(|m| m.role == QbitMessageRole::User).map(|m| {
        m.content.chars().take(200).collect::<String>()
    });
    let first_reply = messages.iter().find(|m| m.role == QbitMessageRole::Assistant).map(|m| {
        m.content.chars().take(200).collect::<String>()
    });

    SessionListingInfo {
        identifier: r.id.to_string(),
        path: std::path::PathBuf::new(),
        workspace_label: r.workspace_label.unwrap_or_default(),
        workspace_path: r.workspace_path.unwrap_or_default(),
        model: r.model.unwrap_or_default(),
        provider: r.provider.unwrap_or_default(),
        started_at: r.created_at,
        ended_at: r.updated_at,
        total_messages: r.total_messages.unwrap_or(0) as usize,
        distinct_tools: r.distinct_tools
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        first_prompt_preview: first_prompt,
        first_reply_preview: first_reply,
        status: Some(r.status),
        title: r.title,
    }
}

fn row_to_snapshot(r: SessionDataRow) -> QbitSessionSnapshot {
    QbitSessionSnapshot {
        workspace_label: r.workspace_label.unwrap_or_default(),
        workspace_path: r.workspace_path.unwrap_or_default(),
        model: r.model.unwrap_or_default(),
        provider: r.provider.unwrap_or_default(),
        started_at: r.created_at,
        ended_at: r.updated_at,
        total_messages: r.total_messages.unwrap_or(0) as usize,
        distinct_tools: r.distinct_tools
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        transcript: r.transcript
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        messages: r.messages
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        sidecar_session_id: r.sidecar_session_id,
        total_tokens: None,
        agent_mode: r.agent_mode,
    }
}

/// Database session persistence handle.
/// Stored in QbitSessionManager for dual-write support.
pub struct DbSessionHandle {
    pub pool: Arc<PgPool>,
    pub session_uuid: Uuid,
}
