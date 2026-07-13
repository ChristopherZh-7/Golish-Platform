use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_reporting_domain::{ReportReadModel, ReportSourceSnapshot, ReportValidationResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::{BuiltReportRevision, ReportingAppError};

#[async_trait]
pub trait ReportTruthPort: Send + Sync {
    /// Implementations must use one REPEATABLE READ, READ ONLY transaction and
    /// enumerate the complete reportable source set, including rows not
    /// consumed by a narrative section.
    async fn build_repeatable_read_snapshot(
        &self,
        operation_id: Uuid,
    ) -> Result<BuiltReportRevision, ReportingAppError>;

    /// Re-runs the exact same full source query used by build. New, updated,
    /// deleted, or invalidated rows must all change the returned snapshot.
    async fn current_source_snapshot(
        &self,
        operation_id: Uuid,
    ) -> Result<ReportSourceSnapshot, ReportingAppError>;

    /// Persists sections, claims, citations and validation attestation in one
    /// short transaction after exact snapshot comparison.
    async fn persist_validated_revision(
        &self,
        revision: &BuiltReportRevision,
        validation_result: &ReportValidationResult,
    ) -> Result<i64, ReportingAppError>;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Markdown,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedArtifact {
    pub revision_id: Uuid,
    pub format: ReportFormat,
    pub staging_key: String,
    pub sha256: String,
    pub byte_len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContentAddressedArtifact {
    pub format: ReportFormat,
    pub content_key: String,
    pub sha256: String,
    pub byte_len: u64,
}

/// Owns the storage-layer lease for one promoted artifact. The finalizer keeps
/// reservations alive until the publication transaction has attached every
/// artifact, preventing orphan GC from acting on a stale DB reference snapshot.
pub trait ArtifactPublicationReservation: Send + Sync {
    fn artifact(&self) -> &ContentAddressedArtifact;
}

#[async_trait]
pub trait ReportArtifactStore: Send + Sync {
    async fn stage(
        &self,
        revision_id: Uuid,
        format: ReportFormat,
        bytes: &[u8],
    ) -> Result<StagedArtifact, ReportingAppError>;
    async fn promote(
        &self,
        staged: &StagedArtifact,
    ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError>;
    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool, ReportingAppError>;
    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<(), ReportingAppError>;
    async fn gc(
        &self,
        now: DateTime<Utc>,
        referenced_content_keys: BTreeSet<String>,
    ) -> Result<(), ReportingAppError>;
}

#[async_trait]
impl<T> ReportArtifactStore for Arc<T>
where
    T: ReportArtifactStore + ?Sized,
{
    async fn stage(
        &self,
        revision_id: Uuid,
        format: ReportFormat,
        bytes: &[u8],
    ) -> Result<StagedArtifact, ReportingAppError> {
        (**self).stage(revision_id, format, bytes).await
    }

    async fn promote(
        &self,
        staged: &StagedArtifact,
    ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError> {
        (**self).promote(staged).await
    }

    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool, ReportingAppError> {
        (**self).verify(artifact).await
    }

    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<(), ReportingAppError> {
        (**self).discard_staging(staged).await
    }

    async fn gc(
        &self,
        now: DateTime<Utc>,
        referenced_content_keys: BTreeSet<String>,
    ) -> Result<(), ReportingAppError> {
        (**self).gc(now, referenced_content_keys).await
    }
}

/// Composition-root factory that binds the storage port to one trusted DB
/// project identity and its server-resolved canonical path. IPC callers never
/// supply a path or storage key.
pub trait ReportArtifactStoreFactory: Send + Sync {
    fn for_project(
        &self,
        project_scope_id: Uuid,
        canonical_project_root: &Path,
    ) -> Arc<dyn ReportArtifactStore>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizePublication {
    pub operation_id: Uuid,
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub expected_row_version: i64,
    pub expected_source_snapshot: ReportSourceSnapshot,
    pub principal_id: Uuid,
    pub artifacts: Vec<ContentAddressedArtifact>,
}

#[async_trait]
pub trait ReportPublicationPort: Send + Sync {
    /// Performs ownership/current-revision checks, full source re-read, CAS,
    /// immutable artifact attachment and audit/outbox write in one short DB
    /// transaction. Filesystem or network I/O is forbidden in this method.
    async fn finalize_publication(
        &self,
        command: FinalizePublication,
    ) -> Result<(), ReportingAppError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct NarrativeRenderInput {
    pub revision_id: Uuid,
    pub model: ReportReadModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NarrativeRenderOutput {
    pub revision_id: Uuid,
    pub narratives_by_claim: Vec<(Uuid, String)>,
}

#[async_trait]
pub trait NarrativeRenderer: Send + Sync {
    /// This port has no tool registry and receives only redacted structured
    /// facts. It may rephrase existing claims but cannot add claim ids.
    async fn render(
        &self,
        input: NarrativeRenderInput,
    ) -> Result<NarrativeRenderOutput, ReportingAppError>;
}
