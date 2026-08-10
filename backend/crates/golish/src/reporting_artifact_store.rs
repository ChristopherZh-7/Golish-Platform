//! Project-local, content-addressed Reporting artifact storage and orphan GC.
//!
//! Paths are resolved from the server-owned `project_scopes` row. IPC/model
//! input never supplies a project root, staging key, content key, or filename.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_reporting_app::{
    ArtifactPublicationReservation, ContentAddressedArtifact, ReportArtifactStore,
    ReportArtifactStoreFactory, ReportFormat, ReportingAppError, StagedArtifact,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

const DEFAULT_ORPHAN_GRACE: Duration = Duration::from_secs(24 * 60 * 60);
const DAILY_GC_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

fn format_to_storage(format: ReportFormat) -> golish_projects::file_storage::ReportArtifactFormat {
    match format {
        ReportFormat::Markdown => golish_projects::file_storage::ReportArtifactFormat::Markdown,
        ReportFormat::Json => golish_projects::file_storage::ReportArtifactFormat::Json,
    }
}

fn format_from_storage(
    format: golish_projects::file_storage::ReportArtifactFormat,
) -> ReportFormat {
    match format {
        golish_projects::file_storage::ReportArtifactFormat::Markdown => ReportFormat::Markdown,
        golish_projects::file_storage::ReportArtifactFormat::Json => ReportFormat::Json,
    }
}

fn artifact_error(error: impl std::fmt::Display) -> ReportingAppError {
    ReportingAppError::Artifact(error.to_string())
}

struct ProjectArtifactPublicationReservation {
    artifact: ContentAddressedArtifact,
    _storage_reservation: golish_projects::file_storage::ReservedReportArtifact,
}

impl ArtifactPublicationReservation for ProjectArtifactPublicationReservation {
    fn artifact(&self) -> &ContentAddressedArtifact {
        &self.artifact
    }
}

#[derive(Clone, Debug)]
pub struct ProjectReportArtifactStore {
    project_root: PathBuf,
    orphan_grace: Duration,
}

impl ProjectReportArtifactStore {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            orphan_grace: DEFAULT_ORPHAN_GRACE,
        }
    }

    #[cfg(test)]
    fn with_orphan_grace(project_root: PathBuf, orphan_grace: Duration) -> Self {
        Self {
            project_root,
            orphan_grace,
        }
    }

    fn stored_staging(
        staged: &StagedArtifact,
    ) -> golish_projects::file_storage::StagedReportArtifact {
        golish_projects::file_storage::StagedReportArtifact {
            revision_id: staged.revision_id.to_string(),
            format: format_to_storage(staged.format),
            staging_key: staged.staging_key.clone(),
            sha256: staged.sha256.clone(),
            byte_len: staged.byte_len,
        }
    }

    fn stored_artifact(
        &self,
        artifact: &ContentAddressedArtifact,
    ) -> golish_projects::file_storage::StoredReportArtifact {
        golish_projects::file_storage::StoredReportArtifact {
            format: format_to_storage(artifact.format),
            content_key: artifact.content_key.clone(),
            storage_path: format!(".golish/reports/blobs/{}", artifact.content_key),
            sha256: artifact.sha256.clone(),
            byte_len: artifact.byte_len,
        }
    }
}

#[async_trait]
impl ReportArtifactStore for ProjectReportArtifactStore {
    async fn stage(
        &self,
        revision_id: Uuid,
        format: ReportFormat,
        bytes: &[u8],
    ) -> Result<StagedArtifact, ReportingAppError> {
        let staged = golish_projects::file_storage::stage_report_artifact(
            &self.project_root,
            &revision_id.to_string(),
            format_to_storage(format),
            bytes,
        )
        .await
        .map_err(artifact_error)?;
        Ok(StagedArtifact {
            revision_id,
            format: format_from_storage(staged.format),
            staging_key: staged.staging_key,
            sha256: staged.sha256,
            byte_len: staged.byte_len,
        })
    }

