use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSectionKind {
    ExecutiveSummary,
    Organization,
    Findings,
    AttackPaths,
    CleanupResiduals,
    Methodology,
    Limitations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportClaimKind {
    Scope,
    Finding,
    CandidateDisposition,
    TechniqueOutcome,
    AttackPath,
    ObjectiveOutcome,
    CleanupResidual,
    Limitation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportClaim {
    pub claim_id: Uuid,
    pub revision_id: Uuid,
    pub section_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub claim_kind: ReportClaimKind,
    pub subject_ref: String,
    pub predicate: String,
    pub value: serde_json::Value,
    pub citation_ids: Vec<Uuid>,
    pub ordinal: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportSectionModel {
    pub section_id: Uuid,
    pub revision_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub organization_name_at_snapshot: Option<String>,
    pub kind: ReportSectionKind,
    pub claims: Vec<ReportClaim>,
    pub rendered_content: Option<String>,
    pub ordinal: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrganizationReportSection {
    pub organization_id_at_time: Uuid,
    pub organization_name_at_snapshot: String,
    pub section: ReportSectionModel,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFinding {
    pub finding_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub candidate_id: Option<Uuid>,
    pub verified_lineage_id: Option<Uuid>,
    pub claim_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportResidual {
    pub obligation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub status: String,
    pub claim_id: Uuid,
}
