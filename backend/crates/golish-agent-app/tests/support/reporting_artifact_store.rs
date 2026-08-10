use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration as StdDuration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_reporting_app::{
    ArtifactPublicationReservation, ContentAddressedArtifact, ReportArtifactStore,
    ReportArtifactStoreFactory, ReportFormat, ReportingAppError, StagedArtifact,
};
use uuid::Uuid;

const REPORT_ARTIFACT_ORPHAN_GRACE: StdDuration = StdDuration::from_secs(24 * 60 * 60);

fn report_format_to_storage(
    format: ReportFormat,
) -> golish_projects::file_storage::ReportArtifactFormat {
    match format {
        ReportFormat::Markdown => golish_projects::file_storage::ReportArtifactFormat::Markdown,
        ReportFormat::Json => golish_projects::file_storage::ReportArtifactFormat::Json,
    }
}

fn report_format_from_storage(
    format: golish_projects::file_storage::ReportArtifactFormat,
) -> ReportFormat {
    match format {
        golish_projects::file_storage::ReportArtifactFormat::Markdown => ReportFormat::Markdown,
        golish_projects::file_storage::ReportArtifactFormat::Json => ReportFormat::Json,
    }
}

fn report_artifact_error(error: impl std::fmt::Display) -> ReportingAppError {
    ReportingAppError::Artifact(error.to_string())
}

struct TestArtifactPublicationReservation {
    artifact: ContentAddressedArtifact,
    _storage_reservation: golish_projects::file_storage::ReservedReportArtifact,
}

impl ArtifactPublicationReservation for TestArtifactPublicationReservation {
    fn artifact(&self) -> &ContentAddressedArtifact {
        &self.artifact
    }
}

#[derive(Clone, Debug)]
struct TestProjectReportArtifactStore {
    project_root: PathBuf,
}

impl TestProjectReportArtifactStore {
    fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn stored_staging(
        staged: &StagedArtifact,
    ) -> golish_projects::file_storage::StagedReportArtifact {
        golish_projects::file_storage::StagedReportArtifact {
            revision_id: staged.revision_id.to_string(),
            format: report_format_to_storage(staged.format),
            staging_key: staged.staging_key.clone(),
            sha256: staged.sha256.clone(),
            byte_len: staged.byte_len,
        }
    }

    fn stored_artifact(
        artifact: &ContentAddressedArtifact,
    ) -> golish_projects::file_storage::StoredReportArtifact {
        golish_projects::file_storage::StoredReportArtifact {
            format: report_format_to_storage(artifact.format),
            content_key: artifact.content_key.clone(),
            storage_path: format!(".golish/reports/blobs/{}", artifact.content_key),
            sha256: artifact.sha256.clone(),
            byte_len: artifact.byte_len,
        }
    }
}

#[async_trait]
impl ReportArtifactStore for TestProjectReportArtifactStore {
    async fn stage(
        &self,
        revision_id: Uuid,
        format: ReportFormat,
        bytes: &[u8],
    ) -> Result<StagedArtifact, ReportingAppError> {
        let staged = golish_projects::file_storage::stage_report_artifact(
            &self.project_root,
            &revision_id.to_string(),
            report_format_to_storage(format),
            bytes,
        )
        .await
        .map_err(report_artifact_error)?;
        Ok(StagedArtifact {
            revision_id,
            format: report_format_from_storage(staged.format),
            staging_key: staged.staging_key,
            sha256: staged.sha256,
            byte_len: staged.byte_len,
        })
    }

    async fn promote(
        &self,
        staged: &StagedArtifact,
    ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError> {
        let storage_reservation = golish_projects::file_storage::promote_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(report_artifact_error)?;
        let stored = storage_reservation.artifact();
        let artifact = ContentAddressedArtifact {
            format: report_format_from_storage(stored.format),
            content_key: stored.content_key.clone(),
            sha256: stored.sha256.clone(),
            byte_len: stored.byte_len,
        };
        Ok(Box::new(TestArtifactPublicationReservation {
            artifact,
            _storage_reservation: storage_reservation,
        }))
    }

    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool, ReportingAppError> {
        golish_projects::file_storage::verify_report_artifact(
            &self.project_root,
            &Self::stored_artifact(artifact),
        )
        .await
        .map_err(report_artifact_error)
    }

    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<(), ReportingAppError> {
        golish_projects::file_storage::discard_staged_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(report_artifact_error)
    }

    async fn gc(
        &self,
        now: DateTime<Utc>,
        referenced_content_keys: BTreeSet<String>,
    ) -> Result<(), ReportingAppError> {
        let now = SystemTime::UNIX_EPOCH
            + StdDuration::from_secs(u64::try_from(now.timestamp()).unwrap_or_default())
            + StdDuration::from_nanos(u64::from(now.timestamp_subsec_nanos()));
        golish_projects::file_storage::gc_report_artifacts(
            &self.project_root,
            now,
            REPORT_ARTIFACT_ORPHAN_GRACE,
            &referenced_content_keys,
        )
        .await
        .map(|_| ())
        .map_err(report_artifact_error)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TestProjectReportArtifactStoreFactory;

impl ReportArtifactStoreFactory for TestProjectReportArtifactStoreFactory {
    fn for_project(
        &self,
        _project_scope_id: Uuid,
        canonical_project_root: &Path,
    ) -> Arc<dyn ReportArtifactStore> {
        Arc::new(TestProjectReportArtifactStore::new(
            canonical_project_root.to_path_buf(),
        ))
    }
}

pub fn report_blob_path(project_root: &Path, content_key: &str) -> PathBuf {
    project_root.join(".golish/reports/blobs").join(content_key)
}