    async fn promote(
        &self,
        staged: &StagedArtifact,
    ) -> Result<Box<dyn ArtifactPublicationReservation>, ReportingAppError> {
        let reservation = golish_projects::file_storage::promote_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(artifact_error)?;
        let artifact = ContentAddressedArtifact {
            format: format_from_storage(reservation.format),
            content_key: reservation.content_key.clone(),
            sha256: reservation.sha256.clone(),
            byte_len: reservation.byte_len,
        };
        Ok(Box::new(ProjectArtifactPublicationReservation {
            artifact,
            _storage_reservation: reservation,
        }))
    }

    async fn verify(&self, artifact: &ContentAddressedArtifact) -> Result<bool, ReportingAppError> {
        golish_projects::file_storage::verify_report_artifact(
            &self.project_root,
            &self.stored_artifact(artifact),
        )
        .await
        .map_err(artifact_error)
    }

    async fn discard_staging(&self, staged: &StagedArtifact) -> Result<(), ReportingAppError> {
        golish_projects::file_storage::discard_staged_report_artifact(
            &self.project_root,
            &Self::stored_staging(staged),
        )
        .await
        .map_err(artifact_error)
    }

    async fn gc(
        &self,
        now: DateTime<Utc>,
        referenced_content_keys: BTreeSet<String>,
    ) -> Result<(), ReportingAppError> {
        let now = SystemTime::UNIX_EPOCH
            + Duration::from_secs(u64::try_from(now.timestamp()).unwrap_or_default())
            + Duration::from_nanos(u64::from(now.timestamp_subsec_nanos()));
        golish_projects::file_storage::gc_report_artifacts(
            &self.project_root,
            now,
            self.orphan_grace,
            &referenced_content_keys,
        )
        .await
        .map(|_| ())
        .map_err(artifact_error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalReportArtifactReadRequest {
    pub receipt_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub principal_id: Uuid,
    /// Server-private digest of request identity, authorization and purpose.
    pub request_private_snapshot_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalReportArtifactReadResult {
    pub bytes: Vec<u8>,
    pub authority: golish_reporting_domain::HistoricalArtifactReadAuthorityV0,
}

/// Complete controlled historical read: authorize exact DB metadata, anchor
/// the project root, read through the hardened no-follow handle path, verify
/// content identity, then append a read attestation against current authority
/// time. Raw bytes never enter report claims or the investigation projection.
pub async fn read_historical_report_artifact(
    pool: &PgPool,
    input: HistoricalReportArtifactReadRequest,
) -> anyhow::Result<HistoricalReportArtifactReadResult> {
    let mut prepare_tx = pool.begin().await?;
    let preparation =
        golish_db::repo::historical_report_artifacts::prepare_historical_artifact_read_on(
            &mut prepare_tx,
            input.receipt_id,
            input.operation_id,
            input.project_scope_id,
            input.principal_id,
        )
        .await?;
    let canonical_project_path: String = sqlx::query_scalar(
        "SELECT canonical_project_path FROM project_scopes WHERE project_scope_id=$1 FOR SHARE",
    )
    .bind(input.project_scope_id)
    .fetch_optional(&mut *prepare_tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("historical_artifact_project_scope_missing"))?;
    prepare_tx.commit().await?;

    let format = match preparation.artifact_kind.as_str() {
        "markdown" => golish_projects::file_storage::ReportArtifactFormat::Markdown,
        "json" => golish_projects::file_storage::ReportArtifactFormat::Json,
        _ => anyhow::bail!("historical_artifact_format_unsupported"),
    };
    let byte_len = u64::try_from(preparation.byte_len)
        .map_err(|_| anyhow::anyhow!("historical_artifact_length_invalid"))?;
    let bytes = golish_projects::file_storage::read_verified_report_artifact(
        Path::new(&canonical_project_path),
        &golish_projects::file_storage::StoredReportArtifact {
            format,
            content_key: preparation.content_key,
            storage_path: preparation.storage_path,
            sha256: preparation.sha256.clone(),
            byte_len,
        },
    )
    .await?;
    let observed_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut attest_tx = pool.begin().await?;
    let current_project_path: String = sqlx::query_scalar(
        "SELECT canonical_project_path FROM project_scopes WHERE project_scope_id=$1 FOR SHARE",
    )
    .bind(input.project_scope_id)
    .fetch_optional(&mut *attest_tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("historical_artifact_project_scope_missing"))?;
    if current_project_path != canonical_project_path {
        anyhow::bail!("historical_artifact_project_scope_changed");
    }
    let authority =
        golish_db::repo::historical_report_artifacts::attest_historical_artifact_read_on(
            &mut attest_tx,
            golish_db::repo::historical_report_artifacts::AttestHistoricalArtifactReadV0 {
                receipt_id: input.receipt_id,
                operation_id: input.operation_id,
                project_scope_id: input.project_scope_id,
                principal_id: input.principal_id,
                request_private_snapshot_hash: input.request_private_snapshot_hash,
                observed_sha256,
                observed_byte_len: i64::try_from(bytes.len())
                    .map_err(|_| anyhow::anyhow!("historical_artifact_length_invalid"))?,
            },
        )
        .await?;
    attest_tx.commit().await?;
    Ok(HistoricalReportArtifactReadResult { bytes, authority })
}

#[derive(Clone, Debug, Default)]
pub struct ProjectReportArtifactStoreFactory;

impl ReportArtifactStoreFactory for ProjectReportArtifactStoreFactory {
    fn for_project(
        &self,
        _project_scope_id: Uuid,
        canonical_project_root: &Path,
    ) -> Arc<dyn ReportArtifactStore> {
        Arc::new(ProjectReportArtifactStore::new(
            canonical_project_root.to_path_buf(),
        ))
    }
}

struct GcLifecycle {
    cancel: watch::Sender<bool>,
    join: JoinHandle<()>,
}

#[derive(Clone)]
pub struct ReportArtifactGcRuntime {
    pool: Arc<PgPool>,
    factory: Arc<dyn ReportArtifactStoreFactory>,
    lifecycle: Arc<Mutex<Option<GcLifecycle>>>,
}

impl ReportArtifactGcRuntime {
    pub fn new(pool: Arc<PgPool>, factory: Arc<dyn ReportArtifactStoreFactory>) -> Self {
        Self {
            pool,
            factory,
            lifecycle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn start(&self) -> bool {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.is_some() {
            return false;
        }
        let (cancel, mut cancelled) = watch::channel(false);
        let runtime = self.clone();
        let join = tokio::spawn(async move {
            let mut interval = tokio::time::interval(DAILY_GC_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(error) = runtime.run_once(Utc::now()).await {
                            tracing::warn!(error = %error, "report artifact GC sweep failed");
                        }
                    }
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() {
                            break;
                        }
                    }
                }
            }
        });
        *lifecycle = Some(GcLifecycle { cancel, join });
        true
    }

    pub async fn shutdown(&self) {
        let lifecycle = self.lifecycle.lock().take();
        if let Some(lifecycle) = lifecycle {
            let _ = lifecycle.cancel.send(true);
            let _ = lifecycle.join.await;
        }
    }

    pub async fn run_once(&self, now: DateTime<Utc>) -> anyhow::Result<()> {
        let projects = golish_db::repo::project_scopes::list_all(&self.pool)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut projects_by_path = BTreeMap::<String, Vec<_>>::new();
        for project in projects {
            projects_by_path
                .entry(project.canonical_project_path.clone())
                .or_default()
                .push(project);
        }
        for (canonical_project_path, projects) in projects_by_path {
            let mut referenced = BTreeSet::new();
            for project in &projects {
                referenced.extend(
                    golish_db::repo::report_artifact_blobs::list_content_keys_for_project_scope(
                        &self.pool,
                        project.project_scope_id,
                    )
                    .await?,
                );
            }
            let representative = projects
                .iter()
                .find(|project| project.retired_at.is_none())
                .or_else(|| projects.first())
                .ok_or_else(|| anyhow::anyhow!("report_artifact_gc_project_group_empty"))?;
            self.factory
                .for_project(
                    representative.project_scope_id,
                    Path::new(&canonical_project_path),
                )
                .gc(now, referenced)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_db::models::NewSession;
    use golish_db::repo::{project_scopes, runtime_memory_tx, sessions};
    use golish_db::{DbConfig, GolishDb};
    use golish_reporting_app::{
        ExplicitFinalizeRequest, FinalizePublication, ReportFinalizer, ReportPublicationPort,
    };
    use golish_reporting_domain::{
        PublicationStatus, ReportReadModel, ReportSourceSnapshot, ValidationStatus,
    };
    use serial_test::serial;
    use tokio::sync::Barrier;

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read local postgres port")
            .port()
    }

    async fn db_fixture() -> (GolishDb, tempfile::TempDir) {
        let data_dir = tempfile::tempdir().expect("temporary postgres data directory");
        let config = DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("report_artifact_gc_{}", Uuid::new_v4().simple()),
            ..DbConfig::default()
        };
        let db = GolishDb::start(config)
            .await
            .expect("start migrated embedded postgres");
        (db, data_dir)
    }

    async fn create_report_revision(
        db: &GolishDb,
        project_path: &str,
        label: &str,
        path_hash: &str,
    ) -> (Uuid, Uuid) {
        let session = sessions::create(
            db.pool(),
            NewSession {
                title: Some(format!("report artifact GC {label}")),
                workspace_path: Some(project_path.to_string()),
                workspace_label: None,
                model: None,
                provider: None,
                project_path: Some(project_path.to_string()),
            },
        )
        .await
        .expect("create report artifact GC session");
        let project_scope = project_scopes::register_first_open(db.pool(), project_path, path_hash)
            .await
            .expect("register report artifact GC project scope");
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        runtime_memory_tx::create_runtime_operation(
            db.pool(),
            &runtime_memory_tx::CreateRuntimeOperationRow {
                operation_id,
                initial_stage_execution_id: stage_execution_id,
                session_id: session.id,
                title: Some(format!("report artifact GC operation {label}")),
                input: "report artifact GC fixture".to_string(),
                profile: "assessment".to_string(),
                entry_stage: "target_intel".to_string(),
                project_scope_id: project_scope.project_scope_id,
                cli_scope: None,
                application_model_contract: golish_core::ApplicationModelContract::LegacyNoModel,
            },
        )
        .await
        .expect("create report artifact GC operation");

        let organization_id = Uuid::new_v4();
        let decision_id = Uuid::new_v4();
        let snapshot_id = Uuid::new_v4();
        let report_id = Uuid::new_v4();
        let revision_id = Uuid::new_v4();
        let mut tx = db.pool().begin().await.expect("begin report GC scope");
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$2,$3)")
            .bind(organization_id)
            .bind(project_path)
            .bind(format!("report artifact GC {label}"))
            .execute(&mut *tx)
            .await
            .expect("insert report GC organization");
        sqlx::query(
            r#"INSERT INTO operation_scope_decisions(
                   id,operation_id,project_scope_id,stage_execution_id,
                   root_organization_id,mode,decision_rows,decision_hash
               ) VALUES($1,$2,$3,$4,$5,'cli_flags',$6,$7)"#,
        )
        .bind(decision_id)
        .bind(operation_id)
        .bind(project_scope.project_scope_id)
        .bind(stage_execution_id)
        .bind(organization_id)
        .bind(serde_json::json!([{"organization_id": organization_id}]))
        .bind(format!("{label}-decision"))
        .execute(&mut *tx)
        .await
        .expect("insert report GC scope decision");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_snapshots(
                   id,operation_id,project_scope_id,scope_decision_id,
                   project_path_at_freeze,root_organization_id,mode,scope_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
        )
        .bind(snapshot_id)
        .bind(operation_id)
        .bind(project_scope.project_scope_id)
        .bind(decision_id)
        .bind(project_path)
        .bind(organization_id)
        .bind(format!("{label}-scope-hash"))
        .execute(&mut *tx)
        .await
        .expect("insert report GC scope snapshot");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,organization_name_at_freeze,
                   role,depth,ordinal,decision_row_id,approval_source
               ) VALUES($1,$2,$3,'root',0,0,'root',$4)"#,
        )
        .bind(snapshot_id)
        .bind(organization_id)
        .bind(format!("report artifact GC {label}"))
        .bind(serde_json::json!({"source": "cli_flags"}))
        .execute(&mut *tx)
        .await
        .expect("insert report GC frozen organization");
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(snapshot_id)
            .execute(&mut *tx)
            .await
            .expect("seal report GC scope");
        sqlx::query(
            r#"INSERT INTO reports(
                   report_id,operation_id,project_scope_id,scope_snapshot_id,
                   scope_snapshot_hash
               ) VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(report_id)
        .bind(operation_id)
        .bind(project_scope.project_scope_id)
        .bind(snapshot_id)
        .bind("a".repeat(64))
        .execute(&mut *tx)
        .await
        .expect("insert report GC report");
        sqlx::query(
            r#"INSERT INTO report_revisions(
                   revision_id,report_id,revision_number,transaction_snapshot,source_set_hash
               ) VALUES($1,$2,1,'report-artifact-gc-fixture',$3)"#,
        )
        .bind(revision_id)
        .bind(report_id)
        .bind("b".repeat(64))
        .execute(&mut *tx)
        .await
        .expect("insert report GC revision");
        tx.commit().await.expect("commit report GC scope");
        (project_scope.project_scope_id, revision_id)
    }

    async fn attach_artifact(
        db: &GolishDb,
        revision_id: Uuid,
        artifact: &ContentAddressedArtifact,
    ) {
        let storage_path = format!(".golish/reports/blobs/{}", artifact.content_key);
        let mut tx = db
            .pool()
            .begin()
            .await
            .expect("begin report artifact attach");
        sqlx::query(
            r#"INSERT INTO report_artifact_blobs(content_key,sha256,storage_path,byte_len)
               VALUES($1,$2,$3,$4)"#,
        )
        .bind(&artifact.content_key)
        .bind(&artifact.sha256)
        .bind(&storage_path)
        .bind(i64::try_from(artifact.byte_len).expect("artifact byte length"))
        .execute(&mut *tx)
        .await
        .expect("insert report GC blob reference");
        sqlx::query(
            r#"INSERT INTO report_revision_artifacts(
                   revision_id,artifact_kind,content_key,redaction_version
               ) VALUES($1,$2,$3,1)"#,
        )
        .bind(revision_id)
        .bind(match artifact.format {
            ReportFormat::Markdown => "markdown",
            ReportFormat::Json => "json",
        })
        .bind(&artifact.content_key)
        .execute(&mut *tx)
        .await
        .expect("attach report GC blob reference");
        tx.commit().await.expect("commit report artifact attach");
    }

    #[derive(Clone)]
    struct BlockingDbPublication {
        pool: PgPool,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    #[async_trait]
    impl ReportPublicationPort for BlockingDbPublication {
        async fn finalize_publication(
            &self,
            command: FinalizePublication,
        ) -> Result<(), ReportingAppError> {
            self.entered.wait().await;
            self.release.wait().await;
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|error| ReportingAppError::Repository(error.to_string()))?;
            for artifact in &command.artifacts {
                let storage_path = format!(".golish/reports/blobs/{}", artifact.content_key);
                sqlx::query(
                    r#"INSERT INTO report_artifact_blobs(
                           content_key,sha256,storage_path,byte_len
                       ) VALUES($1,$2,$3,$4)
                       ON CONFLICT(content_key) DO NOTHING"#,
                )
                .bind(&artifact.content_key)
                .bind(&artifact.sha256)
                .bind(&storage_path)
                .bind(
                    i64::try_from(artifact.byte_len)
                        .map_err(|error| ReportingAppError::Artifact(error.to_string()))?,
                )
                .execute(&mut *tx)
                .await
                .map_err(|error| ReportingAppError::Repository(error.to_string()))?;
                sqlx::query(
                    r#"INSERT INTO report_revision_artifacts(
                           revision_id,artifact_kind,content_key,redaction_version
                       ) VALUES($1,$2,$3,1)"#,
                )
                .bind(command.revision_id)
                .bind(match artifact.format {
                    ReportFormat::Markdown => "markdown",
                    ReportFormat::Json => "json",
                })
                .bind(&artifact.content_key)
                .execute(&mut *tx)
                .await
                .map_err(|error| ReportingAppError::Repository(error.to_string()))?;
            }
            tx.commit()
                .await
                .map_err(|error| ReportingAppError::Repository(error.to_string()))
        }
    }

    #[cfg(unix)]
    fn backdate_blob(path: &Path, age: Duration) {
        use std::os::fd::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open orphan blob for timestamp fixture");
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time after epoch")
            .checked_sub(age)
            .expect("fixture age before now");
        let times = [
            libc::timespec {
                tv_sec: i64::try_from(timestamp.as_secs()).expect("fixture timestamp seconds"),
                tv_nsec: i64::from(timestamp.subsec_nanos()),
            },
            libc::timespec {
                tv_sec: i64::try_from(timestamp.as_secs()).expect("fixture timestamp seconds"),
                tv_nsec: i64::from(timestamp.subsec_nanos()),
            },
        ];
        assert_eq!(
            unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) },
            0
        );
        file.sync_all().expect("sync orphan timestamp fixture");
    }

    #[derive(Clone, Debug, Default)]
    struct ZeroGraceArtifactStoreFactory;

    impl ReportArtifactStoreFactory for ZeroGraceArtifactStoreFactory {
        fn for_project(
            &self,
            _project_scope_id: Uuid,
            canonical_project_root: &Path,
        ) -> Arc<dyn ReportArtifactStore> {
            Arc::new(ProjectReportArtifactStore::with_orphan_grace(
                canonical_project_root.to_path_buf(),
                Duration::ZERO,
            ))
        }
    }

    #[tokio::test]
    async fn artifact_store_is_content_addressed_verified_and_gc_safe() {
        let root = tempfile::tempdir().expect("project root");
        let canonical_root = std::fs::canonicalize(root.path()).expect("canonical project root");
        let store = ProjectReportArtifactStore::with_orphan_grace(canonical_root, Duration::ZERO);
        let revision = Uuid::new_v4();
        let staged = store
            .stage(revision, ReportFormat::Markdown, b"# cited report")
            .await
            .expect("stage");
        let reservation = store.promote(&staged).await.expect("promote");
        let artifact = reservation.artifact().clone();
        assert!(artifact.content_key.starts_with("sha256/"));
        assert!(store.verify(&artifact).await.expect("verify"));
        drop(reservation);

        let mut referenced = BTreeSet::new();
        referenced.insert(artifact.content_key.clone());
        store.gc(Utc::now(), referenced).await.expect("safe gc");
        assert!(store.verify(&artifact).await.expect("referenced retained"));

        store
            .gc(Utc::now(), BTreeSet::new())
            .await
            .expect("orphan gc");
        assert!(!store.verify(&artifact).await.expect("orphan removed"));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[serial]
    async fn finalizer_reservation_prevents_stale_gc_snapshot_from_deleting_new_db_reference() {
        let (mut db, _data_dir) = db_fixture().await;
        let root = tempfile::tempdir().expect("project root");
        let canonical_root = tokio::fs::canonicalize(root.path())
            .await
            .expect("canonical project root");
        let canonical_path = canonical_root.to_str().expect("UTF-8 project root");
        let (project_scope_id, revision_id) =
            create_report_revision(&db, canonical_path, "publish-race", &"3".repeat(64)).await;
        let (report_id, operation_id, scope_snapshot_id) = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
            r#"SELECT reports.report_id,reports.operation_id,reports.scope_snapshot_id
                 FROM reports
                 JOIN report_revisions
                   ON report_revisions.report_id=reports.report_id
                WHERE report_revisions.revision_id=$1"#,
        )
        .bind(revision_id)
        .fetch_one(db.pool())
        .await
        .expect("load report identity");

        let store = ProjectReportArtifactStore::new(canonical_root.clone());
        let bytes = b"# publication reservation race\n";
        let old_staged = store
            .stage(revision_id, ReportFormat::Markdown, bytes)
            .await
            .expect("stage old orphan");
        let old_reservation = store
            .promote(&old_staged)
            .await
            .expect("promote old orphan");
        let old_artifact = old_reservation.artifact().clone();
        drop(old_reservation);
        let blob_path = canonical_root
            .join(".golish/reports/blobs")
            .join(&old_artifact.content_key);
        backdate_blob(&blob_path, DEFAULT_ORPHAN_GRACE + Duration::from_secs(60));

        let source_snapshot = ReportSourceSnapshot::freeze("publish-race", Vec::new())
            .expect("empty deterministic source snapshot");
        let model = ReportReadModel {
            report_id,
            revision_id,
            operation_id,
            project_scope_id,
            scope_snapshot_id,
            scope_snapshot_hash: "a".repeat(64),
            source_snapshot,
            organization_sections: Vec::new(),
            findings: Vec::new(),
            cleanup_residuals: Vec::new(),
            citations: Vec::new(),
        };
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let publication = BlockingDbPublication {
            pool: db.pool().clone(),
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        };
        let finalizer = ReportFinalizer::new(store.clone(), publication);
        let finalize = tokio::spawn(async move {
            finalizer
                .finalize(
                    &model,
                    ExplicitFinalizeRequest {
                        principal_id: Uuid::new_v4(),
                        confirm_final_publish: true,
                        expected_row_version: 0,
                        validation_status: ValidationStatus::Validated,
                        publication_status: PublicationStatus::Unpublished,
                    },
                    vec![(ReportFormat::Markdown, bytes.to_vec())],
                )
                .await
        });
        entered.wait().await;

        let gc_store = store.clone();
        let mut gc = tokio::spawn(async move {
            gc_store
                .gc(Utc::now(), BTreeSet::new())
                .await
                .expect("stale-reference GC completes")
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut gc)
                .await
                .is_err(),
            "GC must wait for the publisher's content-key reservation"
        );

        release.wait().await;
        let artifacts = finalize
            .await
            .expect("join real report finalizer")
            .expect("finalize report artifact");
        gc.await.expect("join stale-reference GC");
        assert_eq!(artifacts, vec![old_artifact.clone()]);
        let attached: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM report_revision_artifacts WHERE revision_id=$1 AND content_key=$2",
        )
        .bind(revision_id)
        .bind(&old_artifact.content_key)
        .fetch_one(db.pool())
        .await
        .expect("count durable artifact reference");
        assert_eq!(attached, 1);
        assert!(
            store
                .verify(&old_artifact)
                .await
                .expect("verify published blob"),
            "GC must re-stat the publisher-refreshed blob after waiting and retain it"
        );
        db.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn gc_unions_retired_and_active_scope_references_for_one_canonical_path() {
        let (mut db, _data_dir) = db_fixture().await;
        let root = tempfile::tempdir().expect("project root");
        let canonical_root = tokio::fs::canonicalize(root.path())
            .await
            .expect("canonical project root");
        let canonical_path = canonical_root.to_str().expect("UTF-8 project root");
        let (retired_scope_id, retired_revision_id) =
            create_report_revision(&db, canonical_path, "retired", &"1".repeat(64)).await;
        sqlx::query(
            r#"UPDATE project_scopes
                  SET retired_at=NOW(),row_version=row_version+1,updated_at=NOW()
                WHERE project_scope_id=$1"#,
        )
        .bind(retired_scope_id)
        .execute(db.pool())
        .await
        .expect("retire first project identity");
        let (_active_scope_id, active_revision_id) =
            create_report_revision(&db, canonical_path, "active", &"2".repeat(64)).await;

        let store =
            ProjectReportArtifactStore::with_orphan_grace(canonical_root.clone(), Duration::ZERO);
        let retired_reservation = store
            .promote(
                &store
                    .stage(
                        retired_revision_id,
                        ReportFormat::Markdown,
                        b"# retained historical report\n",
                    )
                    .await
                    .expect("stage retired-scope artifact"),
            )
            .await
            .expect("promote retired-scope artifact");
        let active_reservation = store
            .promote(
                &store
                    .stage(
                        active_revision_id,
                        ReportFormat::Json,
                        br#"{"active":true}"#,
                    )
                    .await
                    .expect("stage active-scope artifact"),
            )
            .await
            .expect("promote active-scope artifact");
        let retired_artifact = retired_reservation.artifact().clone();
        let active_artifact = active_reservation.artifact().clone();
        attach_artifact(&db, retired_revision_id, &retired_artifact).await;
        attach_artifact(&db, active_revision_id, &active_artifact).await;
        drop(retired_reservation);
        drop(active_reservation);

        let runtime = ReportArtifactGcRuntime::new(
            Arc::new(db.pool().clone()),
            Arc::new(ZeroGraceArtifactStoreFactory),
        );
        runtime
            .run_once(Utc::now() + chrono::Duration::seconds(1))
            .await
            .expect("run path-grouped report artifact GC");

        assert!(
            store
                .verify(&retired_artifact)
                .await
                .expect("verify retired-scope artifact"),
            "a retained historical scope reference must protect its blob"
        );
        assert!(
            store
                .verify(&active_artifact)
                .await
                .expect("verify active-scope artifact"),
            "an active scope reference sharing the path must protect its blob"
        );
        db.stop().await;
    }
}
