use crate::models::CrawlObservation;
use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CrawlObservationWrite<'a> {
    pub origin_target_id: Uuid,
    pub organization_id: Option<Uuid>,
    pub project_path: Option<&'a str>,
    pub origin_url: &'a str,
    pub origin_key: &'a str,
    pub observed_url: &'a str,
    pub observed_host: Option<&'a str>,
    pub observed_path: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub same_origin: bool,
    pub source_tool: Option<&'a str>,
    pub source_record_id: Option<&'a str>,
    pub evidence_id: Option<i64>,
    pub metadata: Option<&'a serde_json::Value>,
}

pub const UPSERT_CRAWL_OBSERVATION_SQL: &str = r#"
INSERT INTO crawl_observations
    (origin_target_id, organization_id, project_path, origin_url, origin_key,
     observed_url, observed_host, observed_path, kind, same_origin, source_tool,
     source_record_id, evidence_id, metadata)
VALUES
    ($1, $2, COALESCE($3, ''), $4, $5,
     $6, $7, $8, COALESCE($9, 'url'), $10, COALESCE($11, 'crawler'),
     $12, $13, COALESCE($14, '{}'::jsonb))
ON CONFLICT (origin_target_id, observed_url, source_tool, kind)
DO UPDATE SET
    organization_id = COALESCE(EXCLUDED.organization_id, crawl_observations.organization_id),
    project_path = CASE
        WHEN EXCLUDED.project_path <> '' THEN EXCLUDED.project_path
        ELSE crawl_observations.project_path
    END,
    origin_url = EXCLUDED.origin_url,
    origin_key = EXCLUDED.origin_key,
    observed_host = COALESCE(EXCLUDED.observed_host, crawl_observations.observed_host),
    observed_path = COALESCE(EXCLUDED.observed_path, crawl_observations.observed_path),
    same_origin = crawl_observations.same_origin OR EXCLUDED.same_origin,
    source_record_id = COALESCE(EXCLUDED.source_record_id, crawl_observations.source_record_id),
    evidence_id = COALESCE(EXCLUDED.evidence_id, crawl_observations.evidence_id),
    metadata = crawl_observations.metadata || EXCLUDED.metadata,
    updated_at = NOW()
RETURNING *
"#;

pub const LIST_FOR_ORIGIN_TARGETS_SQL: &str = r#"
SELECT *
FROM crawl_observations
WHERE origin_target_id = ANY($1::uuid[])
ORDER BY discovered_at DESC, observed_url ASC, id ASC
"#;

pub async fn upsert(pool: &PgPool, input: &CrawlObservationWrite<'_>) -> Result<CrawlObservation> {
    let empty_metadata = serde_json::json!({});
    let row = sqlx::query_as::<_, CrawlObservation>(UPSERT_CRAWL_OBSERVATION_SQL)
        .bind(input.origin_target_id)
        .bind(input.organization_id)
        .bind(input.project_path)
        .bind(input.origin_url)
        .bind(input.origin_key)
        .bind(input.observed_url)
        .bind(input.observed_host)
        .bind(input.observed_path)
        .bind(input.kind)
        .bind(input.same_origin)
        .bind(input.source_tool)
        .bind(input.source_record_id)
        .bind(input.evidence_id)
        .bind(input.metadata.unwrap_or(&empty_metadata))
        .fetch_one(pool)
        .await?;
    Ok(row)
}

pub async fn list_for_origin_targets(
    pool: &PgPool,
    target_ids: &[Uuid],
) -> Result<Vec<CrawlObservation>> {
    if target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, CrawlObservation>(LIST_FOR_ORIGIN_TARGETS_SQL)
        .bind(target_ids)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_dedupes_by_origin_url_tool_kind_without_target_promotion() {
        assert!(UPSERT_CRAWL_OBSERVATION_SQL
            .contains("ON CONFLICT (origin_target_id, observed_url, source_tool, kind)"));
        assert!(UPSERT_CRAWL_OBSERVATION_SQL.contains("metadata = crawl_observations.metadata"));
        assert!(!UPSERT_CRAWL_OBSERVATION_SQL.contains("INSERT INTO targets"));
        assert!(!UPSERT_CRAWL_OBSERVATION_SQL.contains("INSERT INTO api_endpoints"));
    }

    #[test]
    fn list_reads_only_origin_owned_observations() {
        assert!(LIST_FOR_ORIGIN_TARGETS_SQL.contains("WHERE origin_target_id = ANY($1::uuid[])"));
        assert!(LIST_FOR_ORIGIN_TARGETS_SQL.contains("ORDER BY discovered_at DESC"));
    }
}
