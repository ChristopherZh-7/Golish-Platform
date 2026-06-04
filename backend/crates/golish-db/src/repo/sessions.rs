use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewSession, Session, SessionStatus};

pub async fn create(pool: &PgPool, s: NewSession) -> Result<Session> {
    let row = sqlx::query_as::<_, Session>(
        r#"INSERT INTO sessions (title, workspace_path, workspace_label, model, provider, project_path)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(&s.title)
    .bind(&s.workspace_path)
    .bind(&s.workspace_label)
    .bind(&s.model)
    .bind(&s.provider)
    .bind(&s.project_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// SQL for [`upsert_by_chat_key`]. Kept as a const so its shape can be asserted
/// by a unit test without a live DB (mirrors `repo::tasks`'s SQL-string tests).
///
/// Idempotent on `chat_session_key`: first message for a chat session inserts a
/// row; subsequent messages hit the unique index and `DO UPDATE SET
/// updated_at = NOW()` (a no-op touch) so `RETURNING *` always yields the one
/// stable row for that chat session. This is the anchor that lets task mode find
/// + resume the prior operation instead of creating a new session per message.
const UPSERT_SESSION_BY_CHAT_KEY_SQL: &str = r#"INSERT INTO sessions
        (chat_session_key, title, workspace_path, workspace_label, model, provider, project_path)
     VALUES ($1, $2, $3, $4, $5, $6, $7)
     ON CONFLICT (chat_session_key) DO UPDATE SET updated_at = NOW()
     RETURNING *"#;

/// Get the session anchored to `chat_key`, creating it on first use. Returns the
/// same stable row on every call for a given `chat_key` (see
/// [`UPSERT_SESSION_BY_CHAT_KEY_SQL`]).
pub async fn upsert_by_chat_key(pool: &PgPool, chat_key: &str, s: NewSession) -> Result<Session> {
    let row = sqlx::query_as::<_, Session>(UPSERT_SESSION_BY_CHAT_KEY_SQL)
        .bind(chat_key)
        .bind(&s.title)
        .bind(&s.workspace_path)
        .bind(&s.workspace_label)
        .bind(&s.model)
        .bind(&s.provider)
        .bind(&s.project_path)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Session>> {
    super::scoped::get_by_id(pool, "sessions", id).await
}

pub async fn list(pool: &PgPool, limit: i64) -> Result<Vec<Session>> {
    let rows =
        sqlx::query_as::<_, Session>("SELECT * FROM sessions ORDER BY created_at DESC LIMIT $1")
            .bind(limit)
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: SessionStatus) -> Result<()> {
    sqlx::query("UPDATE sessions SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "sessions", id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::UPSERT_SESSION_BY_CHAT_KEY_SQL;

    /// Guard the anchor upsert SQL: it must key on `chat_session_key`, be an
    /// idempotent upsert (ON CONFLICT … DO UPDATE) so repeated chat messages map
    /// to one row, and return the row so the caller gets a stable session id.
    #[test]
    fn upsert_by_chat_key_sql_is_idempotent_upsert() {
        let sql = UPSERT_SESSION_BY_CHAT_KEY_SQL;
        assert!(sql.contains("INSERT INTO sessions"), "sql={sql}");
        assert!(
            sql.contains("chat_session_key"),
            "must key on chat anchor: {sql}"
        );
        assert!(
            sql.contains("ON CONFLICT (chat_session_key) DO UPDATE"),
            "must be an idempotent upsert on the chat anchor: {sql}"
        );
        assert!(
            sql.contains("RETURNING *"),
            "must return the stable row: {sql}"
        );
    }
}
