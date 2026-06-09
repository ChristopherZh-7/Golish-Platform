use std::collections::HashSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use golish_app_core::GolishError;
use golish_core::{emit_opt, EventEmitterHandle};
use serde_json::Value;
use uuid::Uuid;

use crate::asset_intel::{
    merge_candidates, run_providers_for_org, select_asset_intel_providers, AssetIntelRunStatus,
    ToolsConfigState,
};
use crate::organizations::{
    OrganizationCandidate, OrganizationCandidateKind, OrganizationCandidates,
};
use crate::targets::{db_target_list, Scope, Target};

use super::active::{run_active_collection, ActiveCollectionLog};
use super::artifacts::{write_json_manifest, write_raw_bytes, write_records_jsonl};
use super::export::write_recon_assets_workbook;
use super::normalize::{merge_normalized_records, normalize_record_key};
use super::persistence::persist_normalized_records;
use super::state::{OrganizationReconState, RunStateUpdate, TaskProgressUpdate, TaskStateUpdate};
use super::types::{
    NormalizedReconRecord, OrganizationReconEvent, OrganizationReconRunSnapshot,
    OrganizationReconRunStatus, OrganizationReconStageName, OrganizationReconStageSnapshot,
    OrganizationReconStartArgs, OrganizationReconTaskSnapshot, ReconArtifactRef, ReconEvidenceRef,
    ReconRecordKind, ReconTaskError, ReconTaskManifest, ReconTaskStatus,
};

pub const ORGANIZATION_RECON_EVENT: &str = "organization-recon:event";

const PASSIVE_TASK: &str = "passive-internet";
const ACTIVE_TASK: &str = "active-collection";
const PROCESSING_TASK: &str = "processing";
const PERSISTENCE_TASK: &str = "persistence";

pub(crate) struct OrganizationReconRunner {
    pub(crate) pool: sqlx::PgPool,
    pub(crate) tools: ToolsConfigState,
    pub(crate) state: OrganizationReconState,
    pub(crate) sink: Option<EventEmitterHandle>,
}

impl OrganizationReconRunner {
    pub(crate) async fn run(
        self,
        run_id: String,
        args: OrganizationReconStartArgs,
        organization: golish_db::models::Organization,
    ) {
        let result = self.run_inner(&run_id, &args, &organization).await;
        if let Err(error) = result {
            let task_error = ReconTaskError::new("runner_error", error.to_string());
            self.finish_run(
                &run_id,
                OrganizationReconRunStatus::Failed,
                Some(task_error),
            )
            .await;
            let run_dir =
                organization_recon_run_dir(Path::new(&organization.project_path), &run_id);
            if let Err(manifest_error) = self.write_run_manifest(&run_id, &run_dir).await {
                tracing::warn!(
                    run_id,
                    error = %manifest_error,
                    "failed to persist organization recon failure manifest"
                );
            }
        }
    }

