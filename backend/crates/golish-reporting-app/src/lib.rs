pub mod finalizer;
pub mod ports;
pub mod read_model;
pub mod redaction;
pub mod renderer;

pub use finalizer::{ExplicitFinalizeRequest, ReportFinalizer};
pub use ports::{
    ArtifactPublicationReservation, ContentAddressedArtifact, FinalizePublication,
    NarrativeRenderInput, NarrativeRenderOutput, NarrativeRenderer, ReportArtifactStore,
    ReportArtifactStoreFactory, ReportFormat, ReportPublicationPort, ReportTruthPort,
    StagedArtifact,
};
pub use read_model::{BuiltReportRevision, ReportReadModelBuilder};
pub use redaction::redact_report_value;
pub use renderer::{apply_narrative, deterministic_narrative, NarrativeError};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReportingAppError {
    #[error("report source snapshot changed")]
    SourceSnapshotStale,
    #[error("report revision is not the current validated revision")]
    RevisionNotValidated,
    #[error("explicit final publication confirmation is required")]
    FinalizeConfirmationRequired,
    #[error("report artifact failed read-back verification")]
    ArtifactVerificationFailed,
    #[error("report repository failed: {0}")]
    Repository(String),
    #[error("report artifact store failed: {0}")]
    Artifact(String),
    #[error("report validation failed: {0}")]
    Validation(String),
}

impl ReportingAppError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SourceSnapshotStale => "report_source_snapshot_stale",
            Self::RevisionNotValidated => "report_revision_not_validated",
            Self::FinalizeConfirmationRequired => "report_finalize_confirmation_required",
            Self::ArtifactVerificationFailed => "report_artifact_verification_failed",
            Self::Repository(_) => "report_repository_failed",
            Self::Artifact(_) => "report_artifact_store_failed",
            Self::Validation(_) => "report_validation_failed",
        }
    }
}
