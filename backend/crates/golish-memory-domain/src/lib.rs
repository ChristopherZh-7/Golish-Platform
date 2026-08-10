//! Pure Memory Fabric domain contracts.
//!
//! This crate deliberately has no SQL, provider, Graphiti, Tauri, or runtime
//! dependency. Canonical persistence and projector execution live behind
//! ports in higher layers.

pub mod assertion;
pub mod classification;
pub mod context;
pub mod embedding;
pub mod episode;
pub mod event_catalog;
pub mod investigation_snapshot;
pub mod scope;
pub mod source_ref;

pub use assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
    KnowledgeAssertionDraft,
};
pub use classification::{AssertionVisibility, KnowledgeClassification};
pub use context::{
    ContextAuthority, ContextContractError, ContextItem, ContextRequest, ContextSubject,
    KnowledgeClass, KnowledgeValue, VaultCredentialRef, DEFAULT_CONTEXT_TOKEN_CAP,
};
pub use embedding::{validate_embedding_dimension, EMBEDDING_DIMENSION_V1};
pub use episode::{EpisodeVerdict, StageEpisode};
pub use event_catalog::{
    routes_for, KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
    ProjectorId, ProjectorRoute,
};
pub use investigation_snapshot::{
    BaselineContextSnapshotV1, InvestigationAnalysisSnapshotV1, InvestigationContextAuthorityV1,
    InvestigationContextMemberV1, InvestigationContextOmissionV1, InvestigationMethodologyHitRefV1,
    InvestigationMethodologyQueryIntentV1, InvestigationSnapshotError,
    BASELINE_CONTEXT_SNAPSHOT_CONTRACT_V1, INVESTIGATION_ANALYSIS_SNAPSHOT_CONTRACT_V1,
};
pub use scope::{OperationScope, ProjectScopeId};
pub use source_ref::{
    CanonicalRowId, CanonicalSourceKind, SourceRef, SourceRefError, StoredCanonicalRowId,
};
