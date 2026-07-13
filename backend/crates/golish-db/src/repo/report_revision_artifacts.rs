use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportRevisionArtifactRow {
    pub revision_id: Uuid,
    pub artifact_kind: String,
    pub content_key: String,
    pub redaction_version: i32,
    pub created_at: DateTime<Utc>,
}

pub async fn attach(
    tx: &mut Transaction<'_, Postgres>,
    row: &ReportRevisionArtifactRow,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO report_revision_artifacts(
               revision_id,artifact_kind,content_key,redaction_version
           ) VALUES($1,$2,$3,$4)"#,
    )
    .bind(row.revision_id)
    .bind(&row.artifact_kind)
    .bind(&row.content_key)
    .bind(row.redaction_version)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportRevisionArtifactRow>> {
    Ok(sqlx::query_as::<_, ReportRevisionArtifactRow>(
        "SELECT * FROM report_revision_artifacts WHERE revision_id=$1 ORDER BY artifact_kind",
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
