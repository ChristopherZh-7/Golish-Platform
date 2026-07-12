//! Read-only query layer for first-class surface identity rows.
//!
//! Phase 2.3A scope:
//! - read only `network_endpoints`, `web_origins`, and
//!   `web_origin_observations`;
//! - never mutate legacy rows;
//! - never infer/create observations at query time;
//! - keep WebOrigin <-> NetworkEndpoint many-to-many through observations.

use std::net::IpAddr;

use crate::models::{NetworkEndpoint, WebOrigin, WebOriginObservation};
use crate::repo::surface_identity::normalize_network_endpoint;
use crate::Result;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceIdentityQueryScope {
    pub organization_id: Option<Uuid>,
    pub project_path: Option<String>,
    pub ip: Option<String>,
    pub host: Option<String>,
    pub endpoint_id: Option<Uuid>,
    pub network_endpoint_id: Option<Uuid>,
    pub web_origin_id: Option<Uuid>,
    pub target_id: Option<Uuid>,
    #[serde(default)]
    pub include_history: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceIdentitySnapshotQuery {
    pub organization_id: Option<Uuid>,
    pub project_path: Option<String>,
    pub ip: String,
    #[serde(default)]
    pub include_history: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct EasRequiredWebOriginRow {
    pub target_id: Uuid,
    pub origin: String,
    pub target_name: String,
    pub target_value: String,
    pub target_ports: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceIdentitySnapshot {
    pub endpoints: Vec<NetworkEndpoint>,
    pub web_origins: Vec<WebOrigin>,
    pub observations: Vec<WebOriginObservation>,
    pub summary: SurfaceIdentitySnapshotSummary,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceIdentitySnapshotSummary {
    pub endpoint_count: u64,
    pub web_origin_count: u64,
    pub observation_count: u64,
    pub inferred_observation_count: u64,
    pub confirmed_observation_count: u64,
}

pub const LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL: &str = r#"
SELECT DISTINCT ne.*
FROM network_endpoints ne
WHERE (
    ($1::uuid IS NOT NULL AND ne.organization_id = $1)
    OR ($1::uuid IS NULL AND ne.organization_id IS NULL AND ne.project_path = $2)
)
  AND ($3::text IS NULL OR ne.ip = $3)
  AND ($4::uuid IS NULL OR EXISTS (
      SELECT 1
      FROM web_origin_observations woo
      WHERE woo.network_endpoint_id = ne.id
        AND woo.target_id = $4
        AND (
            ($1::uuid IS NOT NULL AND woo.organization_id = $1)
            OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND woo.project_path = $2)
        )
  ))
  AND ($5::text IS NULL OR EXISTS (
      SELECT 1
      FROM web_origin_observations woo
      JOIN web_origins wo ON wo.id = woo.web_origin_id
      WHERE woo.network_endpoint_id = ne.id
        AND wo.host = $5
        AND (
            ($1::uuid IS NOT NULL AND woo.organization_id = $1 AND wo.organization_id = $1)
            OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND wo.organization_id IS NULL
                AND woo.project_path = $2 AND wo.project_path = $2)
        )
  ))
ORDER BY ne.ip ASC, ne.port ASC, ne.transport ASC
"#;

pub const LIST_WEB_ORIGINS_FOR_SCOPE_SQL: &str = r#"
SELECT DISTINCT wo.*
FROM web_origins wo
WHERE (
    ($1::uuid IS NOT NULL AND wo.organization_id = $1)
    OR ($1::uuid IS NULL AND wo.organization_id IS NULL AND wo.project_path = $2)
)
  AND ($3::text IS NULL OR wo.host = $3)
  AND ($4::uuid IS NULL OR EXISTS (
      SELECT 1
      FROM web_origin_observations woo
      WHERE woo.web_origin_id = wo.id
        AND woo.network_endpoint_id = $4
        AND (
            ($1::uuid IS NOT NULL AND woo.organization_id = $1)
            OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND woo.project_path = $2)
        )
  ))
  AND ($5::text IS NULL OR wo.host = $5 OR EXISTS (
      SELECT 1
      FROM web_origin_observations woo
      JOIN network_endpoints ne ON ne.id = woo.network_endpoint_id
      WHERE woo.web_origin_id = wo.id
        AND ne.ip = $5
        AND (
            ($1::uuid IS NOT NULL AND woo.organization_id = $1 AND ne.organization_id = $1)
            OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND ne.organization_id IS NULL
                AND woo.project_path = $2 AND ne.project_path = $2)
        )
  ))
  AND ($6::uuid IS NULL OR EXISTS (
      SELECT 1
      FROM web_origin_observations woo
      WHERE woo.web_origin_id = wo.id
        AND woo.target_id = $6
        AND (
            ($1::uuid IS NOT NULL AND woo.organization_id = $1)
            OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND woo.project_path = $2)
        )
  ))
