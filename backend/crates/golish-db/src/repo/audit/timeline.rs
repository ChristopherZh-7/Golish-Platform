//! Cross-table per-target activity timeline. Moved verbatim from the original
//! `audit.rs`; the `UNION ALL` aggregates `audit_log` + four recon/scan tables
//! into one `TimelineEntry` shape ordered newest-first.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;

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