    async fn run_inner(
        &self,
        run_id: &str,
        args: &OrganizationReconStartArgs,
        organization: &golish_db::models::Organization,
    ) -> Result<(), GolishError> {
        self.set_run_status(run_id, OrganizationReconRunStatus::Running)
            .await;
        let project_root = PathBuf::from(&organization.project_path);
        let run_dir = organization_recon_run_dir(&project_root, run_id);
        std::fs::create_dir_all(&run_dir)?;

        let mut candidates = OrganizationCandidates::default();
        let mut errors = Vec::new();
        let pentest_config = self.tools.0.get().await;
        let scan = golish_pentest::scan_asset_intel_sources_with_status(
            &pentest_config.toolsconfig_dir,
            &pentest_config.intel_providers_dir,
            pentest_config.tools_dir(),
        );

        if !args.allow_external {
            self.finish_task(
                run_id,
                &run_dir,
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                ReconTaskStatus::Skipped,
                0,
                Vec::new(),
            )
            .await?;
        } else if !scan.success {
            let error = ReconTaskError::new(
                "toolsconfig_scan_failed",
                scan.error
                    .unwrap_or_else(|| "toolsconfig scan failed".into()),
            );
            errors.push(error.clone());
            self.finish_task(
                run_id,
                &run_dir,
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                ReconTaskStatus::Failed,
                0,
                vec![error],
            )
            .await?;
        } else {
            self.run_asset_stage(
                run_id,
                &run_dir,
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                organization,
                args,
                &pentest_config,
                &scan.tools,
                select_asset_intel_providers(&scan.tools, &args.provider_ids),
                &mut candidates,
                &mut errors,
            )
            .await?;
        }

        let scope_organization =
            golish_db::repo::organizations::get_one(&self.pool, organization.id)
                .await?
                .unwrap_or_else(|| organization.clone());
        let active_targets =
            in_scope_organization_targets(&self.pool, &organization.project_path, organization.id)
                .await?
                .into_iter()
                .filter(|target| {
                    target_value_belongs_to_organization(&scope_organization, &target.value)
                })
                .collect::<Vec<_>>();
        if !args.allow_active {
            self.finish_task(
                run_id,
                &run_dir,
                OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
                ReconTaskStatus::Skipped,
                0,
                Vec::new(),
            )
            .await?;
        } else {
            self.start_task(
                run_id,
                OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
            )
            .await;
            let active_dir = task_dir(
                &run_dir,
                &OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
            );
            let (log_tx, mut log_rx) =
                tokio::sync::mpsc::unbounded_channel::<ActiveCollectionLog>();
            let state = self.state.clone();
            let sink = self.sink.clone();
            let run_id_for_logs = run_id.to_string();
            let log_forwarder = tokio::spawn(async move {
                while let Some(log) = log_rx.recv().await {
                    let task_id = log.task_id.as_deref().unwrap_or(ACTIVE_TASK);
                    if let Some(progress) = active_task_progress_from_log(task_id, &log) {
                        let snapshot = state
                            .upsert_task_progress(TaskProgressUpdate {
                                run_id: &run_id_for_logs,
                                stage: OrganizationReconStageName::ActiveCollection,
                                task_id,
                                source_id: &progress.source_id,
                                status: progress.status,
                                record_count: progress.record_count,
                                errors: progress.errors,
                            })
                            .await;
                        emit_snapshot(sink.as_ref(), snapshot);
                    }
                    let snapshot = state
                        .append_task_log(
                            &run_id_for_logs,
                            OrganizationReconStageName::ActiveCollection,
                            task_id,
                            &log.level,
                            &log.message,
                        )
                        .await;
                    emit_snapshot(sink.as_ref(), snapshot);
                }
            });
            let active_outcome = run_active_collection(
                &scan.tools,
                pentest_config.tools_dir(),
                pentest_config.proxy_url.as_deref(),
                pentest_config.github_token.as_deref(),
                run_id,
                &active_targets,
                &active_dir,
                Some(log_tx),
            )
            .await?;
            if let Err(error) = log_forwarder.await {
                tracing::warn!(
                    run_id,
                    error = %error,
                    "organization_recon active log forwarder failed"
                );
            }
            if !active_outcome.errors.is_empty() {
                errors.extend(active_outcome.errors.clone());
            }
            self.finish_task_with_artifacts(
                run_id,
                &run_dir,
                OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
                active_outcome.status,
                active_outcome.record_count,
                active_outcome.errors,
                active_outcome.artifacts,
            )
            .await?;
        }

        self.start_task(
            run_id,
            OrganizationReconStageName::Processing,
            PROCESSING_TASK,
        )
        .await;
        let targets: Vec<Target> = db_target_list(&self.pool, Some(&organization.project_path))
            .await?
            .into_iter()
            .filter(|target| {
                target.organization_id.as_deref() == Some(&organization.id.to_string())
            })
            .collect();
        let processing_dir = task_dir(
            &run_dir,
            &OrganizationReconStageName::Processing,
            PROCESSING_TASK,
        );
        let refreshed_organization =
            golish_db::repo::organizations::get_one(&self.pool, organization.id)
                .await?
                .unwrap_or_else(|| organization.clone());
        let profile_snapshot =
            serde_json::to_vec_pretty(&refreshed_organization).map_err(|error| {
                GolishError::Internal(format!("serialize organization profile failed: {error}"))
            })?;
        let profile_artifact = write_raw_bytes(
            &processing_dir,
            "raw/organization-profile.json",
            &profile_snapshot,
            "organization_profile",
        )?;
        let records = normalized_records(
            run_id,
            &refreshed_organization,
            &candidates,
            &targets,
            &profile_artifact.path,
        );
        let records_artifact = write_records_jsonl(&processing_dir, &records)?;
        let workbook_artifact = write_recon_assets_workbook(&processing_dir, &records)?;
        let mut processing_manifest = task_manifest(
            run_id,
            OrganizationReconStageName::Processing,
            PROCESSING_TASK,
        );
        processing_manifest.status = if records.is_empty() {
            ReconTaskStatus::CheckedEmpty
        } else {
            ReconTaskStatus::Completed
        };
        processing_manifest.checked_empty = records.is_empty();
        processing_manifest.record_count = records.len();
        processing_manifest.artifacts.push(profile_artifact);
        processing_manifest.artifacts.push(records_artifact);
        processing_manifest.artifacts.push(workbook_artifact);
        write_json_manifest(&processing_dir, &processing_manifest)?;
        self.update_task(TaskStateUpdate {
            run_id,
            stage: OrganizationReconStageName::Processing,
            task_id: PROCESSING_TASK,
            status: processing_manifest.status,
            record_count: records.len(),
            artifacts: processing_manifest.artifacts.clone(),
            errors: Vec::new(),
        })
        .await;

        self.start_task(
            run_id,
            OrganizationReconStageName::Persistence,
            PERSISTENCE_TASK,
        )
        .await;
        let persistence_dir = task_dir(
            &run_dir,
            &OrganizationReconStageName::Persistence,
            PERSISTENCE_TASK,
        );
        let run_manifest_path = run_dir.join("manifest.json").display().to_string();
        let persistence_summary = persist_normalized_records(
            &self.pool,
            &refreshed_organization,
            run_id,
            &records,
            &run_manifest_path,
        )
        .await?;
        let persistence_summary_bytes =
            serde_json::to_vec_pretty(&persistence_summary).map_err(|error| {
                GolishError::Internal(format!("serialize persistence summary failed: {error}"))
            })?;
        let persistence_artifact = write_raw_bytes(
            &persistence_dir,
            "raw/persistence-summary.json",
            &persistence_summary_bytes,
            "persistence_summary",
        )?;
        self.finish_task_with_artifacts(
            run_id,
            &run_dir,
            OrganizationReconStageName::Persistence,
            PERSISTENCE_TASK,
            ReconTaskStatus::Completed,
            records.len(),
            Vec::new(),
            vec![persistence_artifact],
        )
        .await?;

        let status = if errors.is_empty() {
            OrganizationReconRunStatus::Completed
        } else {
            OrganizationReconRunStatus::Partial
        };
        self.finish_run(run_id, status, None).await;
        self.write_run_manifest(run_id, &run_dir).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_asset_stage(
        &self,
        run_id: &str,
        run_dir: &Path,
        stage: OrganizationReconStageName,
        task_id: &str,
        organization: &golish_db::models::Organization,
        args: &OrganizationReconStartArgs,
        pentest_config: &golish_pentest::config::PentestConfig,
        scan_tools: &[golish_pentest::models::ToolConfig],
        selected: Result<Vec<golish_pentest::models::ToolConfig>, GolishError>,
        candidates: &mut OrganizationCandidates,
        errors: &mut Vec<ReconTaskError>,
    ) -> Result<(), GolishError> {
        self.start_task(run_id, stage.clone(), task_id).await;
        let selected = match selected {
            Ok(selected) if !selected.is_empty() => selected,
            Ok(_) => {
                self.append_task_log(
                    run_id,
                    stage.clone(),
                    task_id,
                    "warning",
                    "passive_provider_plan: no provider selected; passive collection skipped",
                )
                .await;
                self.finish_task(
                    run_id,
                    run_dir,
                    stage,
                    task_id,
                    ReconTaskStatus::Skipped,
                    0,
                    Vec::new(),
                )
                .await?;
                return Ok(());
            }
            Err(error) => {
                let error = ReconTaskError::new("provider_select_error", error.to_string());
                self.append_task_log(
                    run_id,
                    stage.clone(),
                    task_id,
                    "error",
                    format!("passive_provider_select_failed: {}", error.message),
                )
                .await;
                errors.push(error.clone());
                self.finish_task(
                    run_id,
                    run_dir,
                    stage,
                    task_id,
                    ReconTaskStatus::Failed,
                    0,
                    vec![error],
                )
                .await?;
                return Ok(());
            }
        };
        let provider_summary = selected
            .iter()
            .map(|tool| tool.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        self.append_task_log(
            run_id,
            stage.clone(),
            task_id,
            "info",
            format!(
                "passive_provider_plan: selected {} provider(s): {}",
                selected.len(),
                provider_summary
            ),
        )
        .await;
        self.append_task_log(
            run_id,
            stage.clone(),
            task_id,
            "info",
            format!(
                "passive_provider_run: company={} allow_external={} providers={}",
                organization.name, args.allow_external, provider_summary
            ),
        )
        .await;

        match run_providers_for_org(
            self.sink.as_ref(),
            &self.pool,
            pentest_config,
            scan_tools,
            selected,
            organization,
            &organization.name,
            &args.config,
        )
        .await
        {
            Ok(result) => {
                let organization_count = result.candidates.organizations.len();
                let target_count = result.candidates.targets.len();
                let count = organization_count + target_count;
                self.append_task_log(
                    run_id,
                    stage.clone(),
                    task_id,
                    "info",
                    format!(
                        "passive_provider_finished: status={:?} organizations={} targets={} evidence_manifests={}",
                        result.status,
                        organization_count,
                        target_count,
                        result.evidence.len()
                    ),
                )
                .await;
                let artifacts = provider_manifest_artifacts(&result.evidence);
                merge_candidates(candidates, result.candidates);
                let (status, task_errors) = match result.status {
                    AssetIntelRunStatus::Completed => (
                        if count == 0 {
                            ReconTaskStatus::CheckedEmpty
                        } else {
                            ReconTaskStatus::Completed
                        },
                        Vec::new(),
                    ),
                    AssetIntelRunStatus::Partial | AssetIntelRunStatus::Failed => {
                        let task_errors = vec![ReconTaskError::new(
                            "asset_intel_failed",
                            format!("asset intel stage ended as {:?}", result.status),
                        )];
                        errors.extend(task_errors.clone());
                        (ReconTaskStatus::Failed, task_errors)
                    }
                };
                self.finish_task_with_artifacts(
                    run_id,
                    run_dir,
                    stage,
                    task_id,
                    status,
                    count,
                    task_errors,
                    artifacts,
                )
                .await?;
            }
            Err(error) => {
                self.append_task_log(
                    run_id,
                    stage.clone(),
                    task_id,
                    "error",
                    format!("passive_provider_failed: {error}"),
                )
                .await;
                let error = ReconTaskError::new("asset_intel_failed", error.to_string());
                errors.push(error.clone());
                self.finish_task(
                    run_id,
                    run_dir,
                    stage,
                    task_id,
                    ReconTaskStatus::Failed,
                    0,
                    vec![error],
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn start_task(&self, run_id: &str, stage: OrganizationReconStageName, task_id: &str) {
        let snapshot = self.state.start_task(run_id, stage, task_id).await;
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    async fn append_task_log(
        &self,
        run_id: &str,
        stage: OrganizationReconStageName,
        task_id: &str,
        level: impl Into<String>,
        message: impl Into<String>,
    ) {
        let snapshot = self
            .state
            .append_task_log(run_id, stage, task_id, level, message)
            .await;
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_task(
        &self,
        run_id: &str,
        run_dir: &Path,
        stage: OrganizationReconStageName,
        task_id: &str,
        status: ReconTaskStatus,
        record_count: usize,
        errors: Vec<ReconTaskError>,
    ) -> Result<(), GolishError> {
        self.finish_task_with_artifacts(
            run_id,
            run_dir,
            stage,
            task_id,
            status,
            record_count,
            errors,
            Vec::new(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_task_with_artifacts(
        &self,
        run_id: &str,
        run_dir: &Path,
        stage: OrganizationReconStageName,
        task_id: &str,
        status: ReconTaskStatus,
        record_count: usize,
        errors: Vec<ReconTaskError>,
        artifacts: Vec<ReconArtifactRef>,
    ) -> Result<(), GolishError> {
        let task_dir = task_dir(run_dir, &stage, task_id);
        let mut manifest = task_manifest(run_id, stage.clone(), task_id);
        manifest.status = status.clone();
        manifest.record_count = record_count;
        manifest.checked_empty = matches!(status, ReconTaskStatus::CheckedEmpty);
        manifest.errors = errors.clone();
        manifest.artifacts = artifacts.clone();
        write_json_manifest(&task_dir, &manifest)?;
        self.update_task(TaskStateUpdate {
            run_id,
            stage,
            task_id,
            status,
            record_count,
            artifacts,
            errors,
        })
        .await;
        Ok(())
    }

    async fn update_task(&self, update: TaskStateUpdate<'_>) {
        let snapshot = self.state.finish_task(update).await;
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    async fn set_run_status(&self, run_id: &str, status: OrganizationReconRunStatus) {
        let snapshot = if status == OrganizationReconRunStatus::Running {
            self.state.start_run(run_id).await
        } else {
            self.state
                .update(run_id, |run| {
                    run.status = status;
                    run.updated_at = golish_core::time::now_ms();
                })
                .await
        };
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    async fn finish_run(
        &self,
        run_id: &str,
        status: OrganizationReconRunStatus,
        error: Option<ReconTaskError>,
    ) {
        let snapshot = self
            .state
            .finish_run(run_id, RunStateUpdate { status, error })
            .await;
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    async fn write_run_manifest(&self, run_id: &str, run_dir: &Path) -> Result<(), GolishError> {
        let snapshot = self
            .state
            .get(run_id)
            .await
            .ok_or_else(|| GolishError::NotFound(format!("organization recon run {run_id}")))?;
        persist_run_manifest(run_dir, &snapshot)
    }
}

struct ActiveTaskProgress {
    source_id: String,
    status: ReconTaskStatus,
    record_count: usize,
    errors: Vec<ReconTaskError>,
}

fn active_task_progress_from_log(
    task_id: &str,
    log: &ActiveCollectionLog,
) -> Option<ActiveTaskProgress> {
    let message = log.message.as_str();
    if !message.starts_with("active_tool_") {
        return None;
    }
    let source_id = extract_log_field(message, "tool=")
        .unwrap_or(task_id)
        .to_string();
    let record_count = extract_log_field(message, "records=")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();

    if message.starts_with("active_tool_finished:") {
        let status = match extract_log_field(message, "status=") {
            Some("Completed") => ReconTaskStatus::Completed,
            Some("CheckedEmpty") => ReconTaskStatus::CheckedEmpty,
            Some("Failed") => ReconTaskStatus::Failed,
            _ if log.level == "error" => ReconTaskStatus::Failed,
            _ => ReconTaskStatus::Completed,
        };
        let errors = if status == ReconTaskStatus::Failed {
            vec![ReconTaskError::new("active_tool_failed", message)]
        } else {
            Vec::new()
        };
        return Some(ActiveTaskProgress {
            source_id,
            status,
            record_count,
            errors,
        });
    }

    if message.starts_with("active_tool_checked_empty:") {
        return Some(ActiveTaskProgress {
            source_id,
            status: ReconTaskStatus::CheckedEmpty,
            record_count,
            errors: Vec::new(),
        });
    }

    if is_active_tool_failure_log(message) {
        let code = message
            .split_once(':')
            .map(|(code, _)| code)
            .unwrap_or("active_tool_failed");
        return Some(ActiveTaskProgress {
            source_id,
            status: ReconTaskStatus::Failed,
            record_count,
            errors: vec![ReconTaskError::new(code, message)],
        });
    }

    if is_active_tool_running_log(message) {
        return Some(ActiveTaskProgress {
            source_id,
            status: ReconTaskStatus::Running,
            record_count,
            errors: Vec::new(),
        });
    }

    None
}

fn is_active_tool_running_log(message: &str) -> bool {
    [
        "active_tool_auto_install_",
        "active_tool_managed_executable_found:",
        "active_tool_running:",
        "active_tool_spawn:",
        "active_tool_stdout:",
        "active_tool_stderr:",
        "active_tool_stream_read_failed:",
        "active_tool_validation_failed:",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

fn is_active_tool_failure_log(message: &str) -> bool {
    [
        "active_tool_config_missing:",
        "active_tool_install_unavailable:",
        "active_tool_auto_install_failed:",
        "active_tool_spawn_failed:",
        "active_tool_wait_failed:",
        "active_tool_timeout:",
        "active_tool_nonzero_exit:",
        "active_tool_output_decode_failed:",
        "active_tool_output_parse_failed:",
    ]
    .iter()
    .any(|prefix| message.starts_with(prefix))
}

fn extract_log_field<'a>(message: &'a str, field: &str) -> Option<&'a str> {
    let start = message.find(field)? + field.len();
    let rest = &message[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(&rest[..end])
}

fn persist_run_manifest(
    run_dir: &Path,
    snapshot: &OrganizationReconRunSnapshot,
) -> Result<(), GolishError> {
    let bytes = serde_json::to_vec_pretty(snapshot).map_err(|error| {
        GolishError::Internal(format!("serialize organization recon run failed: {error}"))
    })?;
    write_raw_bytes(run_dir, "manifest.json", &bytes, "run_manifest")?;
    Ok(())
}

pub(crate) fn initial_snapshot(
    run_id: String,
    organization_id: String,
    project_path: String,
) -> OrganizationReconRunSnapshot {
    let created_at = golish_core::time::now_ms();
    let tasks = vec![
        task_snapshot(OrganizationReconStageName::PassiveInternet, PASSIVE_TASK),
        task_snapshot(OrganizationReconStageName::ActiveCollection, ACTIVE_TASK),
        task_snapshot(OrganizationReconStageName::Processing, PROCESSING_TASK),
        task_snapshot(OrganizationReconStageName::Persistence, PERSISTENCE_TASK),
    ];
    OrganizationReconRunSnapshot {
        run_id,
        organization_id,
        project_path,
        status: OrganizationReconRunStatus::Queued,
        stages: tasks
            .iter()
            .map(|task| OrganizationReconStageSnapshot {
                stage: task.stage.clone(),
                status: ReconTaskStatus::Queued,
                task_ids: vec![task.task_id.clone()],
            })
            .collect(),
        tasks,
        errors: Vec::new(),
        trace_events: Vec::new(),
        created_at,
        updated_at: created_at,
    }
}

pub(crate) async fn in_scope_organization_targets(
    pool: &sqlx::PgPool,
    project_path: &str,
    organization_id: Uuid,
) -> Result<Vec<Target>, GolishError> {
    Ok(db_target_list(pool, Some(project_path))
        .await?
        .into_iter()
        .filter(|target| {
            target.organization_id.as_deref() == Some(&organization_id.to_string())
                && target.scope == Scope::InScope
        })
        .collect())
}

fn task_snapshot(
    stage: OrganizationReconStageName,
    task_id: &str,
) -> OrganizationReconTaskSnapshot {
    OrganizationReconTaskSnapshot {
        task_id: task_id.into(),
        stage,
        source_id: task_id.into(),
        status: ReconTaskStatus::Queued,
        record_count: 0,
        artifacts: Vec::new(),
        errors: Vec::new(),
    }
}

fn task_manifest(
    run_id: &str,
    stage: OrganizationReconStageName,
    task_id: &str,
) -> ReconTaskManifest {
    ReconTaskManifest::new(run_id, task_id, stage_label(&stage), task_id)
}

fn task_dir(run_dir: &Path, stage: &OrganizationReconStageName, task_id: &str) -> PathBuf {
    run_dir.join(stage_label(stage)).join(task_id)
}

fn organization_recon_run_dir(project_root: &Path, run_id: &str) -> PathBuf {
    golish_projects::file_storage::tool_output_dir(project_root, "recon").join(run_id)
}

fn stage_label(stage: &OrganizationReconStageName) -> &'static str {
    match stage {
        OrganizationReconStageName::EnterpriseIntel => "enterprise_intel",
        OrganizationReconStageName::PassiveInternet => "passive_internet",
        OrganizationReconStageName::ActiveCollection => "active_collection",
        OrganizationReconStageName::Processing => "processing",
        OrganizationReconStageName::Persistence => "persistence",
    }
}

fn normalized_records(
    run_id: &str,
    organization: &golish_db::models::Organization,
    candidates: &OrganizationCandidates,
    targets: &[Target],
    profile_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    let candidate_records = candidates
        .targets
        .iter()
        .filter_map(|candidate| normalized_candidate(run_id, candidate))
        .filter(|record| recon_record_belongs_to_organization(organization, record));
    let target_records = targets
        .iter()
        .filter_map(|target| {
            let kind = record_kind(&target.value);
            normalized_record(run_id, "targets", PERSISTENCE_TASK, kind, &target.value)
        })
        .filter(|record| recon_record_belongs_to_organization(organization, record));
    let profile_records = organization_profile_records(run_id, organization, profile_artifact_path)
        .into_iter()
        .filter(|record| recon_record_belongs_to_organization(organization, record));
    merge_normalized_records(
        candidate_records
            .chain(target_records)
            .chain(profile_records),
    )
}

fn recon_record_belongs_to_organization(
    organization: &golish_db::models::Organization,
    record: &NormalizedReconRecord,
) -> bool {
    match record.kind {
        ReconRecordKind::Domain | ReconRecordKind::Url | ReconRecordKind::Site => {
            target_value_belongs_to_organization(organization, &record.value)
        }
        _ => true,
    }
}

fn target_value_belongs_to_organization(
    organization: &golish_db::models::Organization,
    value: &str,
) -> bool {
    if value.trim().parse::<IpAddr>().is_ok() {
        return true;
    }
    let Some(host) = normalized_host(value) else {
        return false;
    };
    if is_known_public_non_asset_host(&host) {
        return false;
    }
    let domains = organization_owned_domains(organization);
    if domains.is_empty() {
        return false;
    }
    domains
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn organization_owned_domains(organization: &golish_db::models::Organization) -> HashSet<String> {
    let mut domains = HashSet::new();
    collect_owned_domain_values(&mut domains, &organization.domains);
    if let Some(intel) = organization.intel.as_object() {
        if let Some(value) = intel.get("app_domains") {
            collect_owned_domain_values(&mut domains, value);
        }
    }
    domains
}

fn collect_owned_domain_values(domains: &mut HashSet<String>, value: &Value) {
    for item in json_atom_values(value) {
        if let Some(host) = normalized_host(&item) {
            if !is_known_public_non_asset_host(&host) {
                domains.insert(host);
            }
        }
    }
}

fn normalized_host(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if value.is_empty() || value.parse::<IpAddr>().is_ok() {
        return None;
    }
    if let Ok(url) = url::Url::parse(&value) {
        return url
            .host_str()
            .map(|host| host.trim_start_matches("www.").to_string());
    }
    if looks_like_domain(&value) {
        return Some(value.trim_start_matches("www.").to_string());
    }
    None
}

fn is_known_public_non_asset_host(host: &str) -> bool {
    const PUBLIC_HOSTS: &[&str] = &[
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "gitee.com",
        "126.com",
        "163.com",
        "gmail.com",
        "hotmail.com",
        "outlook.com",
        "qq.com",
    ];
    PUBLIC_HOSTS
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

pub(crate) fn normalized_current_asset_records(
    run_id: &str,
    organization: &golish_db::models::Organization,
    targets: &[Target],
    profile_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    normalized_records(
        run_id,
        organization,
        &OrganizationCandidates::default(),
        targets,
        profile_artifact_path,
    )
}

fn normalized_candidate(
    run_id: &str,
    candidate: &OrganizationCandidate,
) -> Option<NormalizedReconRecord> {
    if !matches!(&candidate.kind, OrganizationCandidateKind::Target) {
        return None;
    }
    let kind = record_kind(&candidate.value);
    normalized_record(
        run_id,
        &candidate.source,
        PROCESSING_TASK,
        kind,
        &candidate.value,
    )
}

fn organization_profile_records(
    run_id: &str,
    organization: &golish_db::models::Organization,
    profile_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    let mut records = Vec::new();
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Domain,
        "domains",
        &organization.domains,
        profile_artifact_path,
    );
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Ip,
        "ip_ranges",
        &organization.ip_ranges,
        profile_artifact_path,
    );
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Domain,
        "email_domains",
        &organization.email_domains,
        profile_artifact_path,
    );
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Certificate,
        "certificates",
        &organization.certificates,
        profile_artifact_path,
    );
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Wechat,
        "social_accounts",
        &organization.social_accounts,
        profile_artifact_path,
    );
    push_profile_values(
        &mut records,
        run_id,
        ReconRecordKind::Leak,
        "historical_vulns",
        &organization.historical_vulns,
        profile_artifact_path,
    );
    push_profile_asset_values(
        &mut records,
        run_id,
        "business_systems",
        &organization.business_systems,
        profile_artifact_path,
    );
    push_profile_asset_values(
        &mut records,
        run_id,
        "cloud_assets",
        &organization.cloud_assets,
        profile_artifact_path,
    );
    push_profile_asset_values(
        &mut records,
        run_id,
        "github_orgs",
        &organization.github_orgs,
        profile_artifact_path,
    );
    push_contact_values(
        &mut records,
        run_id,
        "contacts",
        &organization.contacts,
        profile_artifact_path,
    );

    if let Some(intel) = organization.intel.as_object() {
        if let Some(value) = intel.get("mobile_apps") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::App,
                "intel.mobile_apps",
                value,
                profile_artifact_path,
            );
        }
        if let Some(value) = intel.get("mini_programs") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::MiniProgram,
                "intel.mini_programs",
                value,
                profile_artifact_path,
            );
        }
        if let Some(value) = intel.get("app_domains") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::Domain,
                "intel.app_domains",
                value,
                profile_artifact_path,
            );
        }
        if let Some(value) = intel.get("exposed_emails") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::Contact,
                "intel.exposed_emails",
                value,
                profile_artifact_path,
            );
        }
        if let Some(value) = intel.get("code_leaks") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::Leak,
                "intel.code_leaks",
                value,
                profile_artifact_path,
            );
        }
        if let Some(value) = intel.get("mail_mx") {
            push_profile_values(
                &mut records,
                run_id,
                ReconRecordKind::Domain,
                "intel.mail_mx",
                value,
                profile_artifact_path,
            );
        }
        for (field, value) in [
            ("intel.quake_http_titles", intel.get("quake_http_titles")),
            ("intel.quake_http_servers", intel.get("quake_http_servers")),
            ("intel.quake_services", intel.get("quake_services")),
        ] {
            if let Some(value) = value {
                push_profile_values(
                    &mut records,
                    run_id,
                    ReconRecordKind::Service,
                    field,
                    value,
                    profile_artifact_path,
                );
            }
        }
        if let Some(value) = intel.get("contacts") {
            push_contact_values(
                &mut records,
                run_id,
                "intel.contacts",
                value,
                profile_artifact_path,
            );
        }
    }

    records
}

fn provider_manifest_artifacts(evidence: &[Value]) -> Vec<ReconArtifactRef> {
    evidence
        .iter()
        .filter_map(provider_manifest_artifact)
        .collect()
}

fn provider_manifest_artifact(evidence: &Value) -> Option<ReconArtifactRef> {
    let path = evidence.get("manifestPath")?.as_str()?;
    let bytes = std::fs::metadata(path).ok()?.len();
    Some(ReconArtifactRef {
        path: path.to_string(),
        kind: "provider_manifest".into(),
        bytes,
    })
}

fn push_profile_asset_values(
    records: &mut Vec<NormalizedReconRecord>,
    run_id: &str,
    field: &str,
    value: &Value,
    profile_artifact_path: &str,
) {
    for item in json_atom_values(value) {
        let kind = record_kind(&item);
        push_profile_record(records, run_id, kind, field, &item, profile_artifact_path);
    }
}

fn push_profile_values(
    records: &mut Vec<NormalizedReconRecord>,
    run_id: &str,
    kind: ReconRecordKind,
    field: &str,
    value: &Value,
    profile_artifact_path: &str,
) {
    for item in json_atom_values(value) {
        push_profile_record(
            records,
            run_id,
            kind.clone(),
            field,
            &item,
            profile_artifact_path,
        );
    }
}

fn push_contact_values(
    records: &mut Vec<NormalizedReconRecord>,
    run_id: &str,
    field: &str,
    value: &Value,
    profile_artifact_path: &str,
) {
    match value {
        Value::Object(map) => {
            for (channel, values) in map {
                for item in json_atom_values(values) {
                    push_profile_record(
                        records,
                        run_id,
                        ReconRecordKind::Contact,
                        &format!("{field}.{channel}"),
                        &item,
                        profile_artifact_path,
                    );
                }
            }
        }
        _ => push_profile_values(
            records,
            run_id,
            ReconRecordKind::Contact,
            field,
            value,
            profile_artifact_path,
        ),
    }
}

fn push_profile_record(
    records: &mut Vec<NormalizedReconRecord>,
    run_id: &str,
    kind: ReconRecordKind,
    field: &str,
    value: &str,
    profile_artifact_path: &str,
) {
    if let Some(record) = normalized_record_with_attributes(
        run_id,
        "organization_profile",
        PROCESSING_TASK,
        kind,
        value,
        serde_json::json!({ "profileField": field }),
        profile_artifact_path,
    ) {
        records.push(record);
    }
}

fn json_atom_values(value: &Value) -> Vec<String> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => non_empty_value(text).into_iter().collect(),
        Value::Number(number) => non_empty_value(&number.to_string()).into_iter().collect(),
        Value::Bool(boolean) => non_empty_value(&boolean.to_string()).into_iter().collect(),
        Value::Array(items) => items.iter().flat_map(json_atom_values).collect(),
        Value::Object(map) => display_json_atom(value)
            .or_else(|| {
                map.values()
                    .flat_map(json_atom_values)
                    .find(|item| !item.trim().is_empty())
            })
            .into_iter()
            .collect(),
    }
}

