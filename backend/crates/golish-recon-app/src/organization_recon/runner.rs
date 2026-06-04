use std::net::IpAddr;
use std::path::{Path, PathBuf};

use golish_app_core::GolishError;
use golish_core::{emit_opt, EventEmitterHandle};
use serde_json::Value;
use uuid::Uuid;

use crate::asset_intel::{
    merge_candidates, run_providers_for_org, select_enrichment_providers,
    select_subsidiary_providers, AssetIntelRunStatus, ToolsConfigState,
};
use crate::organizations::{
    OrganizationCandidate, OrganizationCandidateKind, OrganizationCandidates,
};
use crate::targets::{db_target_list, Scope, Target};

use super::active::run_active_collection;
use super::artifacts::{write_json_manifest, write_raw_bytes, write_records_jsonl};
use super::export::write_recon_assets_workbook;
use super::normalize::{merge_normalized_records, normalize_record_key};
use super::persistence::persist_normalized_records;
use super::state::OrganizationReconState;
use super::types::{
    NormalizedReconRecord, OrganizationReconEvent, OrganizationReconRunSnapshot,
    OrganizationReconRunStatus, OrganizationReconStageName, OrganizationReconStageSnapshot,
    OrganizationReconStartArgs, OrganizationReconTaskSnapshot, ReconArtifactRef, ReconEvidenceRef,
    ReconRecordKind, ReconTaskError, ReconTaskManifest, ReconTaskStatus,
};

pub const ORGANIZATION_RECON_EVENT: &str = "organization-recon:event";

const ENTERPRISE_TASK: &str = "enterprise-intel";
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
        let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);

        if !args.allow_external {
            self.finish_task(
                run_id,
                &run_dir,
                OrganizationReconStageName::EnterpriseIntel,
                ENTERPRISE_TASK,
                ReconTaskStatus::Skipped,
                0,
                Vec::new(),
            )
            .await?;
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
            for (stage, task) in [
                (OrganizationReconStageName::EnterpriseIntel, ENTERPRISE_TASK),
                (OrganizationReconStageName::PassiveInternet, PASSIVE_TASK),
            ] {
                self.finish_task(
                    run_id,
                    &run_dir,
                    stage,
                    task,
                    ReconTaskStatus::Failed,
                    0,
                    vec![error.clone()],
                )
                .await?;
            }
        } else {
            self.run_asset_stage(
                run_id,
                &run_dir,
                OrganizationReconStageName::EnterpriseIntel,
                ENTERPRISE_TASK,
                organization,
                args,
                &pentest_config,
                &scan.tools,
                select_subsidiary_providers(&scan.tools, &args.provider_ids),
                &mut candidates,
                &mut errors,
            )
            .await?;
            self.run_asset_stage(
                run_id,
                &run_dir,
                OrganizationReconStageName::PassiveInternet,
                PASSIVE_TASK,
                organization,
                args,
                &pentest_config,
                &scan.tools,
                select_enrichment_providers(&scan.tools, &args.provider_ids),
                &mut candidates,
                &mut errors,
            )
            .await?;
        }

        let active_targets =
            in_scope_organization_targets(&self.pool, &organization.project_path, organization.id)
                .await?;
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
            let active_outcome = run_active_collection(
                &scan.tools,
                pentest_config.tools_dir(),
                run_id,
                &active_targets,
                &active_dir,
            )
            .await?;
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
        self.update_task(
            run_id,
            OrganizationReconStageName::Processing,
            PROCESSING_TASK,
            processing_manifest.status,
            records.len(),
            processing_manifest.artifacts.clone(),
            Vec::new(),
        )
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
                let count = result.candidates.organizations.len() + result.candidates.targets.len();
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
                self.finish_task(run_id, run_dir, stage, task_id, status, count, task_errors)
                    .await?;
            }
            Err(error) => {
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
        self.update_task(
            run_id,
            stage,
            task_id,
            ReconTaskStatus::Running,
            0,
            Vec::new(),
            Vec::new(),
        )
        .await;
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
        self.update_task(
            run_id,
            stage,
            task_id,
            status,
            record_count,
            artifacts,
            errors,
        )
        .await;
        Ok(())
    }

    async fn update_task(
        &self,
        run_id: &str,
        stage: OrganizationReconStageName,
        task_id: &str,
        status: ReconTaskStatus,
        record_count: usize,
        artifacts: Vec<ReconArtifactRef>,
        errors: Vec<ReconTaskError>,
    ) {
        let snapshot = self
            .state
            .update(run_id, |run| {
                run.updated_at = golish_core::time::now_ms();
                if let Some(task) = run.tasks.iter_mut().find(|task| task.task_id == task_id) {
                    task.status = status.clone();
                    task.record_count = record_count;
                    task.artifacts = artifacts;
                    task.errors = errors.clone();
                }
                for error in errors {
                    if !run.errors.contains(&error) {
                        run.errors.push(error);
                    }
                }
                if let Some(stage_snapshot) = run.stages.iter_mut().find(|item| item.stage == stage)
                {
                    stage_snapshot.status = status;
                }
            })
            .await;
        emit_snapshot(self.sink.as_ref(), snapshot);
    }

    async fn set_run_status(&self, run_id: &str, status: OrganizationReconRunStatus) {
        let snapshot = self
            .state
            .update(run_id, |run| {
                run.status = status;
                run.updated_at = golish_core::time::now_ms();
            })
            .await;
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
            .update(run_id, |run| {
                run.status = status;
                run.updated_at = golish_core::time::now_ms();
                if let Some(error) = error {
                    run.errors.push(error);
                }
            })
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
        task_snapshot(OrganizationReconStageName::EnterpriseIntel, ENTERPRISE_TASK),
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
        .filter_map(|candidate| normalized_candidate(run_id, candidate));
    let target_records = targets.iter().filter_map(|target| {
        let kind = record_kind(&target.value);
        normalized_record(run_id, "targets", PERSISTENCE_TASK, kind, &target.value)
    });
    let profile_records = organization_profile_records(run_id, organization, profile_artifact_path);
    merge_normalized_records(
        candidate_records
            .chain(target_records)
            .chain(profile_records),
    )
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
    fn initial_snapshot_has_the_five_stage_barrier_order() {
        let snapshot = initial_snapshot("run".into(), "org".into(), "/tmp/project".into());
        let stages: Vec<OrganizationReconStageName> = snapshot
            .stages
            .into_iter()
            .map(|stage| stage.stage)
            .collect();

        assert_eq!(
            stages,
            vec![
                OrganizationReconStageName::EnterpriseIntel,
                OrganizationReconStageName::PassiveInternet,
                OrganizationReconStageName::ActiveCollection,
                OrganizationReconStageName::Processing,
                OrganizationReconStageName::Persistence,
            ]
        );
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
                OrganizationReconStageName::EnterpriseIntel,
                ENTERPRISE_TASK,
                ReconTaskStatus::Completed,
                1,
                Vec::new(),
            ),
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
                ReconTaskStatus::Completed,
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
}
