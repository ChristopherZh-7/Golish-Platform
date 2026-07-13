use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ReportSourceVersion;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationSourceType {
    CanonicalFact,
    EvidenceAudit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportCitation {
    pub citation_id: Uuid,
    pub revision_id: Uuid,
    pub claim_id: Uuid,
    pub source_type: CitationSourceType,
    pub source: ReportSourceVersion,
    pub evidence_audit_id: Option<i64>,
    pub organization_id_at_time: Uuid,
    pub display_label: String,
    pub ordinal: i32,
}
