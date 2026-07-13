use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportArtifactBlobRow {
    pub content_key: String,
    pub sha256: String,
    pub storage_path: String,
    pub byte_len: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutReportArtifactBlob {
    pub content_key: String,
    pub sha256: String,
    pub storage_path: String,
    pub byte_len: i64,
}

pub async fn put(
    tx: &mut Transaction<'_, Postgres>,
    input: &PutReportArtifactBlob,
) -> Result<ReportArtifactBlobRow> {
    sqlx::query(
        r#"INSERT INTO report_artifact_blobs(content_key,sha256,storage_path,byte_len)
           VALUES($1,$2,$3,$4)
           ON CONFLICT(content_key) DO NOTHING"#,
    )
    .bind(&input.content_key)
    .bind(&input.sha256)
    .bind(&input.storage_path)
    .bind(input.byte_len)
    .execute(&mut **tx)
    .await?;
    let row = sqlx::query_as::<_, ReportArtifactBlobRow>(
        "SELECT * FROM report_artifact_blobs WHERE content_key=$1",
    )
    .bind(&input.content_key)
    .fetch_one(&mut **tx)
    .await?;
    if row.sha256 != input.sha256
        || row.storage_path != input.storage_path
        || row.byte_len != input.byte_len
    {
        return Err(anyhow::anyhow!("report_artifact_blob_identity_conflict").into());
    }
    Ok(row)
}

pub async fn list_content_keys_for_project_scope(
    pool: &PgPool,
    project_scope_id: Uuid,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        r#"SELECT DISTINCT artifact.content_key
             FROM report_revision_artifacts AS artifact
             JOIN report_revisions AS revision
               ON revision.revision_id=artifact.revision_id
             JOIN reports AS report ON report.report_id=revision.report_id
            WHERE report.project_scope_id=$1
            ORDER BY artifact.content_key"#,
    )
    .bind(project_scope_id)
    .fetch_all(pool)
    .await?)
}
