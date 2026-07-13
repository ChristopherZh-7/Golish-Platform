use golish_memory_domain::source_ref::StoredCanonicalRowId;
use golish_reporting_domain::{ReportSourceKind, ReportSourceSnapshot, ReportSourceVersion};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportSourceManifestRow {
    pub revision_id: Uuid,
    pub ordinal: i32,
    pub source_kind: String,
    pub source_id_kind: String,
    pub source_id_value: String,
    pub source_row_version: i64,
    pub content_hash: Vec<u8>,
}

pub async fn insert_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
    snapshot: &ReportSourceSnapshot,
) -> Result<()> {
    for (ordinal, source) in snapshot.ordered_sources.iter().enumerate() {
        let id = StoredCanonicalRowId::from_domain(&source.id)
            .map_err(|error| anyhow::anyhow!(error.code()))?;
        sqlx::query(
            r#"INSERT INTO report_source_manifest(
                   revision_id,ordinal,source_kind,source_id_kind,source_id_value,
                   source_row_version,content_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(revision_id)
        .bind(i32::try_from(ordinal).map_err(|_| anyhow::anyhow!("report_source_overflow"))?)
        .bind(source.kind.as_str())
        .bind(id.kind)
        .bind(id.value)
        .bind(source.row_version)
        .bind(source.content_hash.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub fn parse_source_kind(value: &str) -> Result<ReportSourceKind> {
    let kind = match value {
        "stage_episode" => ReportSourceKind::StageEpisode,
        "stage_handoff" => ReportSourceKind::StageHandoff,
        "finding" => ReportSourceKind::Finding,
        "technique_outcome" => ReportSourceKind::TechniqueOutcome,
        "candidate_attempt" => ReportSourceKind::CandidateAttempt,
        "finding_lineage" => ReportSourceKind::FindingLineage,
        "post_exploit_action" => ReportSourceKind::PostExploitAction,
        "foothold" => ReportSourceKind::Foothold,
        "internal_asset_observation" => ReportSourceKind::InternalAssetObservation,
        "attack_path" => ReportSourceKind::AttackPath,
        "objective_attempt" => ReportSourceKind::ObjectiveAttempt,
        "cleanup_obligation" => ReportSourceKind::CleanupObligation,
        "cleanup_waiver" => ReportSourceKind::CleanupWaiver,
        "cleanup_blocked_decision" => ReportSourceKind::CleanupBlockedDecision,
        "evidence_audit" => ReportSourceKind::EvidenceAudit,
        _ => return Err(anyhow::anyhow!("report_source_kind_corrupt").into()),
    };
    Ok(kind)
}

pub fn row_to_source(row: ReportSourceManifestRow) -> Result<ReportSourceVersion> {
    let content_hash: [u8; 32] = row
        .content_hash
        .try_into()
        .map_err(|_| anyhow::anyhow!("report_source_hash_corrupt"))?;
    Ok(ReportSourceVersion {
        kind: parse_source_kind(&row.source_kind)?,
        id: StoredCanonicalRowId {
            kind: row.source_id_kind,
            value: row.source_id_value,
        }
        .into_domain()
        .map_err(|error| anyhow::anyhow!(error.code()))?,
        row_version: row.source_row_version,
        content_hash,
    })
}

pub async fn list(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> Result<Vec<ReportSourceManifestRow>> {
    Ok(sqlx::query_as::<_, ReportSourceManifestRow>(
        "SELECT * FROM report_source_manifest WHERE revision_id=$1 ORDER BY ordinal",
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}
