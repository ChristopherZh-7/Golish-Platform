use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportRow {
    pub report_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub current_revision_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateReport {
    pub report_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
}

pub async fn create(tx: &mut Transaction<'_, Postgres>, input: &CreateReport) -> Result<ReportRow> {
    if input.scope_snapshot_hash.len() != 64
        || !input
            .scope_snapshot_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(anyhow::anyhow!("report_scope_snapshot_hash_invalid").into());
    }
    Ok(sqlx::query_as::<_, ReportRow>(
        r#"INSERT INTO reports(
               report_id,operation_id,project_scope_id,scope_snapshot_id,scope_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5)
           RETURNING *"#,
    )
    .bind(input.report_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(input.scope_snapshot_id)
    .bind(&input.scope_snapshot_hash)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn get(pool: &PgPool, report_id: Uuid) -> Result<Option<ReportRow>> {
    Ok(
        sqlx::query_as::<_, ReportRow>("SELECT * FROM reports WHERE report_id=$1")
            .bind(report_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_for_operation(pool: &PgPool, operation_id: Uuid) -> Result<Option<ReportRow>> {
    Ok(
        sqlx::query_as::<_, ReportRow>("SELECT * FROM reports WHERE operation_id=$1")
            .bind(operation_id)
            .fetch_optional(pool)
            .await?,
    )
}
