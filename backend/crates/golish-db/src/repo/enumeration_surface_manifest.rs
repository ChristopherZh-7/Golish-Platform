use crate::models::{ApiEndpoint, Fingerprint};
use crate::repo::scoped::{lock_target_write_guard, TargetWriteGuard};
use crate::{DbError, Result};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointParameterWrite {
    pub name: String,
    pub location: String,
    pub value_type: String,
    pub required: bool,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct EnumerationEndpointObservation {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub target_id: Uuid,
    pub web_origin_id: Uuid,
    pub endpoint_id: Uuid,
    pub project_path: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ExecutableQueryEndpoint {
    pub endpoint_observation_id: Uuid,
    pub endpoint_id: Uuid,
    pub target_id: Uuid,
    pub web_origin_id: Uuid,
    pub url: String,
    pub method: String,
    pub parameter_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, FromRow)]
pub struct OriginSurfaceSummary {
    pub endpoint_count: i64,
    pub executable_query_endpoint_count: i64,
    pub query_parameter_count: i64,
    pub fingerprint_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OriginSurfaceSummaryRow {
    pub origin: String,
    pub endpoint_count: i64,
    pub executable_query_endpoint_count: i64,
    pub query_parameter_count: i64,
    pub fingerprint_count: i64,
}

impl OriginSurfaceSummaryRow {
    pub fn summary(&self) -> OriginSurfaceSummary {
        OriginSurfaceSummary {
            endpoint_count: self.endpoint_count,
            executable_query_endpoint_count: self.executable_query_endpoint_count,
            query_parameter_count: self.query_parameter_count,
            fingerprint_count: self.fingerprint_count,
        }
    }
}

fn validate_parameter(parameter: &EndpointParameterWrite) -> Result<()> {
    if parameter.name.trim().is_empty() || parameter.source.trim().is_empty() {
        return Err(DbError::Other(anyhow::anyhow!(
            "enumeration parameter name/source must not be empty"
        )));
    }
    if !matches!(
        parameter.location.as_str(),
        "query" | "body_or_form" | "path" | "header" | "unknown"
    ) {
        return Err(DbError::Other(anyhow::anyhow!(
            "unsupported enumeration parameter location {}",
            parameter.location
        )));
    }
    Ok(())
}

/// Resolve an already-published exact origin only when it is still linked to
/// the guarded current target. Enumeration must consume the EAS surface truth;
/// it must not create an origin merely because an endpoint URL was observed.
pub async fn resolve_guarded_web_origin_id(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    url: &str,
) -> Result<Uuid> {
    let identity = crate::repo::surface_identity::normalize_web_origin(url).ok_or_else(|| {
        DbError::Other(anyhow::anyhow!(
            "enumeration manifest URL is not an exact HTTP origin"
        ))
    })?;
    let organization_id = guard.organization_id.ok_or_else(|| {
        DbError::Other(anyhow::anyhow!(
            "enumeration manifest target has no organization"
        ))
    })?;
    let row = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT wo.id
           FROM web_origins wo
           JOIN web_origin_observations woo
             ON woo.web_origin_id = wo.id AND woo.target_id = $1
           JOIN targets t ON t.id = woo.target_id
           WHERE wo.organization_id = $2
             AND wo.project_path = $3
             AND wo.scheme = $4
             AND wo.host = $5
             AND wo.port = $6
             AND t.scope::text = 'in'
             AND t.organization_id = $2
             AND t.project_path IS NOT DISTINCT FROM $3
           ORDER BY woo.observed_at DESC
           LIMIT 1"#,
    )
    .bind(guard.target_id)
    .bind(organization_id)
    .bind(&guard.project_path)
    .bind(&identity.scheme)
    .bind(&identity.host)
    .bind(identity.port)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| {
        DbError::NotFound(format!(
            "Enumeration exact origin {} is not present in the authorized EAS surface",
            identity.origin
        ))
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_endpoint_observation_guarded(
    pool: &PgPool,
    guard: &TargetWriteGuard,
    operation_id: Uuid,
    organization_id: Uuid,
    web_origin_id: Uuid,
    endpoint_id: Uuid,
    source: &str,
    parameters: &[EndpointParameterWrite],
) -> Result<EnumerationEndpointObservation> {
    if guard.organization_id != Some(organization_id) {
        return Err(DbError::Other(anyhow::anyhow!(
            "enumeration manifest organization does not match target guard"
        )));
    }
    if source.trim().is_empty() {
        return Err(DbError::Other(anyhow::anyhow!(
            "enumeration manifest source must not be empty"
        )));
    }
    for parameter in parameters {
        validate_parameter(parameter)?;
    }

    let mut tx = pool.begin().await?;
    lock_target_write_guard(&mut tx, guard).await?;

    let observation = sqlx::query_as::<_, EnumerationEndpointObservation>(
        r#"INSERT INTO enumeration_endpoint_observations
               (operation_id, organization_id, target_id, web_origin_id, endpoint_id,
                project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (operation_id, web_origin_id, endpoint_id) DO UPDATE SET
               source = EXCLUDED.source,
               observed_at = NOW(),
               updated_at = NOW()
           WHERE enumeration_endpoint_observations.organization_id = EXCLUDED.organization_id
             AND enumeration_endpoint_observations.target_id = EXCLUDED.target_id
             AND enumeration_endpoint_observations.project_path = EXCLUDED.project_path
           RETURNING id, operation_id, organization_id, target_id, web_origin_id,
                     endpoint_id, project_path, source"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(guard.target_id)
    .bind(web_origin_id)
    .bind(endpoint_id)
    .bind(&guard.project_path)
    .bind(source)
    .fetch_one(&mut *tx)
    .await?;

    for parameter in parameters {
        sqlx::query(
            r#"INSERT INTO enumeration_endpoint_parameters
                   (endpoint_observation_id, name, location, value_type, required, source)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (endpoint_observation_id, location, name) DO UPDATE SET
                   value_type = CASE
                       WHEN EXCLUDED.value_type <> 'unknown' THEN EXCLUDED.value_type
                       ELSE enumeration_endpoint_parameters.value_type
                   END,
                   required = enumeration_endpoint_parameters.required OR EXCLUDED.required,
                   source = EXCLUDED.source,
                   observed_at = NOW(),
                   updated_at = NOW()"#,
        )
        .bind(observation.id)
        .bind(parameter.name.trim())
        .bind(&parameter.location)
        .bind(&parameter.value_type)
        .bind(parameter.required)
        .bind(&parameter.source)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(observation)
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_fingerprint_origin_observation_in_tx(
    connection: &mut PgConnection,
    fingerprint_id: Uuid,
    web_origin_id: Uuid,
    organization_id: Uuid,
    target_id: Uuid,
    project_path: &str,
    source: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"INSERT INTO fingerprint_origin_observations
               (fingerprint_id, web_origin_id, organization_id, target_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (fingerprint_id, web_origin_id) DO UPDATE SET
               source = EXCLUDED.source,
               observed_at = NOW(),
               updated_at = NOW()
           WHERE fingerprint_origin_observations.organization_id = EXCLUDED.organization_id
             AND fingerprint_origin_observations.target_id = EXCLUDED.target_id
             AND fingerprint_origin_observations.project_path = EXCLUDED.project_path"#,
    )
    .bind(fingerprint_id)
    .bind(web_origin_id)
    .bind(organization_id)
    .bind(target_id)
    .bind(project_path)
    .bind(source)
    .execute(connection)
    .await?;
    if result.rows_affected() != 1 {
        return Err(DbError::Other(anyhow::anyhow!(
            "fingerprint/origin observation conflicts with its persisted owner tuple"
        )));
    }
    Ok(())
}

pub async fn list_endpoints_for_operation_origin(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    web_origin_id: Uuid,
) -> Result<Vec<ApiEndpoint>> {
    let rows = sqlx::query_as::<_, ApiEndpoint>(
        r#"SELECT ae.*
           FROM enumeration_endpoint_observations eeo
           JOIN api_endpoints ae ON ae.id = eeo.endpoint_id
           JOIN targets t ON t.id = eeo.target_id
           WHERE eeo.operation_id = $1
             AND eeo.organization_id = $2
             AND eeo.web_origin_id = $3
             AND t.scope::text = 'in'
             AND t.organization_id = eeo.organization_id
             AND t.project_path IS NOT DISTINCT FROM eeo.project_path
             AND ae.project_path IS NOT DISTINCT FROM eeo.project_path
           ORDER BY ae.url, ae.method, ae.id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(web_origin_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_executable_query_endpoints(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    web_origin_id: Uuid,
) -> Result<Vec<ExecutableQueryEndpoint>> {
    let rows = sqlx::query_as::<_, ExecutableQueryEndpoint>(
        r#"SELECT eeo.id AS endpoint_observation_id,
                  ae.id AS endpoint_id,
                  eeo.target_id,
                  eeo.web_origin_id,
                  ae.url,
                  ae.method,
                  array_agg(DISTINCT eep.name ORDER BY eep.name) AS parameter_names
           FROM enumeration_endpoint_observations eeo
           JOIN api_endpoints ae ON ae.id = eeo.endpoint_id
           JOIN targets t ON t.id = eeo.target_id
           JOIN enumeration_endpoint_parameters eep
             ON eep.endpoint_observation_id = eeo.id AND eep.location = 'query'
           WHERE eeo.operation_id = $1
             AND eeo.organization_id = $2
             AND eeo.web_origin_id = $3
             AND upper(ae.method) = 'GET'
             AND t.scope::text = 'in'
             AND t.organization_id = eeo.organization_id
             AND t.project_path IS NOT DISTINCT FROM eeo.project_path
           GROUP BY eeo.id, ae.id, eeo.target_id, eeo.web_origin_id, ae.url, ae.method
           ORDER BY ae.url, ae.id"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(web_origin_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_fingerprints_for_origin(
    pool: &PgPool,
    organization_id: Uuid,
    web_origin_id: Uuid,
) -> Result<Vec<Fingerprint>> {
    let rows = sqlx::query_as::<_, Fingerprint>(
        r#"SELECT f.*
           FROM fingerprint_origin_observations foo
           JOIN fingerprints f ON f.id = foo.fingerprint_id
           JOIN targets t ON t.id = foo.target_id
           JOIN web_origins wo ON wo.id = foo.web_origin_id
           WHERE foo.organization_id = $1
             AND foo.web_origin_id = $2
             AND t.scope::text = 'in'
             AND t.organization_id = foo.organization_id
             AND t.project_path IS NOT DISTINCT FROM foo.project_path
             AND f.project_path IS NOT DISTINCT FROM foo.project_path
             AND wo.organization_id = foo.organization_id
             AND wo.project_path = foo.project_path
           ORDER BY f.confidence DESC, f.detected_at DESC, f.id"#,
    )
    .bind(organization_id)
    .bind(web_origin_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn summarize_origin_surface(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    web_origin_id: Uuid,
) -> Result<OriginSurfaceSummary> {
    let row = sqlx::query_as::<_, OriginSurfaceSummary>(
        r#"SELECT
               COUNT(DISTINCT eeo.endpoint_id)::BIGINT AS endpoint_count,
               COUNT(DISTINCT eeo.endpoint_id) FILTER (
                   WHERE upper(ae.method) = 'GET' AND eep.location = 'query'
               )::BIGINT AS executable_query_endpoint_count,
               COUNT(DISTINCT (eep.endpoint_observation_id, eep.name)) FILTER (
                   WHERE upper(ae.method) = 'GET' AND eep.location = 'query'
               )::BIGINT AS query_parameter_count,
               (
                   SELECT COUNT(DISTINCT foo.fingerprint_id)::BIGINT
                   FROM fingerprint_origin_observations foo
                   JOIN fingerprints f ON f.id = foo.fingerprint_id
                   JOIN targets ft ON ft.id = foo.target_id
                   WHERE foo.organization_id = $2
                     AND foo.web_origin_id = $3
                     AND ft.scope::text = 'in'
                     AND ft.organization_id = foo.organization_id
                     AND ft.project_path IS NOT DISTINCT FROM foo.project_path
                     AND f.project_path IS NOT DISTINCT FROM foo.project_path
               ) AS fingerprint_count
           FROM enumeration_endpoint_observations eeo
           JOIN api_endpoints ae ON ae.id = eeo.endpoint_id
           JOIN targets t ON t.id = eeo.target_id
           LEFT JOIN enumeration_endpoint_parameters eep
             ON eep.endpoint_observation_id = eeo.id
           WHERE eeo.operation_id = $1
             AND eeo.organization_id = $2
             AND eeo.web_origin_id = $3
             AND t.scope::text = 'in'
             AND t.organization_id = eeo.organization_id
             AND t.project_path IS NOT DISTINCT FROM eeo.project_path"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(web_origin_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Return every exact origin owned by the organization, including origins with
/// zero manifest rows. The caller intersects this with the final-sealed
/// Enumeration handoff; absent surfaces therefore become explicit zeros rather
/// than disappearing from the Vuln denominator.
pub async fn summarize_operation_surfaces(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<Vec<OriginSurfaceSummaryRow>> {
    let authority_operation_id = super::operation_stage_forks::source_operation_for_stage(
        pool,
        operation_id,
        organization_id,
        "enumeration",
    )
    .await
    .map_err(|error| crate::DbError::Other(anyhow::anyhow!(error)))?
    .unwrap_or(operation_id);
    let rows = sqlx::query_as::<_, OriginSurfaceSummaryRow>(
        r#"SELECT wo.origin,
                  COUNT(DISTINCT eeo.endpoint_id) FILTER (
                      WHERE t.id IS NOT NULL
                  )::BIGINT AS endpoint_count,
                  COUNT(DISTINCT eeo.endpoint_id) FILTER (
                      WHERE t.id IS NOT NULL
                        AND upper(ae.method) = 'GET'
                        AND eep.location = 'query'
                  )::BIGINT AS executable_query_endpoint_count,
                  COUNT(DISTINCT (eep.endpoint_observation_id, eep.name)) FILTER (
                      WHERE t.id IS NOT NULL
                        AND upper(ae.method) = 'GET'
                        AND eep.location = 'query'
                  )::BIGINT AS query_parameter_count,
                  (
                      SELECT COUNT(DISTINCT foo.fingerprint_id)::BIGINT
                      FROM fingerprint_origin_observations foo
                      JOIN fingerprints f ON f.id = foo.fingerprint_id
                      JOIN targets ft ON ft.id = foo.target_id
                      WHERE foo.organization_id = $2
                        AND foo.web_origin_id = wo.id
                        AND ft.scope::text = 'in'
                        AND ft.organization_id = foo.organization_id
                        AND ft.project_path IS NOT DISTINCT FROM foo.project_path
                        AND f.project_path IS NOT DISTINCT FROM foo.project_path
                  ) AS fingerprint_count
           FROM web_origins wo
           LEFT JOIN enumeration_endpoint_observations eeo
             ON eeo.web_origin_id = wo.id
            AND eeo.operation_id = $1
            AND eeo.organization_id = $2
           LEFT JOIN api_endpoints ae ON ae.id = eeo.endpoint_id
           LEFT JOIN targets t
             ON t.id = eeo.target_id
            AND t.scope::text = 'in'
            AND t.organization_id = eeo.organization_id
            AND t.project_path IS NOT DISTINCT FROM eeo.project_path
           LEFT JOIN enumeration_endpoint_parameters eep
             ON eep.endpoint_observation_id = eeo.id
           WHERE wo.organization_id = $2
           GROUP BY wo.id, wo.origin
           ORDER BY wo.origin"#,
    )
    .bind(authority_operation_id)
    .bind(organization_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_location_is_closed_and_raw_values_are_not_part_of_the_contract() {
        let valid = EndpointParameterWrite {
            name: "id".to_string(),
            location: "query".to_string(),
            value_type: "unknown".to_string(),
            required: false,
            source: "js_extract_apis".to_string(),
        };
        assert!(validate_parameter(&valid).is_ok());

        let invalid = EndpointParameterWrite {
            location: "cookie".to_string(),
            ..valid
        };
        assert!(validate_parameter(&invalid).is_err());
    }
}
