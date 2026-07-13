use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportSectionRow {
    pub section_id: Uuid,
    pub revision_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub organization_name_at_snapshot: Option<String>,
    pub section_kind: String,
    pub ordinal: i32,
    pub rendered_content: Option<String>,
    pub content_hash: String,
}

pub async fn insert(tx: &mut Transaction<'_, Postgres>, row: &ReportSectionRow) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO report_sections(
               section_id,revision_id,organization_id_at_time,
               organization_name_at_snapshot,section_kind,ordinal,
               rendered_content,content_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(row.section_id)
    .bind(row.revision_id)
    .bind(row.organization_id_at_time)
    .bind(&row.organization_name_at_snapshot)
    .bind(&row.section_kind)
    .bind(row.ordinal)
    .bind(&row.rendered_content)
    .bind(&row.content_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportSectionRow>> {
    Ok(sqlx::query_as::<_, ReportSectionRow>(
        "SELECT * FROM report_sections WHERE revision_id=$1 ORDER BY ordinal,section_id",
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
