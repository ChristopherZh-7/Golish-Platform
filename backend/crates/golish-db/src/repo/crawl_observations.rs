use crate::models::CrawlObservation;
use crate::repo::scoped::{lock_target_write_guard, TargetWriteGuard};
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
WHERE crawl_observations.organization_id IS NOT DISTINCT FROM EXCLUDED.organization_id
  AND crawl_observations.project_path = EXCLUDED.project_path
RETURNING *
"#;

pub const LIST_FOR_ORIGIN_TARGETS_SQL: &str = r#"
SELECT *
FROM crawl_observations
WHERE origin_target_id = ANY($1::uuid[])
ORDER BY discovered_at DESC, observed_url ASC, id ASC
"#;

fn build_list_for_current_target_owners_sql() -> &'static str {
    r#"SELECT observation.*
       FROM crawl_observations observation
       JOIN targets t ON t.id = observation.origin_target_id
       WHERE observation.origin_target_id = ANY($1::uuid[])
         AND t.scope::text = 'in'
         AND observation.organization_id IS NOT DISTINCT FROM t.organization_id
         AND observation.project_path IS NOT DISTINCT FROM COALESCE(t.project_path, '')
       ORDER BY observation.discovered_at DESC,
                observation.observed_url ASC,
                observation.id ASC"#
}

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

fn guard_owns_input(guard: &TargetWriteGuard, input: &CrawlObservationWrite<'_>) -> bool {
    guard.organization_id.is_some()
        && input.origin_target_id == guard.target_id
        && input.organization_id == guard.organization_id
        && input.project_path == Some(guard.project_path.as_str())
}

/// Upsert a crawler observation only while the owning origin target still
/// matches the producer's exact authorization snapshot.
///
/// This is deliberately a short database-only transaction: the target row is
/// locked and compared before the child write, and no network work happens
/// while the lock is held. The input owner fields must exactly match the guard;
/// an existing row bound to another org/project makes the upsert return no row
/// and is rejected rather than reassigned.
pub async fn upsert_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    input: &CrawlObservationWrite<'_>,
) -> Result<CrawlObservation> {
    if !guard_owns_input(guard, input) {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "guarded crawl observation owner does not match target authorization"
        )));
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;
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
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            crate::DbError::NotFound(format!(
                "guarded crawl observation conflict has a different owner for target {}",
                guard.target_id
            ))
        })?;
    tx.commit().await?;
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

/// Read crawler observations only while their persisted org/project owner still
/// matches the current in-scope origin target. Historical rows are preserved
/// but do not follow a moved target into another engagement/workspace.
pub async fn list_for_current_target_owners(
    pool: &PgPool,
    target_ids: &[Uuid],
) -> Result<Vec<CrawlObservation>> {
    if target_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, CrawlObservation>(build_list_for_current_target_owners_sql())
        .bind(target_ids)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> TargetWriteGuard {
        TargetWriteGuard {
            target_id: Uuid::new_v4(),
            organization_id: Some(Uuid::new_v4()),
            project_path: "/workspace".to_string(),
            scope: "in".to_string(),
            name: "https://app.example/".to_string(),
            value: "https://app.example/".to_string(),
            ports: serde_json::json!([]),
        }
    }

    #[test]
    fn upsert_dedupes_by_origin_url_tool_kind_without_target_promotion() {
        assert!(UPSERT_CRAWL_OBSERVATION_SQL
            .contains("ON CONFLICT (origin_target_id, observed_url, source_tool, kind)"));
        assert!(UPSERT_CRAWL_OBSERVATION_SQL.contains("metadata = crawl_observations.metadata"));
        assert!(UPSERT_CRAWL_OBSERVATION_SQL.contains(
            "WHERE crawl_observations.organization_id IS NOT DISTINCT FROM EXCLUDED.organization_id"
        ));
        assert!(UPSERT_CRAWL_OBSERVATION_SQL
            .contains("AND crawl_observations.project_path = EXCLUDED.project_path"));
        assert!(
            !UPSERT_CRAWL_OBSERVATION_SQL.contains("crawl_observations.organization_id IS NULL")
        );
        assert!(!UPSERT_CRAWL_OBSERVATION_SQL.contains("INSERT INTO targets"));
        assert!(!UPSERT_CRAWL_OBSERVATION_SQL.contains("INSERT INTO api_endpoints"));
    }

    #[test]
    fn list_reads_only_origin_owned_observations() {
        assert!(LIST_FOR_ORIGIN_TARGETS_SQL.contains("WHERE origin_target_id = ANY($1::uuid[])"));
        assert!(LIST_FOR_ORIGIN_TARGETS_SQL.contains("ORDER BY discovered_at DESC"));
    }

    #[test]
    fn current_owner_list_checks_scope_and_project() {
        let sql = build_list_for_current_target_owners_sql();
        assert!(sql.contains("JOIN targets t ON t.id = observation.origin_target_id"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("observation.organization_id IS NOT DISTINCT FROM t.organization_id"));
        assert!(sql.contains(
            "observation.project_path IS NOT DISTINCT FROM COALESCE(t.project_path, '')"
        ));
    }

    #[test]
    fn guarded_input_requires_exact_target_org_and_project() {
        let guard = guard();
        let input = CrawlObservationWrite {
            origin_target_id: guard.target_id,
            organization_id: guard.organization_id,
            project_path: Some(&guard.project_path),
            origin_url: "https://app.example:443",
            origin_key: "https://app.example:443",
            observed_url: "https://app.example/a",
            observed_host: Some("app.example"),
            observed_path: Some("/a"),
            kind: Some("url"),
            same_origin: true,
            source_tool: Some("katana"),
            source_record_id: None,
            evidence_id: None,
            metadata: None,
        };

        assert!(guard_owns_input(&guard, &input));

        let foreign_guard = TargetWriteGuard {
            organization_id: Some(Uuid::new_v4()),
            ..guard.clone()
        };
        assert!(!guard_owns_input(&foreign_guard, &input));

        let unowned_guard = TargetWriteGuard {
            organization_id: None,
            ..guard.clone()
        };
        assert!(!guard_owns_input(&unowned_guard, &input));
    }
}
