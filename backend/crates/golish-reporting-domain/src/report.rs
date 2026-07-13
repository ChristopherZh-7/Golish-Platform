use golish_memory_domain::source_ref::{CanonicalRowId, StoredCanonicalRowId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{OrganizationReportSection, ReportCitation, ReportFinding, ReportResidual};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSourceKind {
    StageEpisode,
    StageHandoff,
    Finding,
    TechniqueOutcome,
    CandidateAttempt,
    FindingLineage,
    PostExploitAction,
    Foothold,
    InternalAssetObservation,
    AttackPath,
    ObjectiveAttempt,
    CleanupObligation,
    CleanupWaiver,
    CleanupBlockedDecision,
    EvidenceAudit,
}

impl ReportSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StageEpisode => "stage_episode",
            Self::StageHandoff => "stage_handoff",
            Self::Finding => "finding",
            Self::TechniqueOutcome => "technique_outcome",
            Self::CandidateAttempt => "candidate_attempt",
            Self::FindingLineage => "finding_lineage",
            Self::PostExploitAction => "post_exploit_action",
            Self::Foothold => "foothold",
            Self::InternalAssetObservation => "internal_asset_observation",
            Self::AttackPath => "attack_path",
            Self::ObjectiveAttempt => "objective_attempt",
            Self::CleanupObligation => "cleanup_obligation",
            Self::CleanupWaiver => "cleanup_waiver",
            Self::CleanupBlockedDecision => "cleanup_blocked_decision",
            Self::EvidenceAudit => "evidence_audit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportSourceVersion {
    pub kind: ReportSourceKind,
    pub id: CanonicalRowId,
    pub row_version: i64,
    pub content_hash: [u8; 32],
}

impl ReportSourceVersion {
    fn canonical_key(&self) -> Result<(String, String, String), String> {
        let stored = StoredCanonicalRowId::from_domain(&self.id)
            .map_err(|error| error.code().to_string())?;
        Ok((self.kind.as_str().to_string(), stored.kind, stored.value))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportSourceSnapshot {
    pub transaction_snapshot: String,
    pub ordered_sources: Vec<ReportSourceVersion>,
    pub source_set_hash: [u8; 32],
}

impl ReportSourceSnapshot {
    pub fn freeze(
        transaction_snapshot: impl Into<String>,
        mut sources: Vec<ReportSourceVersion>,
    ) -> Result<Self, String> {
        if sources.iter().any(|source| source.row_version < 0) {
            return Err("report_source_version_invalid".to_string());
        }
        sources.sort_by_cached_key(|source| source.canonical_key());
        let has_duplicate = sources
            .windows(2)
            .any(|pair| pair[0].kind == pair[1].kind && pair[0].id == pair[1].id);
        if has_duplicate {
            return Err("report_source_duplicate".to_string());
        }
        let source_set_hash = canonical_source_set_hash(&sources)?;
        Ok(Self {
            transaction_snapshot: transaction_snapshot.into(),
            ordered_sources: sources,
            source_set_hash,
        })
    }
}

pub fn canonical_source_set_hash(sources: &[ReportSourceVersion]) -> Result<[u8; 32], String> {
    let mut hasher = Sha256::new();
    for source in sources {
        let (kind, id_kind, id_value) = source.canonical_key()?;
        for field in [kind.as_bytes(), id_kind.as_bytes(), id_value.as_bytes()] {
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field);
        }
        hasher.update(source.row_version.to_be_bytes());
        hasher.update(source.content_hash);
    }
    Ok(hasher.finalize().into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportReadModel {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub scope_snapshot_hash: String,
    pub source_snapshot: ReportSourceSnapshot,
    pub organization_sections: Vec<OrganizationReportSection>,
    pub findings: Vec<ReportFinding>,
    pub cleanup_residuals: Vec<ReportResidual>,
    pub citations: Vec<ReportCitation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: ReportSourceKind, id: &str, version: i64, byte: u8) -> ReportSourceVersion {
        ReportSourceVersion {
            kind,
            id: CanonicalRowId::Text(id.to_string()),
            row_version: version,
            content_hash: [byte; 32],
        }
    }

    #[test]
    fn source_set_hash_is_order_independent_but_rejects_new_source() {
        let first = source(ReportSourceKind::Finding, "finding-1", 4, 7);
        let second = source(ReportSourceKind::CleanupObligation, "cleanup-1", 2, 9);
        let a = ReportSourceSnapshot::freeze("tx-1", vec![first.clone(), second.clone()])
            .expect("snapshot");
        let b = ReportSourceSnapshot::freeze("tx-2", vec![second.clone(), first.clone()])
            .expect("snapshot");
        assert_eq!(a.source_set_hash, b.source_set_hash);

        let changed = ReportSourceSnapshot::freeze(
            "tx-3",
            vec![
                first,
                second,
                source(ReportSourceKind::TechniqueOutcome, "outcome-1", 1, 3),
            ],
        )
        .expect("changed snapshot");
        assert_ne!(a.source_set_hash, changed.source_set_hash);
    }
}