ORDER BY wo.host ASC, wo.scheme ASC, wo.port ASC
"#;

pub const LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL: &str = r#"
SELECT *
FROM (
    SELECT DISTINCT ON (woo.web_origin_id, woo.network_endpoint_id)
        woo.*
    FROM web_origin_observations woo
    JOIN web_origins wo ON wo.id = woo.web_origin_id
    LEFT JOIN network_endpoints ne ON ne.id = woo.network_endpoint_id
    WHERE (
        ($1::uuid IS NOT NULL AND woo.organization_id = $1)
        OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND woo.project_path = $2)
    )
      AND ($3::uuid IS NULL OR woo.web_origin_id = $3)
      AND ($4::uuid IS NULL OR woo.network_endpoint_id = $4)
      AND ($5::uuid IS NULL OR woo.target_id = $5)
      AND ($6::text IS NULL OR ne.ip = $6 OR woo.observed_ip = $6)
      AND ($7::text IS NULL OR wo.host = $7)
    ORDER BY
        woo.web_origin_id ASC,
        woo.network_endpoint_id ASC NULLS LAST,
        woo.observed_at DESC,
        woo.created_at DESC,
        woo.id ASC
) latest
ORDER BY
    web_origin_id ASC,
    network_endpoint_id ASC NULLS LAST,
    observed_at DESC,
    created_at DESC,
    id ASC
"#;

pub const LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL: &str = r#"
SELECT woo.*
FROM web_origin_observations woo
JOIN web_origins wo ON wo.id = woo.web_origin_id
LEFT JOIN network_endpoints ne ON ne.id = woo.network_endpoint_id
WHERE (
    ($1::uuid IS NOT NULL AND woo.organization_id = $1)
    OR ($1::uuid IS NULL AND woo.organization_id IS NULL AND woo.project_path = $2)
)
  AND ($3::uuid IS NULL OR woo.web_origin_id = $3)
  AND ($4::uuid IS NULL OR woo.network_endpoint_id = $4)
  AND ($5::uuid IS NULL OR woo.target_id = $5)
  AND ($6::text IS NULL OR ne.ip = $6 OR woo.observed_ip = $6)
  AND ($7::text IS NULL OR wo.host = $7)
ORDER BY
    woo.web_origin_id ASC,
    woo.network_endpoint_id ASC NULLS LAST,
    woo.observed_at DESC,
    woo.created_at DESC,
    woo.id ASC
"#;

/// Exact HTTP(S) origins that the current EAS run has actively confirmed for
/// an in-scope target. Observations are relation facts, so the target/org joins
/// are mandatory; a project-only or unbound historical row never expands the
/// gate denominator. `$3 = NULL` means the live org axis, while an explicit
/// (including empty) UUID array freezes the query to the current asset wave.
pub const LIST_EAS_REQUIRED_WEB_ORIGINS_SQL: &str = r#"
SELECT DISTINCT woo.target_id, wo.origin,
       t.name AS target_name, t.value AS target_value,
       COALESCE(t.ports, '[]'::jsonb) AS target_ports
