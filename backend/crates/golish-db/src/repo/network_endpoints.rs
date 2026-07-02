use crate::models::NetworkEndpoint;
use crate::repo::surface_identity::NormalizedNetworkEndpoint;
use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub const UPSERT_BY_IDENTITY_SQL: &str = r#"
INSERT INTO network_endpoints
    (organization_id, project_path, ip, port, transport, state,
     service_name, service_product, service_version, banner, tls_detected,
     source, confidence, first_seen_at, last_seen_at, last_confirmed_at)
VALUES
    ($1, COALESCE($2, ''), $3, $4, $5, COALESCE($6, 'unknown'),
     $7, $8, $9, $10, COALESCE($11, FALSE),
     COALESCE($12, 'unknown'), COALESCE($13, 0.5), NOW(), NOW(), $14)
ON CONFLICT (organization_id, ip, transport, port) WHERE organization_id IS NOT NULL
DO UPDATE SET
    project_path = CASE
        WHEN EXCLUDED.project_path <> '' THEN EXCLUDED.project_path
        ELSE network_endpoints.project_path
    END,
    state = CASE
        WHEN EXCLUDED.state <> 'unknown' THEN EXCLUDED.state
        ELSE network_endpoints.state
    END,
    service_name = COALESCE(EXCLUDED.service_name, network_endpoints.service_name),
    service_product = COALESCE(EXCLUDED.service_product, network_endpoints.service_product),
    service_version = COALESCE(EXCLUDED.service_version, network_endpoints.service_version),
    banner = COALESCE(EXCLUDED.banner, network_endpoints.banner),
    tls_detected = network_endpoints.tls_detected OR EXCLUDED.tls_detected,
    last_seen_at = NOW(),
    last_confirmed_at = COALESCE(EXCLUDED.last_confirmed_at, network_endpoints.last_confirmed_at),
    source = EXCLUDED.source,
    confidence = GREATEST(network_endpoints.confidence, EXCLUDED.confidence),
    updated_at = NOW()
RETURNING *
"#;

pub const UPSERT_BY_PROJECT_IDENTITY_SQL: &str = r#"
INSERT INTO network_endpoints
    (organization_id, project_path, ip, port, transport, state,
     service_name, service_product, service_version, banner, tls_detected,
     source, confidence, first_seen_at, last_seen_at, last_confirmed_at)
VALUES
    (NULL, COALESCE($1, ''), $2, $3, $4, COALESCE($5, 'unknown'),
     $6, $7, $8, $9, COALESCE($10, FALSE),
     COALESCE($11, 'unknown'), COALESCE($12, 0.5), NOW(), NOW(), $13)
ON CONFLICT (project_path, ip, transport, port) WHERE organization_id IS NULL
DO UPDATE SET
    state = CASE
        WHEN EXCLUDED.state <> 'unknown' THEN EXCLUDED.state
        ELSE network_endpoints.state
    END,
    service_name = COALESCE(EXCLUDED.service_name, network_endpoints.service_name),
    service_product = COALESCE(EXCLUDED.service_product, network_endpoints.service_product),
    service_version = COALESCE(EXCLUDED.service_version, network_endpoints.service_version),
    banner = COALESCE(EXCLUDED.banner, network_endpoints.banner),
    tls_detected = network_endpoints.tls_detected OR EXCLUDED.tls_detected,
    last_seen_at = NOW(),
    last_confirmed_at = COALESCE(EXCLUDED.last_confirmed_at, network_endpoints.last_confirmed_at),
    source = EXCLUDED.source,
    confidence = GREATEST(network_endpoints.confidence, EXCLUDED.confidence),
    updated_at = NOW()
RETURNING *
"#;

pub async fn upsert_by_identity(
    pool: &PgPool,
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
    identity: &NormalizedNetworkEndpoint,
    state: Option<&str>,
    service_name: Option<&str>,
    service_product: Option<&str>,
    service_version: Option<&str>,
    banner: Option<&str>,
    tls_detected: Option<bool>,
    source: Option<&str>,
    confidence: Option<f32>,
    last_confirmed: bool,
) -> Result<NetworkEndpoint> {
    if let Some(org_id) = organization_id {
        let row = sqlx::query_as::<_, NetworkEndpoint>(UPSERT_BY_IDENTITY_SQL)
            .bind(org_id)
            .bind(project_path)
            .bind(&identity.ip)
            .bind(identity.port)
            .bind(&identity.transport)
            .bind(state)
            .bind(service_name)
            .bind(service_product)
            .bind(service_version)
            .bind(banner)
            .bind(tls_detected)
            .bind(source)
            .bind(confidence)
            .bind(last_confirmed.then(chrono::Utc::now))
            .fetch_one(pool)
            .await?;
        Ok(row)
    } else {
        let row = sqlx::query_as::<_, NetworkEndpoint>(UPSERT_BY_PROJECT_IDENTITY_SQL)
            .bind(project_path)
            .bind(&identity.ip)
            .bind(identity.port)
            .bind(&identity.transport)
            .bind(state)
            .bind(service_name)
            .bind(service_product)
            .bind(service_version)
            .bind(banner)
            .bind(tls_detected)
            .bind(source)
            .bind(confidence)
            .bind(last_confirmed.then(chrono::Utc::now))
            .fetch_one(pool)
            .await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_upsert_is_idempotent_on_endpoint_identity() {
        assert!(UPSERT_BY_IDENTITY_SQL.contains(
            "ON CONFLICT (organization_id, ip, transport, port) WHERE organization_id IS NOT NULL"
        ));
        assert!(UPSERT_BY_IDENTITY_SQL.contains("last_seen_at = NOW()"));
        assert!(UPSERT_BY_IDENTITY_SQL.contains("GREATEST(network_endpoints.confidence"));
    }

    #[test]
    fn project_upsert_is_idempotent_when_org_is_null() {
        assert!(UPSERT_BY_PROJECT_IDENTITY_SQL.contains(
            "ON CONFLICT (project_path, ip, transport, port) WHERE organization_id IS NULL"
        ));
        assert!(UPSERT_BY_PROJECT_IDENTITY_SQL.contains("updated_at = NOW()"));
    }
}
