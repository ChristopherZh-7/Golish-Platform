//! Endpoint test history (per-API-endpoint security probes).
//!
//! Records each authentication / IDOR / injection / fuzz attempt against a
//! discovered API endpoint, with payload, evidence and severity. Used by the
//! AI agent and the Endpoint Inspector UI to render a per-endpoint test
//! timeline and cross-target stats.

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::EndpointTest;

/// Insert a single endpoint test row and return the generated row id.
#[allow(clippy::too_many_arguments)]
pub async fn record_endpoint_test(
    pool: &PgPool,
    endpoint_id: Option<Uuid>,
    target_id: Option<Uuid>,
    test_type: &str,
    tool_used: Option<&str>,
    payload: Option<&str>,
    result: Option<&str>,
    severity: Option<&str>,
    evidence: Option<&str>,
    detail: &serde_json::Value,
) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO endpoint_tests
               (endpoint_id, target_id, test_type, tool_used, payload,
                result, severity, evidence, detail)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
    )
    .bind(endpoint_id)
    .bind(target_id)
    .bind(test_type)
    .bind(tool_used)
    .bind(payload)
    .bind(result)
    .bind(severity)
    .bind(evidence)
    .bind(detail)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List the test history for a single endpoint, newest first.
pub async fn list_endpoint_tests(
    pool: &PgPool,
    endpoint_id: Uuid,
) -> Result<Vec<EndpointTest>> {
    let rows = sqlx::query_as::<_, EndpointTest>(
        r#"SELECT id, endpoint_id, target_id, test_type, tool_used, payload,
                  result, severity, evidence, detail, tested_at
           FROM endpoint_tests
           WHERE endpoint_id = $1
           ORDER BY tested_at DESC"#,
    )
    .bind(endpoint_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List endpoint tests scoped to a target (across all its endpoints).
pub async fn list_by_target(
    pool: &PgPool,
    target_id: Uuid,
    limit: i64,
) -> Result<Vec<EndpointTest>> {
    let rows = sqlx::query_as::<_, EndpointTest>(
        r#"SELECT id, endpoint_id, target_id, test_type, tool_used, payload,
                  result, severity, evidence, detail, tested_at
           FROM endpoint_tests
           WHERE target_id = $1
           ORDER BY tested_at DESC
           LIMIT $2"#,
    )
    .bind(target_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Aggregate counts grouped by test_type for a given target.
pub async fn endpoint_test_stats(
    pool: &PgPool,
    target_id: Uuid,
) -> Result<Vec<(String, i64)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT test_type, COUNT(*) AS count
           FROM endpoint_tests
           WHERE target_id = $1
           GROUP BY test_type
           ORDER BY count DESC"#,
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Total number of tests for a target.
pub async fn count_by_target(pool: &PgPool, target_id: Uuid) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM endpoint_tests WHERE target_id = $1",
    )
    .bind(target_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
