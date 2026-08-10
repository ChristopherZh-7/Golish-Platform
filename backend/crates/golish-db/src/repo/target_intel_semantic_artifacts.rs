//! Immutable, exact-replay Target Intel semantic artifact persistence.

use anyhow::{bail, Result};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelSemanticArtifactRow {
    pub artifact_ref: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub session_id: Uuid,
    pub artifact_sha256: String,
    pub redacted_payload: Value,
}

pub async fn put_redacted(
    pool: &PgPool,
    artifact: &TargetIntelSemanticArtifactRow,
) -> Result<bool> {
    if artifact.operation_id.is_nil()
        || artifact.organization_id.is_nil()
        || artifact.session_id.is_nil()
        || artifact
            .artifact_ref
            .strip_prefix("intel-artifact:sha256:")
            .is_none_or(|value| value != artifact.artifact_sha256.as_str())
    {
        bail!("TARGET_INTEL_SEMANTIC_ARTIFACT_IDENTITY_INVALID");
    }
    let inserted = sqlx::query(
        r#"INSERT INTO target_intel_semantic_artifacts (
               artifact_ref, operation_id, organization_id, session_id,
               artifact_sha256, redacted_payload
           ) VALUES ($1,$2,$3,$4,$5,$6)
           ON CONFLICT (operation_id, organization_id, session_id, artifact_ref) DO NOTHING"#,
    )
    .bind(&artifact.artifact_ref)
    .bind(artifact.operation_id)
    .bind(artifact.organization_id)
    .bind(artifact.session_id)
    .bind(&artifact.artifact_sha256)
    .bind(&artifact.redacted_payload)
    .execute(pool)
    .await?;
    let persisted = sqlx::query_as::<_, TargetIntelSemanticArtifactRow>(
        r#"SELECT artifact_ref, operation_id, organization_id, session_id,
                  artifact_sha256, redacted_payload
             FROM target_intel_semantic_artifacts
            WHERE artifact_ref = $1
              AND operation_id = $2
              AND organization_id = $3
              AND session_id = $4"#,
    )
    .bind(&artifact.artifact_ref)
    .bind(artifact.operation_id)
    .bind(artifact.organization_id)
    .bind(artifact.session_id)
    .fetch_one(pool)
    .await?;
    if &persisted != artifact {
        bail!("TARGET_INTEL_SEMANTIC_ARTIFACT_REPLAY_MISMATCH");
    }
    Ok(inserted.rows_affected() == 0)
}
