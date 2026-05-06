use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::AuditEntry;

pub async fn log(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    project_path: Option<&str>,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO audit_log (action, category, details, entity_type, entity_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(entity_type)
    .bind(entity_id)
    .bind(project_path)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}

/// Extended log with pentest operation fields
pub async fn log_operation(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    status: &str,
    detail: &serde_json::Value,
) -> Result<AuditEntry> {
    log_operation_with_lineage(
        pool, action, category, details, project_path, source,
        target_id, session_id, tool_name, status, detail, None, None,
    )
    .await
}

/// Internal helper that supports parent_id (self-ref) + run_id (correlation UUID).
/// All audit_log writers ultimately route through this single SQL.
#[allow(clippy::too_many_arguments)]
pub async fn log_operation_with_lineage(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    status: &str,
    detail: &serde_json::Value,
    parent_id: Option<i64>,
    run_id: Option<Uuid>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                parent_id, run_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(project_path)
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(status)
    .bind(detail)
    .bind(parent_id)
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// `PentestAudit` writes started→completed/failed pairs into `audit_log`,
/// linking children to parents via `parent_id` and correlating all rows from
/// the same logical run via `run_id`.
///
/// Conventions:
/// - `started` allocates a new `run_id` (UUID v4) and returns the new row id
///   so callers can pass it as `parent_id` to `completed`/`failed`.
/// - `completed`/`failed` resolve the parent's `run_id` via SELECT, so the
///   caller never has to thread the run id through their own state.
/// - Every `detail` JSON gets `schema_v: 1` injected (when the input is an
///   object) for forward-compatible parsing.
/// - All writes are routed through `log_operation_with_lineage`, so the
///   table-level invariants and indices remain centralised.
pub struct PentestAudit;

const PENTEST_AUDIT_SOURCE: &str = "pentest_bridge";

impl PentestAudit {
    /// Insert a `*_started` row, allocate a new `run_id`, and return the new
    /// audit_log row id (to be used as `parent_id` of the eventual
    /// `completed`/`failed` row).
    #[allow(clippy::too_many_arguments)]
    pub async fn started(
        pool: &PgPool,
        action: &str,
        category: &str,
        details: &str,
        target_id: Option<Uuid>,
        tool_name: Option<&str>,
        detail_json: Value,
    ) -> Result<i64> {
        let run_id = Uuid::new_v4();
        let detail = ensure_schema_v(detail_json);
        let row = log_operation_with_lineage(
            pool,
            action,
            category,
            details,
            None,
            PENTEST_AUDIT_SOURCE,
            target_id,
            None,
            tool_name,
            "started",
            &detail,
            None,
            Some(run_id),
        )
        .await?;
        Ok(row.id)
    }

    /// Insert a `*_completed` row that links back to a previous `started` row
    /// via `parent_id` and reuses its `run_id`.
    #[allow(clippy::too_many_arguments)]
    pub async fn completed(
        pool: &PgPool,
        parent_id: i64,
        action: &str,
        category: &str,
        details: &str,
        target_id: Option<Uuid>,
        tool_name: Option<&str>,
        detail_json: Value,
    ) -> Result<()> {
        let run_id = lookup_run_id(pool, parent_id).await?;
        let detail = ensure_schema_v(detail_json);
        log_operation_with_lineage(
            pool,
            action,
            category,
            details,
            None,
            PENTEST_AUDIT_SOURCE,
            target_id,
            None,
            tool_name,
            "completed",
            &detail,
            Some(parent_id),
            run_id,
        )
        .await?;
        Ok(())
    }

    /// Insert a `*_failed` row linked to the original `started` row.
    #[allow(clippy::too_many_arguments)]
    pub async fn failed(
        pool: &PgPool,
        parent_id: i64,
        action: &str,
        category: &str,
        error: &str,
        target_id: Option<Uuid>,
        tool_name: Option<&str>,
        detail_json: Value,
    ) -> Result<()> {
        let run_id = lookup_run_id(pool, parent_id).await?;
        let mut detail = ensure_schema_v(detail_json);
        if let Some(obj) = detail.as_object_mut() {
            obj.entry("error".to_string())
                .or_insert_with(|| Value::String(error.to_string()));
        }
        log_operation_with_lineage(
            pool,
            action,
            category,
            error,
            None,
            PENTEST_AUDIT_SOURCE,
            target_id,
            None,
            tool_name,
            "failed",
            &detail,
            Some(parent_id),
            run_id,
        )
        .await?;
        Ok(())
    }
}

fn ensure_schema_v(value: Value) -> Value {
    match value {
        Value::Object(mut map) => {
            map.entry("schema_v".to_string())
                .or_insert_with(|| json!(1));
            Value::Object(map)
        }
        Value::Null => json!({"schema_v": 1}),
        other => json!({"schema_v": 1, "value": other}),
    }
}

async fn lookup_run_id(pool: &PgPool, parent_id: i64) -> Result<Option<Uuid>> {
    let run_id: Option<Option<Uuid>> =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT run_id FROM audit_log WHERE id = $1")
            .bind(parent_id)
            .fetch_optional(pool)
            .await?;
    Ok(run_id.flatten())
}

impl PentestAudit {
    /// Look up a previously-inserted `started` row by matching a key/value
    /// inside its `detail` JSONB. Used by fire-and-forget async pipelines
    /// (e.g. ZAP active scan / spider) where `started` and `completed` are
    /// emitted from different functions and we only have an external
    /// correlation id (e.g. ZAP's scan_id) to bridge them.
    ///
    /// Returns the most recent matching row's `id` (newest wins) or `None`.
    pub async fn lookup_parent_by_detail_kv(
        pool: &PgPool,
        started_action: &str,
        detail_key: &str,
        detail_value: &str,
    ) -> Result<Option<i64>> {
        let id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM audit_log
               WHERE action = $1
                 AND status = 'started'
                 AND detail ->> $2 = $3
               ORDER BY id DESC
               LIMIT 1"#,
        )
        .bind(started_action)
        .bind(detail_key)
        .bind(detail_value)
        .fetch_optional(pool)
        .await?;
        Ok(id)
    }
}

pub async fn list(pool: &PgPool, project_path: Option<&str>, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_category(
    pool: &PgPool,
    category: &str,
    project_path: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE category = $1
             AND ($2 IS NULL OR project_path = $2 OR project_path IS NULL)
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(category)
    .bind(project_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_target(pool: &PgPool, target_id: Uuid, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE target_id = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(target_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_by_session(pool: &PgPool, session_id: &str, limit: i64) -> Result<Vec<AuditEntry>> {
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE session_id = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn search(
    pool: &PgPool,
    project_path: Option<&str>,
    query: &str,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let pattern = format!("%{}%", query.to_lowercase());
    let rows = sqlx::query_as::<_, AuditEntry>(
        r#"SELECT * FROM audit_log
           WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)
             AND (LOWER(action) LIKE $2 OR LOWER(details) LIKE $2
                  OR LOWER(category) LIKE $2 OR LOWER(COALESCE(tool_name, '')) LIKE $2)
           ORDER BY created_at DESC LIMIT $3"#,
    )
    .bind(project_path)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count(pool: &PgPool, project_path: Option<&str>) -> Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)")
            .bind(project_path)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn clear(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let result = sqlx::query("DELETE FROM audit_log WHERE ($1 IS NULL OR project_path = $1 OR project_path IS NULL)")
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ─── Cross-table timeline ──────────────────────────────────────────────

/// One row of the per-target activity timeline. Aggregates events from
/// `audit_log`, `target_assets`, `api_endpoints`, `passive_scan_logs`, and
/// `findings` into a unified shape ordered by `created_at DESC`.
///
/// All five SELECT branches in `target_timeline` produce the same column
/// types so the UNION ALL is well-typed (`tool_name` is always nullable
/// `text`, `detail` is always `jsonb`, etc.). Empty `tool` / `tool_used` /
/// `source` fall back to `NULL` via `NULLIF` so consumers can rely on
/// `tool_name IS NULL` rather than empty-string sentinels.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Origin table label (`audit_log` / `target_assets` / `api_endpoints` /
    /// `passive_scan_logs` / `findings`).
    pub source: String,
    /// Event keyword (e.g. `target_added`, `asset_discovered`,
    /// `endpoint_discovered`, `xss`, `finding_recorded`).
    pub event: String,
    /// Bucket category (`scan`, `targets`, `api`, severity level, ...).
    pub category: String,
    /// Human-readable summary line.
    pub details: String,
    /// Optional tool name when the event came from a tool execution.
    pub tool_name: Option<String>,
    /// Status / verdict (`completed`, `vulnerable`, `tested`, `open`, ...).
    pub status: String,
    /// Source-specific JSON payload (audit detail / asset metadata /
    /// endpoint headers / passive scan detail / finding evidence).
    pub detail: Value,
    /// Event time, used for the ORDER BY clause.
    pub created_at: DateTime<Utc>,
}

/// Aggregate every event tied to `target_id` from the five recon / scan
/// tables and return the most recent `limit` rows newest-first. The query
/// is intentionally `UNION ALL` (not `UNION DISTINCT`) — duplicates across
/// sources are real and meaningful for the timeline.
pub async fn target_timeline(
    pool: &PgPool,
    target_id: Uuid,
    limit: i64,
) -> Result<Vec<TimelineEntry>> {
    let rows = sqlx::query_as::<_, TimelineEntry>(
        r#"
        SELECT
            'audit_log'::text          AS source,
            action                     AS event,
            category                   AS category,
            details                    AS details,
            tool_name                  AS tool_name,
            status                     AS status,
            detail                     AS detail,
            created_at                 AS created_at
        FROM audit_log
        WHERE target_id = $1

        UNION ALL

        SELECT
            'target_assets'::text      AS source,
            'asset_discovered'::text   AS event,
            asset_type                 AS category,
            CASE
                WHEN port IS NOT NULL THEN value || ':' || port::text
                ELSE value
            END                        AS details,
            NULL::text                 AS tool_name,
            status                     AS status,
            metadata                   AS detail,
            discovered_at              AS created_at
        FROM target_assets
        WHERE target_id = $1

        UNION ALL

        SELECT
            'api_endpoints'::text      AS source,
            'endpoint_discovered'::text AS event,
            'api'::text                AS category,
            method || ' ' || url       AS details,
            NULLIF(source, '')         AS tool_name,
            CASE WHEN tested THEN 'tested'::text ELSE 'pending'::text END AS status,
            headers                    AS detail,
            discovered_at              AS created_at
        FROM api_endpoints
        WHERE target_id = $1

        UNION ALL

        SELECT
            'passive_scan_logs'::text  AS source,
            test_type                  AS event,
            'scan'::text               AS category,
            CASE
                WHEN payload <> '' THEN payload || ' on ' || url
                ELSE url
            END                        AS details,
            NULLIF(tool_used, '')      AS tool_name,
            result                     AS status,
            detail                     AS detail,
            tested_at                  AS created_at
        FROM passive_scan_logs
        WHERE target_id = $1

        UNION ALL

        SELECT
            'findings'::text           AS source,
            'finding_recorded'::text   AS event,
            sev::text                  AS category,
            title                      AS details,
            NULLIF(tool, '')           AS tool_name,
            status::text               AS status,
            evidence                   AS detail,
            created_at                 AS created_at
        FROM findings
        WHERE target_id = $1

        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(target_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
