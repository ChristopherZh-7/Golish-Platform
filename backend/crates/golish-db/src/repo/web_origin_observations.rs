use crate::models::WebOriginObservation;
use crate::Result;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NewWebOriginObservation<'a> {
    pub organization_id: Option<Uuid>,
    pub project_path: Option<&'a str>,
    pub web_origin_id: Uuid,
    pub network_endpoint_id: Option<Uuid>,
    pub target_id: Option<Uuid>,
    pub observed_ip: Option<&'a str>,
    pub sni: Option<&'a str>,
    pub host_header: Option<&'a str>,
    pub status_code: Option<i32>,
    pub title: Option<&'a str>,
    pub final_url: Option<&'a str>,
    pub redirect_chain: Option<&'a serde_json::Value>,
    pub body_hash: Option<&'a str>,
    pub favicon_hash: Option<&'a str>,
    pub screenshot_path: Option<&'a str>,
    pub capture_path: Option<&'a str>,
    pub confidence: Option<f32>,
    pub source: Option<&'a str>,
    pub raw: Option<&'a serde_json::Value>,
}

pub const INSERT_OBSERVATION_SQL: &str = r#"
INSERT INTO web_origin_observations
    (organization_id, project_path, web_origin_id, network_endpoint_id, target_id,
     observed_ip, sni, host_header, status_code, title, final_url, redirect_chain,
     body_hash, favicon_hash, screenshot_path, capture_path, observed_at,
     confidence, source, raw)
VALUES
    ($1, COALESCE($2, ''), $3, $4, $5,
     $6, $7, $8, $9, $10, $11, COALESCE($12, '[]'::jsonb),
     $13, $14, $15, $16, NOW(),
     COALESCE($17, 0.5), COALESCE($18, 'unknown'), COALESCE($19, '{}'::jsonb))
RETURNING *
"#;

pub const UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL: &str = r#"
INSERT INTO web_origin_observations
    (organization_id, project_path, web_origin_id, network_endpoint_id, target_id,
     observed_ip, sni, host_header, status_code, title, final_url, redirect_chain,
     body_hash, favicon_hash, screenshot_path, capture_path, observed_at,
     confidence, source, raw)
VALUES
    ($1, COALESCE($2, ''), $3, $4, $5,
     $6, $7, $8, $9, $10, $11, COALESCE($12, '[]'::jsonb),
     $13, $14, $15, $16, NOW(),
     COALESCE($17, 0.5), COALESCE($18, 'unknown'), COALESCE($19, '{}'::jsonb))
ON CONFLICT (web_origin_id, network_endpoint_id, source, capture_path)
WHERE network_endpoint_id IS NOT NULL AND capture_path IS NOT NULL AND capture_path <> ''
DO UPDATE SET
    target_id = COALESCE(EXCLUDED.target_id, web_origin_observations.target_id),
    observed_ip = COALESCE(EXCLUDED.observed_ip, web_origin_observations.observed_ip),
    sni = COALESCE(EXCLUDED.sni, web_origin_observations.sni),
    host_header = COALESCE(EXCLUDED.host_header, web_origin_observations.host_header),
    status_code = COALESCE(EXCLUDED.status_code, web_origin_observations.status_code),
    title = COALESCE(EXCLUDED.title, web_origin_observations.title),
    final_url = COALESCE(EXCLUDED.final_url, web_origin_observations.final_url),
    redirect_chain = EXCLUDED.redirect_chain,
    body_hash = COALESCE(EXCLUDED.body_hash, web_origin_observations.body_hash),
    favicon_hash = COALESCE(EXCLUDED.favicon_hash, web_origin_observations.favicon_hash),
    screenshot_path = COALESCE(EXCLUDED.screenshot_path, web_origin_observations.screenshot_path),
    observed_at = NOW(),
    confidence = GREATEST(web_origin_observations.confidence, EXCLUDED.confidence),
    raw = web_origin_observations.raw || EXCLUDED.raw,
    updated_at = NOW()
RETURNING *
"#;

pub async fn insert_observation(
    pool: &PgPool,
    input: &NewWebOriginObservation<'_>,
) -> Result<WebOriginObservation> {
    write_observation(pool, INSERT_OBSERVATION_SQL, input).await
}

pub async fn upsert_observation_dedupe(
    pool: &PgPool,
    input: &NewWebOriginObservation<'_>,
) -> Result<WebOriginObservation> {
    if input.network_endpoint_id.is_some()
        && input.capture_path.is_some_and(|path| !path.is_empty())
    {
        write_observation(pool, UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL, input).await
    } else {
        insert_observation(pool, input).await
    }
}

async fn write_observation(
    pool: &PgPool,
    sql: &str,
    input: &NewWebOriginObservation<'_>,
) -> Result<WebOriginObservation> {
    let empty_array = serde_json::json!([]);
    let empty_object = serde_json::json!({});
    let row = sqlx::query_as::<_, WebOriginObservation>(sql)
        .bind(input.organization_id)
        .bind(input.project_path)
        .bind(input.web_origin_id)
        .bind(input.network_endpoint_id)
        .bind(input.target_id)
        .bind(input.observed_ip)
        .bind(input.sni)
        .bind(input.host_header)
        .bind(input.status_code)
        .bind(input.title)
        .bind(input.final_url)
        .bind(input.redirect_chain.unwrap_or(&empty_array))
        .bind(input.body_hash)
        .bind(input.favicon_hash)
        .bind(input.screenshot_path)
        .bind(input.capture_path)
        .bind(input.confidence)
        .bind(input.source)
        .bind(input.raw.unwrap_or(&empty_object))
        .fetch_one(pool)
        .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_insert_allows_history_without_dedupe() {
        assert!(!INSERT_OBSERVATION_SQL.contains("ON CONFLICT"));
        assert!(INSERT_OBSERVATION_SQL.contains("web_origin_id"));
        assert!(INSERT_OBSERVATION_SQL.contains("network_endpoint_id"));
    }

    #[test]
    fn capture_dedupe_keeps_origin_endpoint_many_to_many() {
        assert!(UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL
            .contains("ON CONFLICT (web_origin_id, network_endpoint_id, source, capture_path)"));
        assert!(
            UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL.contains("WHERE network_endpoint_id IS NOT NULL")
        );
        assert!(
            !UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL.contains("web_origins SET network_endpoint_id")
        );
    }

    #[test]
    fn observation_model_allows_same_origin_many_endpoints_and_same_endpoint_many_origins() {
        // The observation identity is the pair (web_origin_id, network_endpoint_id)
        // plus evidence provenance. There is no uniqueness on web_origin_id alone
        // or network_endpoint_id alone, so both directions remain many-to-many.
        assert!(!UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL.contains("ON CONFLICT (web_origin_id)"));
        assert!(
            !UPSERT_OBSERVATION_CAPTURE_DEDUPE_SQL.contains("ON CONFLICT (network_endpoint_id)")
        );
    }
}
