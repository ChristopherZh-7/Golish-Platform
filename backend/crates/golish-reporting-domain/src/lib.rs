//! Pure contracts for the canonical, cited Reporting read model.
//!
//! Reporting is a projection of durable facts. Retrieval and graph systems may
//! help an operator navigate those facts, but neither is accepted as Gate
//! authority and neither can manufacture a report claim.

pub mod citation;
pub mod report;
pub mod revision;
pub mod section;
pub mod validation;

pub use citation::{CitationSourceType, ReportCitation};
pub use report::{
    canonical_source_set_hash, AllFreshToolTruthAuthorityBundleRefV1,
    HistoricalArtifactReadAuthorityV0, HistoricalAuthorityTimeStatusV0,
    LegacyCoverageLimitationCode, LegacyReportInputSealV1, ReportAuthorityClass, ReportInputSealV1,
    ReportReadModel, ReportSourceKind, ReportSourceSnapshot, ReportSourceVersion,
    ReportToolTruthAuthoritySetRefV1, RevisionAdjudicationAuthorityMemberV1,
    RevisionAdjudicationAuthoritySetRefV1, RevisionAdjudicationOutcomeV1,
    RevisionAdjudicationReportInputSealV1, WaveTerminalReceiptRefV1,
};
pub use revision::{PublicationStatus, ReportRevision, ValidationStatus};
pub use section::{
    CoverageSufficiencyProjection, OrganizationReportSection, ReportClaim, ReportClaimKind,
    ReportClaimValue, ReportFinding, ReportResidual, ReportSectionKind, ReportSectionModel,
    SecurityVerdictAuthority, SecurityVerdictProjection,
};
pub use validation::{
    validate_report, CleanupBlockedDecisionTruth, CleanupCloseoutTruth, EvidenceAuditTruth,
    ReportValidationError, ReportValidationIssue, ReportValidationResult, ReportValidationTruth,
};