fn display_json_atom(value: &Value) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    for key in [
        "domain",
        "url",
        "host",
        "ip",
        "name",
        "value",
        "app_id",
        "wechat_id",
        "id",
    ] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            if let Some(value) = non_empty_value(value) {
                return Some(value);
            }
        }
    }
    None
}

fn non_empty_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "-" {
        None
    } else {
        Some(value.into())
    }
}

fn normalized_record(
    run_id: &str,
    source_id: &str,
    task_id: &str,
    kind: ReconRecordKind,
    value: &str,
) -> Option<NormalizedReconRecord> {
    normalized_record_with_attributes(
        run_id,
        source_id,
        task_id,
        kind,
        value,
        Value::Null,
        &format!("{}/manifest.json", source_id),
    )
}

fn normalized_record_with_attributes(
    run_id: &str,
    source_id: &str,
    task_id: &str,
    kind: ReconRecordKind,
    value: &str,
    attributes: Value,
    raw_artifact_path: &str,
) -> Option<NormalizedReconRecord> {
    let key = normalize_record_key(&kind, value).ok()?;
    Some(NormalizedReconRecord {
        record_id: key.clone(),
        kind,
        key,
        value: value.into(),
        attributes,
        evidence: vec![ReconEvidenceRef {
            source_id: source_id.into(),
            run_id: run_id.into(),
            task_id: task_id.into(),
            raw_artifact_path: raw_artifact_path.into(),
        }],
    })
}

