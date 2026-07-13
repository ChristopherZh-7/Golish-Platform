use golish_memory_domain::source_ref::StoredCanonicalRowId;
use golish_reporting_domain::ReportCitation;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportClaimCitationRow {
    pub citation_id: Uuid,
    pub revision_id: Uuid,
    pub claim_id: Uuid,
    pub citation_ordinal: i32,
    pub source_type: String,
    pub source_kind: String,
    pub source_id_kind: String,
    pub source_id_value: String,
    pub source_row_version: i64,
    pub source_hash: Vec<u8>,
    pub evidence_audit_id: i64,
    pub organization_id_at_time: Uuid,
    pub display_label: String,
}

pub async fn insert(tx: &mut Transaction<'_, Postgres>, citation: &ReportCitation) -> Result<()> {
    let source_id = StoredCanonicalRowId::from_domain(&citation.source.id)
        .map_err(|error| anyhow::anyhow!(error.code()))?;
    let evidence_id = citation
        .evidence_audit_id
        .ok_or_else(|| anyhow::anyhow!("report_evidence_citation_required"))?;
    let source_type = match citation.source_type {
        golish_reporting_domain::CitationSourceType::CanonicalFact => "canonical_fact",
        golish_reporting_domain::CitationSourceType::EvidenceAudit => "evidence_audit",
    };
    sqlx::query(
        r#"INSERT INTO report_claim_citations(
               citation_id,revision_id,claim_id,citation_ordinal,source_type,
               source_kind,source_id_kind,source_id_value,source_row_version,
               source_hash,evidence_audit_id,organization_id_at_time,display_label
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(citation.citation_id)
    .bind(citation.revision_id)
    .bind(citation.claim_id)
    .bind(citation.ordinal)
    .bind(source_type)
    .bind(citation.source.kind.as_str())
    .bind(source_id.kind)
    .bind(source_id.value)
    .bind(citation.source.row_version)
    .bind(citation.source.content_hash.as_slice())
    .bind(evidence_id)
    .bind(citation.organization_id_at_time)
    .bind(&citation.display_label)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportClaimCitationRow>> {
    Ok(sqlx::query_as::<_, ReportClaimCitationRow>(
        r#"SELECT * FROM report_claim_citations
            WHERE revision_id=$1 ORDER BY claim_id,citation_ordinal"#,
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
