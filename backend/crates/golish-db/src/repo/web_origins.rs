use crate::models::WebOrigin;
use crate::repo::surface_identity::NormalizedWebOrigin;
use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub const UPSERT_BY_IDENTITY_SQL: &str = r#"
INSERT INTO web_origins
    (organization_id, project_path, scheme, host, host_type, port, origin,
     source, confidence, first_seen_at, last_seen_at, last_confirmed_at)
VALUES
    ($1, COALESCE($2, ''), $3, $4, $5, $6, $7,
     COALESCE($8, 'unknown'), COALESCE($9, 0.5), NOW(), NOW(), $10)
ON CONFLICT (organization_id, scheme, host, port) WHERE organization_id IS NOT NULL
DO UPDATE SET
    project_path = CASE
        WHEN EXCLUDED.project_path <> '' THEN EXCLUDED.project_path
        ELSE web_origins.project_path
    END,
    host_type = CASE
        WHEN EXCLUDED.host_type <> 'unknown' THEN EXCLUDED.host_type
        ELSE web_origins.host_type
    END,
    origin = EXCLUDED.origin,
    last_seen_at = NOW(),
    last_confirmed_at = COALESCE(EXCLUDED.last_confirmed_at, web_origins.last_confirmed_at),
    source = EXCLUDED.source,
    confidence = GREATEST(web_origins.confidence, EXCLUDED.confidence),
    updated_at = NOW()
RETURNING *
"#;

pub const UPSERT_BY_PROJECT_IDENTITY_SQL: &str = r#"
INSERT INTO web_origins
    (organization_id, project_path, scheme, host, host_type, port, origin,
     source, confidence, first_seen_at, last_seen_at, last_confirmed_at)
VALUES
    (NULL, COALESCE($1, ''), $2, $3, $4, $5, $6,
     COALESCE($7, 'unknown'), COALESCE($8, 0.5), NOW(), NOW(), $9)
ON CONFLICT (project_path, scheme, host, port) WHERE organization_id IS NULL
DO UPDATE SET
    host_type = CASE
        WHEN EXCLUDED.host_type <> 'unknown' THEN EXCLUDED.host_type
        ELSE web_origins.host_type
    END,
    origin = EXCLUDED.origin,
    last_seen_at = NOW(),
    last_confirmed_at = COALESCE(EXCLUDED.last_confirmed_at, web_origins.last_confirmed_at),
    source = EXCLUDED.source,
    confidence = GREATEST(web_origins.confidence, EXCLUDED.confidence),
    updated_at = NOW()
RETURNING *
"#;

pub async fn upsert_by_identity(
    pool: &PgPool,
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
    identity: &NormalizedWebOrigin,
    source: Option<&str>,
    confidence: Option<f32>,
    last_confirmed: bool,
) -> Result<WebOrigin> {
    if let Some(org_id) = organization_id {
        let row = sqlx::query_as::<_, WebOrigin>(UPSERT_BY_IDENTITY_SQL)
            .bind(org_id)
            .bind(project_path)
            .bind(&identity.scheme)
            .bind(&identity.host)
            .bind(&identity.host_type)
            .bind(identity.port)
            .bind(&identity.origin)
            .bind(source)
            .bind(confidence)
            .bind(last_confirmed.then(chrono::Utc::now))
            .fetch_one(pool)
            .await?;
        Ok(row)
    } else {
        let row = sqlx::query_as::<_, WebOrigin>(UPSERT_BY_PROJECT_IDENTITY_SQL)
            .bind(project_path)
            .bind(&identity.scheme)
            .bind(&identity.host)
            .bind(&identity.host_type)
            .bind(identity.port)
            .bind(&identity.origin)
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
    fn org_upsert_is_idempotent_on_origin_identity() {
        assert!(UPSERT_BY_IDENTITY_SQL.contains(
            "ON CONFLICT (organization_id, scheme, host, port) WHERE organization_id IS NOT NULL"
        ));
        assert!(UPSERT_BY_IDENTITY_SQL.contains("last_seen_at = NOW()"));
        assert!(UPSERT_BY_IDENTITY_SQL.contains("GREATEST(web_origins.confidence"));
    }

    #[test]
    fn project_upsert_is_idempotent_when_org_is_null() {
        assert!(UPSERT_BY_PROJECT_IDENTITY_SQL.contains(
            "ON CONFLICT (project_path, scheme, host, port) WHERE organization_id IS NULL"
        ));
        assert!(UPSERT_BY_PROJECT_IDENTITY_SQL.contains("updated_at = NOW()"));
    }
}