fn record_kind(value: &str) -> ReconRecordKind {
    if value.parse::<IpAddr>().is_ok() {
        ReconRecordKind::Ip
    } else if url::Url::parse(value).is_ok() {
        ReconRecordKind::Url
    } else if looks_like_domain(value) {
        ReconRecordKind::Domain
    } else {
        ReconRecordKind::Site
    }
}

fn looks_like_domain(value: &str) -> bool {
    let value = value.trim().trim_end_matches('.');
    if value.contains(char::is_whitespace) || !value.contains('.') {
        return false;
    }
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

fn emit_snapshot(
    sink: Option<&EventEmitterHandle>,
    snapshot: Option<OrganizationReconRunSnapshot>,
) {
    if let Some(run) = snapshot {
        emit_opt(
            sink,
            ORGANIZATION_RECON_EVENT,
            &OrganizationReconEvent { run },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_snapshot_has_the_four_stage_barrier_order() {
        let snapshot = initial_snapshot("run".into(), "org".into(), "/tmp/project".into());
        let stages: Vec<OrganizationReconStageName> = snapshot
            .stages
            .into_iter()
            .map(|stage| stage.stage)
            .collect();

        assert_eq!(
            stages,
            vec![
                OrganizationReconStageName::PassiveInternet,
                OrganizationReconStageName::ActiveCollection,
                OrganizationReconStageName::Processing,
                OrganizationReconStageName::Persistence,
            ]
        );
    }

    #[test]
    fn normalized_records_drop_public_code_leak_urls_from_targets() {
        let now = chrono::Utc::now();
        let organization = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "中国平安".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: serde_json::json!([{ "domain": "pingan.com.cn" }]),
            ip_ranges: serde_json::json!([]),
            asns: serde_json::json!([]),
            email_domains: serde_json::json!(["126.com"]),
            scope_rules: serde_json::json!({}),
            intel: serde_json::json!({
                "code_leaks": ["https://github.com/example/leak/blob/main/key.txt"],
                "app_domains": ["app.pingan.com.cn"]
            }),
            notes: String::new(),
            certificates: serde_json::json!([]),
            subsidiaries: serde_json::json!([]),
            business_systems: serde_json::json!([]),
            cloud_assets: serde_json::json!([]),
            github_orgs: serde_json::json!(["https://github.com/pingan"]),
            social_accounts: serde_json::json!([]),
            historical_vulns: serde_json::json!([]),
            contacts: serde_json::json!([]),
            created_at: now,
            updated_at: now,
        };
        let candidates = OrganizationCandidates {
            targets: vec![
                OrganizationCandidate {
                    id: String::new(),
                    kind: OrganizationCandidateKind::Target,
                    label: "GitHub leak".into(),
                    value: "https://github.com/example/leak/blob/main/key.txt".into(),
                    source: "0.zone".into(),
                    confidence: 0.9,
                    status: String::new(),
                    evidence: Value::Null,
                    created_at: 0,
                },
                OrganizationCandidate {
                    id: String::new(),
                    kind: OrganizationCandidateKind::Target,
                    label: "Owned".into(),
                    value: "https://www.pingan.com.cn/".into(),
                    source: "0.zone".into(),
                    confidence: 0.9,
                    status: String::new(),
                    evidence: Value::Null,
                    created_at: 0,
                },
            ],
            ..OrganizationCandidates::default()
        };

        let records = normalized_records("run", &organization, &candidates, &[], "profile.json");
        let values = records
            .iter()
            .map(|record| record.value.as_str())
            .collect::<Vec<_>>();

        assert!(values.contains(&"https://www.pingan.com.cn/"));
        assert!(values.contains(&"https://github.com/example/leak/blob/main/key.txt"));
        assert!(!records.iter().any(|record| {
            matches!(record.kind, ReconRecordKind::Url | ReconRecordKind::Domain)
                && record.value.contains("github.com")
        }));
        assert!(!values.contains(&"126.com"));
    }

    #[test]
    fn provider_manifest_path_becomes_stage_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("manifest.json");
        std::fs::write(&manifest_path, br#"{"status":"completed"}"#).unwrap();

        let artifacts = provider_manifest_artifacts(&[serde_json::json!({
            "provider": "quake",
            "manifestPath": manifest_path,
        })]);

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "provider_manifest");
        assert!(artifacts[0].bytes > 0);
    }

    #[test]
    fn normalized_records_include_quake_service_intel() {
        let now = chrono::Utc::now();
        let organization = golish_db::models::Organization {
            id: Uuid::new_v4(),
            project_path: "/tmp/project".into(),
            name: "中国平安".into(),
            parent_id: None,
            description: String::new(),
            owner: String::new(),
            sort_order: 0,
            aliases: Vec::new(),
            industry: String::new(),
            tier: String::new(),
            credit_code: String::new(),
            domains: serde_json::json!([]),
            ip_ranges: serde_json::json!([]),
            asns: serde_json::json!([]),
            email_domains: serde_json::json!([]),
            scope_rules: serde_json::json!({}),
            intel: serde_json::json!({
                "quake_http_titles": ["平安官网"],
                "quake_http_servers": ["nginx"],
                "quake_services": ["https"]
            }),
            notes: String::new(),
            certificates: serde_json::json!([]),
            subsidiaries: serde_json::json!([]),
            business_systems: serde_json::json!([]),
            cloud_assets: serde_json::json!([]),
            github_orgs: serde_json::json!([]),
            social_accounts: serde_json::json!([]),
            historical_vulns: serde_json::json!([]),
            contacts: serde_json::json!([]),
            created_at: now,
            updated_at: now,
        };

        let records = normalized_records(
            "run",
            &organization,
            &OrganizationCandidates::default(),
            &[],
            "profile.json",
        );

        let services = records
            .iter()
            .filter(|record| record.kind == ReconRecordKind::Service)
            .map(|record| record.value.as_str())
            .collect::<Vec<_>>();
        assert!(services.contains(&"平安官网"));
        assert!(services.contains(&"nginx"));
        assert!(services.contains(&"https"));
    }

    #[tokio::test]
    async fn staged_fixture_persists_partial_run_with_checked_empty_evidence() {
        let state = OrganizationReconState::default();
        let run_id = "fixture-run";
        state
            .insert(initial_snapshot(
                run_id.into(),
                "fixture-org".into(),
                "/tmp/fixture-project".into(),
            ))
            .await;
        let run_dir = tempfile::tempdir().unwrap();
        let passive_error = ReconTaskError::new("fixture_passive_failed", "fixture quota exceeded");
        let fixtures = [
            (
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                ReconTaskStatus::Failed,
                0,
                vec![passive_error.clone()],
            ),
            (
                OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
                ReconTaskStatus::CheckedEmpty,
                0,
                Vec::new(),
            ),
            (
                OrganizationReconStageName::Processing,
                PROCESSING_TASK,
                ReconTaskStatus::Completed,
                1,
                Vec::new(),
            ),
            (
                OrganizationReconStageName::Persistence,
                PERSISTENCE_TASK,
                ReconTaskStatus::Completed,
                1,
                Vec::new(),
            ),
        ];

        for (stage, task_id, status, record_count, errors) in fixtures {
            let mut manifest = task_manifest(run_id, stage.clone(), task_id);
            manifest.status = status.clone();
            manifest.record_count = record_count;
            manifest.checked_empty = matches!(status, ReconTaskStatus::CheckedEmpty);
            manifest.errors = errors.clone();
            write_json_manifest(&task_dir(run_dir.path(), &stage, task_id), &manifest).unwrap();
            state
                .update(run_id, |run| {
                    let task = run
                        .tasks
                        .iter_mut()
                        .find(|task| task.task_id == task_id)
                        .unwrap();
                    task.status = status.clone();
                    task.record_count = record_count;
                    task.errors = errors.clone();
                    run.stages
                        .iter_mut()
                        .find(|item| item.stage == stage)
                        .unwrap()
                        .status = status.clone();
                    run.errors.extend(errors.clone());
                })
                .await
                .unwrap();
        }

        let snapshot = state
            .update(run_id, |run| {
                run.status = OrganizationReconRunStatus::Partial
            })
            .await
            .unwrap();
        persist_run_manifest(run_dir.path(), &snapshot).unwrap();

        let persisted: OrganizationReconRunSnapshot =
            serde_json::from_slice(&std::fs::read(run_dir.path().join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(persisted.status, OrganizationReconRunStatus::Partial);
        assert_eq!(persisted.errors, vec![passive_error]);
        assert_eq!(
            persisted
                .stages
                .iter()
                .map(|stage| stage.status.clone())
                .collect::<Vec<_>>(),
            vec![
                ReconTaskStatus::Failed,
                ReconTaskStatus::CheckedEmpty,
                ReconTaskStatus::Completed,
                ReconTaskStatus::Completed,
            ]
        );
        let active_manifest: ReconTaskManifest = serde_json::from_slice(
            &std::fs::read(
                task_dir(
                    run_dir.path(),
                    &OrganizationReconStageName::ActiveCollection,
                    ACTIVE_TASK,
                )
                .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(active_manifest.checked_empty);
    }

    #[tokio::test]
    async fn staged_fixture_can_finish_all_four_stages_without_active_scope() {
        let state = OrganizationReconState::default();
        let run_id = "fixture-complete-run";
        state
            .insert(initial_snapshot(
                run_id.into(),
                "fixture-org".into(),
                "/tmp/fixture-project".into(),
            ))
            .await;
        let run_dir = tempfile::tempdir().unwrap();
        let fixtures = [
            (
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                ReconTaskStatus::Completed,
                5,
            ),
            (
                OrganizationReconStageName::ActiveCollection,
                ACTIVE_TASK,
                ReconTaskStatus::CheckedEmpty,
                0,
            ),
            (
                OrganizationReconStageName::Processing,
                PROCESSING_TASK,
                ReconTaskStatus::Completed,
                8,
            ),
            (
                OrganizationReconStageName::Persistence,
                PERSISTENCE_TASK,
                ReconTaskStatus::Completed,
                8,
            ),
        ];

        for (stage, task_id, status, record_count) in fixtures {
            let mut manifest = task_manifest(run_id, stage.clone(), task_id);
            manifest.status = status.clone();
            manifest.record_count = record_count;
            manifest.checked_empty = matches!(status, ReconTaskStatus::CheckedEmpty);
            write_json_manifest(&task_dir(run_dir.path(), &stage, task_id), &manifest).unwrap();
            state
                .update(run_id, |run| {
                    let task = run
                        .tasks
                        .iter_mut()
                        .find(|task| task.task_id == task_id)
                        .unwrap();
                    task.status = status.clone();
                    task.record_count = record_count;
                    run.stages
                        .iter_mut()
                        .find(|item| item.stage == stage)
                        .unwrap()
                        .status = status.clone();
                })
                .await
                .unwrap();
        }

        let snapshot = state
            .update(run_id, |run| {
                run.status = OrganizationReconRunStatus::Completed
            })
            .await
            .unwrap();
        persist_run_manifest(run_dir.path(), &snapshot).unwrap();

        let persisted: OrganizationReconRunSnapshot =
            serde_json::from_slice(&std::fs::read(run_dir.path().join("manifest.json")).unwrap())
                .unwrap();

        assert_eq!(persisted.status, OrganizationReconRunStatus::Completed);
        assert!(persisted.errors.is_empty());
        assert_eq!(persisted.stages.len(), 4);
        assert_eq!(
            persisted
                .stages
                .iter()
                .map(|stage| stage.status.clone())
                .collect::<Vec<_>>(),
            vec![
                ReconTaskStatus::Completed,
                ReconTaskStatus::CheckedEmpty,
                ReconTaskStatus::Completed,
                ReconTaskStatus::Completed,
            ]
        );
    }
}