FROM web_origin_observations woo
JOIN web_origins wo ON wo.id = woo.web_origin_id
JOIN targets t ON woo.target_id = t.id
WHERE woo.organization_id = $1
  AND wo.organization_id = $1
  AND t.organization_id = $1
  AND t.scope::text = 'in'
  AND t.project_path IS NOT NULL
  AND t.project_path <> ''
  AND woo.project_path = t.project_path
  AND wo.project_path = t.project_path
  AND t.created_at <= $2
  AND woo.observed_at >= $2
  AND ($3::uuid[] IS NULL OR woo.target_id = ANY($3))
  AND LOWER(woo.source) IN ('httpx', 'nmap', 'eas_probe_http_liveness')
ORDER BY woo.target_id, wo.origin
"#;

pub async fn list_network_endpoints_for_scope(
    pool: &PgPool,
    scope: SurfaceIdentityQueryScope,
) -> Result<Vec<NetworkEndpoint>> {
    let project_path = scoped_project_path(scope.organization_id, scope.project_path.as_deref())?;
    let ip = normalize_optional_ip(scope.ip.as_deref())?;
    let host = normalize_optional_host(scope.host.as_deref())?;

    let rows = sqlx::query_as::<_, NetworkEndpoint>(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL)
        .bind(scope.organization_id)
        .bind(project_path.as_deref())
        .bind(ip.as_deref())
        .bind(scope.target_id)
        .bind(host.as_deref())
        .fetch_all(pool)
        .await?;

    Ok(rows)
}

pub async fn list_web_origins_for_scope(
    pool: &PgPool,
    scope: SurfaceIdentityQueryScope,
) -> Result<Vec<WebOrigin>> {
    let project_path = scoped_project_path(scope.organization_id, scope.project_path.as_deref())?;
    let host = normalize_optional_host(scope.host.as_deref())?;
    let ip = normalize_optional_ip(scope.ip.as_deref())?;
    let endpoint_id = scope.endpoint_id.or(scope.network_endpoint_id);

    let rows = sqlx::query_as::<_, WebOrigin>(LIST_WEB_ORIGINS_FOR_SCOPE_SQL)
        .bind(scope.organization_id)
        .bind(project_path.as_deref())
        .bind(host.as_deref())
        .bind(endpoint_id)
        .bind(ip.as_deref())
        .bind(scope.target_id)
        .fetch_all(pool)
        .await?;

    Ok(rows)
}

pub async fn list_observations_for_scope(
    pool: &PgPool,
    scope: SurfaceIdentityQueryScope,
) -> Result<Vec<WebOriginObservation>> {
    let project_path = scoped_project_path(scope.organization_id, scope.project_path.as_deref())?;
    let ip = normalize_optional_ip(scope.ip.as_deref())?;
    let host = normalize_optional_host(scope.host.as_deref())?;
    let endpoint_id = scope.endpoint_id.or(scope.network_endpoint_id);
    let sql = if scope.include_history {
        LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL
    } else {
        LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL
    };

    let rows = sqlx::query_as::<_, WebOriginObservation>(sql)
        .bind(scope.organization_id)
        .bind(project_path.as_deref())
        .bind(scope.web_origin_id)
        .bind(endpoint_id)
        .bind(scope.target_id)
        .bind(ip.as_deref())
        .bind(host.as_deref())
        .fetch_all(pool)
        .await?;

    Ok(rows)
}

pub async fn list_eas_required_web_origins(
    pool: &PgPool,
    organization_id: Uuid,
    since: chrono::DateTime<chrono::Utc>,
    current_wave_target_ids: Option<&[Uuid]>,
) -> Result<Vec<String>> {
    let rows =
        list_eas_required_web_origin_rows(pool, organization_id, since, current_wave_target_ids)
            .await?;
    let mut origins = rows.into_iter().map(|row| row.origin).collect::<Vec<_>>();
    origins.sort();
    origins.dedup();
    Ok(origins)
}

