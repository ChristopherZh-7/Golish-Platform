use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq)]
pub struct ReportClaimRow {
    pub claim_id: Uuid,
    pub revision_id: Uuid,
    pub section_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub claim_kind: String,
    pub authority_class: String,
    pub subject_ref: String,
    pub predicate: String,
    pub object_value: Value,
    pub claim_hash: String,
    pub ordinal: i32,
}

pub async fn insert(tx: &mut Transaction<'_, Postgres>, row: &ReportClaimRow) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO report_claims(
               claim_id,revision_id,section_id,organization_id_at_time,
               claim_kind,authority_class,subject_ref,predicate,object_value,claim_hash,ordinal
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(row.claim_id)
    .bind(row.revision_id)
    .bind(row.section_id)
    .bind(row.organization_id_at_time)
    .bind(&row.claim_kind)
    .bind(&row.authority_class)
    .bind(&row.subject_ref)
    .bind(&row.predicate)
    .bind(&row.object_value)
    .bind(&row.claim_hash)
    .bind(row.ordinal)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportClaimRow>> {
    Ok(sqlx::query_as::<_, ReportClaimRow>(
        "SELECT * FROM report_claims WHERE revision_id=$1 ORDER BY section_id,ordinal",
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