pub async fn list_eas_required_web_origin_rows(
    pool: &PgPool,
    organization_id: Uuid,
    since: chrono::DateTime<chrono::Utc>,
    current_wave_target_ids: Option<&[Uuid]>,
) -> Result<Vec<EasRequiredWebOriginRow>> {
    let target_ids = current_wave_target_ids.map(<[Uuid]>::to_vec);
    let rows = sqlx::query_as::<_, EasRequiredWebOriginRow>(LIST_EAS_REQUIRED_WEB_ORIGINS_SQL)
        .bind(organization_id)
        .bind(since)
        .bind(target_ids)
        .fetch_all(pool)
        .await?;
    let mut current = Vec::new();
    for row in rows {
        if eas_required_origin_still_authorized(&row)? {
            current.push(row);
        }
    }
    Ok(current)
}

fn eas_required_origin_still_authorized(row: &EasRequiredWebOriginRow) -> Result<bool> {
    let origin = golish_pentest_domain::canonical_web_origin(&row.origin).ok_or_else(|| {
        anyhow::anyhow!(
            "malformed EAS required Web Origin '{}' for target {}",
            row.origin,
            row.target_id
        )
    })?;
    Ok(golish_pentest_domain::confirmed_target_web_origins(
        &row.target_name,
        &row.target_value,
        &row.target_ports,
    )
    .into_iter()
    .any(|candidate| candidate.key == origin.key))
}

pub async fn get_surface_identity_snapshot_for_ip(
    pool: &PgPool,
    query: SurfaceIdentitySnapshotQuery,
) -> Result<SurfaceIdentitySnapshot> {
    let ip = normalize_required_ip(&query.ip)?;
    let scope = SurfaceIdentityQueryScope {
        organization_id: query.organization_id,
        project_path: query.project_path,
        ip: Some(ip),
        include_history: query.include_history,
        ..SurfaceIdentityQueryScope::default()
    };

    let endpoints = list_network_endpoints_for_scope(pool, scope.clone()).await?;
    let web_origins = list_web_origins_for_scope(pool, scope.clone()).await?;
    let observations = list_observations_for_scope(pool, scope).await?;
    let summary = summarize_snapshot(endpoints.len(), web_origins.len(), &observations);

    Ok(SurfaceIdentitySnapshot {
        endpoints,
        web_origins,
        observations,
        summary,
    })
}

fn scoped_project_path(
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
) -> Result<Option<String>> {
    if organization_id.is_some() {
        return Ok(None);
    }

    let Some(project_path) = project_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(anyhow::anyhow!(
            "surface identity query requires project_path when organization_id is None"
        )
        .into());
    };

    Ok(Some(project_path.to_string()))
}

fn normalize_optional_ip(raw: Option<&str>) -> Result<Option<String>> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_required_ip)
        .transpose()
}

fn normalize_required_ip(raw: &str) -> Result<String> {
    normalize_network_endpoint(raw, 1, "tcp")
        .map(|identity| identity.ip)
        .ok_or_else(|| anyhow::anyhow!("invalid surface identity ip query: {raw}").into())
}

fn normalize_optional_host(raw: Option<&str>) -> Result<Option<String>> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_host_for_query)
        .transpose()
}

fn normalize_host_for_query(raw: &str) -> Result<String> {
    let normalized = raw
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return Err(anyhow::anyhow!("invalid surface identity host query: host is empty").into());
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        Ok(ip.to_string())
    } else {
        Ok(normalized)
    }
}

fn summarize_snapshot(
    endpoint_count: usize,
    web_origin_count: usize,
    observations: &[WebOriginObservation],
) -> SurfaceIdentitySnapshotSummary {
    SurfaceIdentitySnapshotSummary {
        endpoint_count: endpoint_count as u64,
        web_origin_count: web_origin_count as u64,
        observation_count: observations.len() as u64,
        inferred_observation_count: observations
            .iter()
            .filter(|observation| observation_kind(observation) == Some("inferred"))
            .count() as u64,
        confirmed_observation_count: observations
            .iter()
            .filter(|observation| observation_kind(observation) == Some("confirmed"))
            .count() as u64,
    }
}

fn observation_kind(observation: &WebOriginObservation) -> Option<&str> {
    observation
        .raw
        .get("observation_kind")
        .and_then(serde_json::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn observation_with_kind(kind: &str) -> WebOriginObservation {
        WebOriginObservation {
            id: Uuid::new_v4(),
            organization_id: None,
            project_path: "proj-a".to_string(),
            web_origin_id: Uuid::new_v4(),
            network_endpoint_id: Some(Uuid::new_v4()),
            target_id: None,
            observed_ip: Some("1.1.1.1".to_string()),
            sni: None,
            host_header: None,
            status_code: Some(200),
            title: None,
            final_url: None,
            redirect_chain: json!([]),
            body_hash: None,
            favicon_hash: None,
            screenshot_path: None,
            capture_path: None,
            observed_at: Utc::now(),
            confidence: 0.8,
            source: "test".to_string(),
            raw: json!({ "observation_kind": kind }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn list_endpoints_by_ip_uses_normalized_ip_filter_and_stable_sort() {
        assert_eq!(normalize_required_ip(" 1.1.1.1 ").unwrap(), "1.1.1.1");
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL.contains("ne.ip = $3"));
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL
            .contains("ORDER BY ne.ip ASC, ne.port ASC, ne.transport ASC"));
    }

    #[test]
    fn list_origins_by_endpoint_reads_observation_relation_only() {
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("woo.web_origin_id = wo.id"));
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("woo.network_endpoint_id = $4"));
        assert!(!LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("wo.network_endpoint_id"));
    }

    #[test]
    fn list_origins_by_ip_supports_endpoint_relation_and_ip_literal_host() {
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("JOIN network_endpoints ne"));
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("ne.ip = $5"));
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("wo.host = $5"));
        assert_eq!(normalize_host_for_query("1.1.1.1").unwrap(), "1.1.1.1");
    }

    #[test]
    fn snapshot_summary_counts_same_ip_multi_domain_shape() {
        let observations = vec![
            observation_with_kind("confirmed"),
            observation_with_kind("confirmed"),
        ];

        let summary = summarize_snapshot(1, 2, &observations);

        assert_eq!(summary.endpoint_count, 1);
        assert_eq!(summary.web_origin_count, 2);
        assert_eq!(summary.observation_count, 2);
        assert_eq!(summary.confirmed_observation_count, 2);
    }

    #[test]
    fn same_domain_multi_ip_can_query_origins_endpoints_and_observations_by_host() {
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("wo.host = $3"));
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL.contains("JOIN web_origins wo"));
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL.contains("wo.host = $5"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("wo.host = $7"));
        assert_eq!(
            normalize_host_for_query("A.Example.COM.").unwrap(),
            "a.example.com"
        );
    }

    #[test]
    fn ip_literal_web_origin_without_observation_is_included_by_snapshot_origin_query_only() {
        assert!(
            LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("$5::text IS NULL OR wo.host = $5 OR EXISTS")
        );
        assert!(!LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("INSERT"));
        assert!(!LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("UPSERT"));
    }

    #[test]
    fn latest_observation_query_returns_latest_per_origin_endpoint_pair_by_default() {
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL
            .contains("DISTINCT ON (woo.web_origin_id, woo.network_endpoint_id)"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("woo.observed_at DESC"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("woo.created_at DESC"));
        assert!(!LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL.contains("DISTINCT ON"));
    }

    #[test]
    fn observation_history_query_can_return_multiple_rows_for_same_pair() {
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL.contains("SELECT woo.*"));
        assert!(!LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL.contains("DISTINCT ON"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_HISTORY_SQL.contains("ORDER BY"));
    }

    #[test]
    fn organization_and_project_scope_are_safe_against_full_table_scan() {
        assert!(scoped_project_path(Some(Uuid::new_v4()), None)
            .unwrap()
            .is_none());
        assert_eq!(
            scoped_project_path(None, Some(" proj-a "))
                .unwrap()
                .as_deref(),
            Some("proj-a")
        );
        assert!(scoped_project_path(None, None).is_err());
        assert!(scoped_project_path(None, Some("  ")).is_err());
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL.contains("ne.organization_id = $1"));
        assert!(LIST_NETWORK_ENDPOINTS_FOR_SCOPE_SQL.contains("ne.project_path = $2"));
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("wo.organization_id = $1"));
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL.contains("wo.project_path = $2"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("woo.organization_id = $1"));
        assert!(LIST_OBSERVATIONS_FOR_SCOPE_LATEST_SQL.contains("woo.project_path = $2"));
    }

    #[test]
    fn origins_have_stable_sorting() {
        assert!(LIST_WEB_ORIGINS_FOR_SCOPE_SQL
            .contains("ORDER BY wo.host ASC, wo.scheme ASC, wo.port ASC"));
    }

    #[test]
    fn eas_required_origins_are_current_org_target_bound_fresh_and_probe_confirmed() {
        let sql = LIST_EAS_REQUIRED_WEB_ORIGINS_SQL;
        assert!(sql.contains("woo.organization_id = $1"));
        assert!(sql.contains("wo.organization_id = $1"));
        assert!(sql.contains("t.organization_id = $1"));
        assert!(sql.contains("t.scope::text = 'in'"));
        assert!(sql.contains("woo.project_path = t.project_path"));
        assert!(sql.contains("wo.project_path = t.project_path"));
        assert!(sql.contains("t.created_at <= $2"));
        assert!(sql.contains("woo.target_id = t.id"));
        assert!(sql.contains("woo.observed_at >= $2"));
        assert!(sql.contains("$3::uuid[] IS NULL OR woo.target_id = ANY($3)"));
        assert!(sql.contains("LOWER(woo.source) IN"));
        assert!(sql.contains("'httpx'"));
        assert!(sql.contains("'nmap'"));
        assert!(sql.contains("SELECT DISTINCT woo.target_id, wo.origin"));
        assert!(sql.contains("ORDER BY woo.target_id, wo.origin"));
    }

    #[test]
    fn eas_required_origin_is_dropped_after_target_no_longer_owns_exact_origin() {
        let mut row = EasRequiredWebOriginRow {
            target_id: Uuid::new_v4(),
            origin: "https://app.example.test:443".to_string(),
            target_name: "app.example.test".to_string(),
            target_value: "app.example.test".to_string(),
            target_ports: serde_json::json!([{
                "state": "open",
                "url": "https://app.example.test:443/"
            }]),
        };
        assert!(eas_required_origin_still_authorized(&row).unwrap());

        row.target_ports = serde_json::json!([{
            "state": "open",
            "url": "https://other.example.test:443/"
        }]);
        row.target_name = "other.example.test".to_string();
        row.target_value = "other.example.test".to_string();
        assert!(!eas_required_origin_still_authorized(&row).unwrap());
    }

    #[test]
    fn eas_required_origin_never_trusts_display_name_or_foreign_port_url() {
        let row = EasRequiredWebOriginRow {
            target_id: Uuid::new_v4(),
            origin: "https://vendor.example:443".to_string(),
            target_name: "https://vendor.example".to_string(),
            target_value: "moresec.cn".to_string(),
            target_ports: serde_json::json!([{
                "state": "open",
                "url": "https://vendor.example:443/"
            }]),
        };

        assert!(!eas_required_origin_still_authorized(&row).unwrap());
    }

    #[test]
    fn snapshot_summary_separates_inferred_and_confirmed_observations() {
        let observations = vec![
            observation_with_kind("inferred"),
            observation_with_kind("confirmed"),
            observation_with_kind("confirmed"),
        ];

        let summary = summarize_snapshot(2, 1, &observations);

        assert_eq!(summary.endpoint_count, 2);
        assert_eq!(summary.web_origin_count, 1);
        assert_eq!(summary.observation_count, 3);
        assert_eq!(summary.inferred_observation_count, 1);
        assert_eq!(summary.confirmed_observation_count, 2);
    }
}
