use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::types::{
    OrganizationReconRunSnapshot, OrganizationReconRunStatus, OrganizationReconStageName,
    OrganizationReconTaskSnapshot, OrganizationReconTraceEvent, OrganizationReconTraceKind,
    ReconArtifactRef, ReconTaskError, ReconTaskStatus,
};

const MAX_TRACE_EVENTS: usize = 200;

#[derive(Debug, Clone)]
pub(crate) struct TaskStateUpdate<'a> {
    pub run_id: &'a str,
    pub stage: OrganizationReconStageName,
    pub task_id: &'a str,
    pub status: ReconTaskStatus,
    pub record_count: usize,
    pub artifacts: Vec<ReconArtifactRef>,
    pub errors: Vec<ReconTaskError>,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskProgressUpdate<'a> {
    pub run_id: &'a str,
    pub stage: OrganizationReconStageName,
    pub task_id: &'a str,
    pub source_id: &'a str,
    pub status: ReconTaskStatus,
    pub record_count: usize,
    pub errors: Vec<ReconTaskError>,
}

#[derive(Debug, Clone)]
pub(crate) struct RunStateUpdate {
    pub status: OrganizationReconRunStatus,
    pub error: Option<ReconTaskError>,
}

#[derive(Debug, Clone, Default)]
pub struct OrganizationReconState {
    runs: Arc<RwLock<HashMap<String, OrganizationReconRunSnapshot>>>,
}

impl OrganizationReconState {
    pub(crate) async fn insert(&self, run: OrganizationReconRunSnapshot) {
        self.runs.write().await.insert(run.run_id.clone(), run);
    }

    pub async fn get(&self, run_id: &str) -> Option<OrganizationReconRunSnapshot> {
        self.runs.read().await.get(run_id).cloned()
    }

    pub(crate) async fn update(
        &self,
        run_id: &str,
        update: impl FnOnce(&mut OrganizationReconRunSnapshot),
    ) -> Option<OrganizationReconRunSnapshot> {
        let mut runs = self.runs.write().await;
        let run = runs.get_mut(run_id)?;
        update(run);
        Some(run.clone())
    }

    pub(crate) async fn start_run(&self, run_id: &str) -> Option<OrganizationReconRunSnapshot> {
        self.update(run_id, |run| {
            let was_started = run
                .trace_events
                .iter()
                .any(|event| event.kind == OrganizationReconTraceKind::RunStarted);
            run.status = OrganizationReconRunStatus::Running;
            run.updated_at = golish_core::time::now_ms();
            if !was_started {
                push_trace(
                    run,
                    OrganizationReconTraceKind::RunStarted,
                    None,
                    None,
                    None,
                    "info",
                    "Organization Recon run started",
                );
            }
        })
        .await
    }

    pub(crate) async fn start_task(
        &self,
        run_id: &str,
        stage: OrganizationReconStageName,
        task_id: &str,
    ) -> Option<OrganizationReconRunSnapshot> {
        self.update(run_id, |run| {
            run.status = OrganizationReconRunStatus::Running;
            run.updated_at = golish_core::time::now_ms();
            if let Some(task) = run.tasks.iter_mut().find(|task| task.task_id == task_id) {
                task.status = ReconTaskStatus::Running;
                task.record_count = 0;
                task.artifacts.clear();
                task.errors.clear();
            }
            if let Some(stage_snapshot) = run.stages.iter_mut().find(|item| item.stage == stage) {
                stage_snapshot.status = ReconTaskStatus::Running;
            }
            push_trace(
                run,
                OrganizationReconTraceKind::StepStarted,
                Some(stage),
                Some(task_id.to_string()),
                Some(ReconTaskStatus::Running),
                "info",
                format!("Step {task_id} started"),
            );
        })
        .await
    }

    pub(crate) async fn finish_task(
        &self,
        update: TaskStateUpdate<'_>,
    ) -> Option<OrganizationReconRunSnapshot> {
        self.update(update.run_id, |run| {
            run.updated_at = golish_core::time::now_ms();
            if let Some(task) = run
                .tasks
                .iter_mut()
                .find(|task| task.task_id == update.task_id)
            {
                task.status = update.status.clone();
                task.record_count = update.record_count;
                task.artifacts = update.artifacts.clone();
                task.errors = update.errors.clone();
            }
            for error in &update.errors {
                if !run.errors.contains(error) {
                    run.errors.push(error.clone());
                }
            }
            let stage_status = stage_status_from_tasks(run, &update.stage);
            if let Some(stage_snapshot) = run
                .stages
                .iter_mut()
                .find(|item| item.stage == update.stage)
            {
                stage_snapshot.status = stage_status;
            }
            for artifact in &update.artifacts {
                push_trace(
                    run,
                    OrganizationReconTraceKind::ArtifactCreated,
                    Some(update.stage.clone()),
                    Some(update.task_id.to_string()),
                    Some(update.status.clone()),
                    "info",
                    format!("Artifact created: {}", artifact.kind),
                );
            }
            for error in &update.errors {
                push_trace(
                    run,
                    OrganizationReconTraceKind::StepLog,
                    Some(update.stage.clone()),
                    Some(update.task_id.to_string()),
                    Some(update.status.clone()),
                    "error",
                    format!("{}: {}", error.code, error.message),
                );
            }
            let level = if update.errors.is_empty() {
                "info"
            } else if update.status == ReconTaskStatus::Failed {
                "error"
            } else {
                "warning"
            };
            let message = if update.errors.is_empty() {
                format!(
                    "Step {} finished as {:?} with {} record(s)",
                    update.task_id, update.status, update.record_count
                )
            } else {
                format!(
                    "Step {} finished as {:?}: {}",
                    update.task_id,
                    update.status,
                    update
                        .errors
                        .iter()
                        .map(|error| error.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            if !update.errors.is_empty() {
                push_trace(
                    run,
                    OrganizationReconTraceKind::StepAnnotation,
                    Some(update.stage.clone()),
                    Some(update.task_id.to_string()),
                    Some(update.status.clone()),
                    level,
                    message.clone(),
                );
            }
            push_trace(
                run,
                OrganizationReconTraceKind::StepCompleted,
                Some(update.stage),
                Some(update.task_id.to_string()),
                Some(update.status),
                level,
                message,
            );
        })
        .await
    }

    pub(crate) async fn append_task_log(
        &self,
        run_id: &str,
        stage: OrganizationReconStageName,
        task_id: &str,
        level: impl Into<String>,
        message: impl Into<String>,
    ) -> Option<OrganizationReconRunSnapshot> {
        self.update(run_id, |run| {
            run.updated_at = golish_core::time::now_ms();
            let current_status = run
                .tasks
                .iter()
                .find(|task| task.task_id == task_id)
                .map(|task| task.status.clone());
            push_trace(
                run,
                OrganizationReconTraceKind::StepLog,
                Some(stage),
                Some(task_id.to_string()),
                current_status,
                level,
                message,
            );
        })
        .await
    }

    pub(crate) async fn upsert_task_progress(
        &self,
        update: TaskProgressUpdate<'_>,
    ) -> Option<OrganizationReconRunSnapshot> {
        self.update(update.run_id, |run| {
            run.updated_at = golish_core::time::now_ms();
            if let Some(task) = run
                .tasks
                .iter_mut()
                .find(|task| task.task_id == update.task_id)
            {
                task.source_id = update.source_id.to_string();
                task.status = update.status.clone();
                task.record_count = update.record_count;
                if !update.errors.is_empty() {
                    task.errors = update.errors.clone();
                }
            } else {
                run.tasks.push(OrganizationReconTaskSnapshot {
                    task_id: update.task_id.to_string(),
                    stage: update.stage.clone(),
                    source_id: update.source_id.to_string(),
                    status: update.status.clone(),
                    record_count: update.record_count,
                    artifacts: Vec::new(),
                    errors: update.errors.clone(),
                });
            }
            if let Some(stage_snapshot) = run
                .stages
                .iter_mut()
                .find(|item| item.stage == update.stage)
            {
                if !stage_snapshot
                    .task_ids
                    .iter()
                    .any(|id| id == update.task_id)
                {
                    stage_snapshot.task_ids.push(update.task_id.to_string());
                }
            }
            let stage_status = stage_status_from_tasks(run, &update.stage);
            if let Some(stage_snapshot) = run
                .stages
                .iter_mut()
                .find(|item| item.stage == update.stage)
            {
                stage_snapshot.status = stage_status;
            }
            for error in update.errors {
                if !run.errors.contains(&error) {
                    run.errors.push(error);
                }
            }
        })
        .await
    }

    pub(crate) async fn finish_run(
        &self,
        run_id: &str,
        update: RunStateUpdate,
    ) -> Option<OrganizationReconRunSnapshot> {
        self.update(run_id, |run| {
            run.status = update.status.clone();
            run.updated_at = golish_core::time::now_ms();
            if let Some(error) = update.error {
                run.errors.push(error.clone());
                push_trace(
                    run,
                    OrganizationReconTraceKind::StepAnnotation,
                    None,
                    None,
                    None,
                    "error",
                    format!("Run failed: {}", error.code),
                );
            }
            push_trace(
                run,
                OrganizationReconTraceKind::RunCompleted,
                None,
                None,
                None,
                if run.errors.is_empty() {
                    "info"
                } else {
                    "warning"
                },
                format!("Organization Recon run finished as {:?}", run.status),
            );
        })
        .await
    }
}

fn push_trace(
    run: &mut OrganizationReconRunSnapshot,
    kind: OrganizationReconTraceKind,
    stage: Option<OrganizationReconStageName>,
    task_id: Option<String>,
    status: Option<ReconTaskStatus>,
    level: impl Into<String>,
    message: impl Into<String>,
) {
    let timestamp = golish_core::time::now_ms();
    run.trace_events.push(OrganizationReconTraceEvent {
        id: format!("{}-{}", timestamp, run.trace_events.len() + 1),
        kind,
        timestamp,
        stage,
        task_id,
        status,
        level: level.into(),
        message: message.into(),
    });
    if run.trace_events.len() > MAX_TRACE_EVENTS {
        let drop_count = run.trace_events.len() - MAX_TRACE_EVENTS;
        run.trace_events.drain(0..drop_count);
    }
}

fn stage_status_from_tasks(
    run: &OrganizationReconRunSnapshot,
    stage: &OrganizationReconStageName,
) -> ReconTaskStatus {
    let statuses = run
        .tasks
        .iter()
        .filter(|task| task.stage == *stage)
        .map(|task| &task.status)
        .collect::<Vec<_>>();
    if statuses
        .iter()
        .any(|status| **status == ReconTaskStatus::Running)
    {
        return ReconTaskStatus::Running;
    }
    if let Some(summary_status) = stage_summary_task_status(run, stage) {
        return summary_status;
    }
    if statuses
        .iter()
        .any(|status| **status == ReconTaskStatus::Failed)
    {
        return ReconTaskStatus::Failed;
    }
    if statuses
        .iter()
        .all(|status| matches!(status, ReconTaskStatus::Queued))
    {
        return ReconTaskStatus::Queued;
    }
    if statuses
        .iter()
        .all(|status| matches!(status, ReconTaskStatus::Skipped))
    {
        return ReconTaskStatus::Skipped;
    }
    if statuses
        .iter()
        .all(|status| matches!(status, ReconTaskStatus::CheckedEmpty))
    {
        return ReconTaskStatus::CheckedEmpty;
    }
    if statuses.iter().all(|status| {
        matches!(
            status,
            ReconTaskStatus::Completed | ReconTaskStatus::CheckedEmpty | ReconTaskStatus::Skipped
        )
    }) {
        return ReconTaskStatus::Completed;
    }
    ReconTaskStatus::Queued
}

fn stage_summary_task_status(
    run: &OrganizationReconRunSnapshot,
    stage: &OrganizationReconStageName,
) -> Option<ReconTaskStatus> {
    let task_id = match stage {
        OrganizationReconStageName::EnterpriseIntel
        | OrganizationReconStageName::PassiveInternet => "passive-internet",
        OrganizationReconStageName::ActiveCollection => "active-collection",
        OrganizationReconStageName::Processing => "processing",
        OrganizationReconStageName::Persistence => "persistence",
    };
    run.tasks
        .iter()
        .find(|task| task.stage == *stage && task.task_id == task_id)
        .and_then(|task| match task.status {
            ReconTaskStatus::Queued | ReconTaskStatus::Running => None,
            _ => Some(task.status.clone()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_recon::runner::initial_snapshot;

    #[tokio::test]
    async fn state_machine_emits_run_and_step_trace_events() {
        let state = OrganizationReconState::default();
        state
            .insert(initial_snapshot(
                "run-state-machine".into(),
                "org-state-machine".into(),
                "/tmp/project".into(),
            ))
            .await;

        let started = state.start_run("run-state-machine").await.unwrap();
        assert_eq!(started.status, OrganizationReconRunStatus::Running);
        assert_eq!(started.trace_events.len(), 1);
        assert_eq!(
            started.trace_events[0].kind,
            OrganizationReconTraceKind::RunStarted
        );

        let task_started = state
            .start_task(
                "run-state-machine",
                OrganizationReconStageName::PassiveInternet,
                "passive-internet",
            )
            .await
            .unwrap();
        assert_eq!(
            task_started
                .tasks
                .iter()
                .find(|task| task.task_id == "passive-internet")
                .unwrap()
                .status,
            ReconTaskStatus::Running
        );

        let task_finished = state
            .finish_task(TaskStateUpdate {
                run_id: "run-state-machine",
                stage: OrganizationReconStageName::PassiveInternet,
                task_id: "passive-internet",
                status: ReconTaskStatus::Completed,
                record_count: 2,
                artifacts: vec![ReconArtifactRef {
                    path: "/tmp/passive/manifest.json".into(),
                    kind: "provider_manifest".into(),
                    bytes: 42,
                }],
                errors: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(
            task_finished
                .stages
                .iter()
                .find(|stage| stage.stage == OrganizationReconStageName::PassiveInternet)
                .unwrap()
                .status,
            ReconTaskStatus::Completed
        );
        assert!(task_finished
            .trace_events
            .iter()
            .any(|event| event.kind == OrganizationReconTraceKind::ArtifactCreated));

        let logged = state
            .append_task_log(
                "run-state-machine",
                OrganizationReconStageName::PassiveInternet,
                "passive-internet",
                "info",
                "provider finished one batch",
            )
            .await
            .unwrap();
        assert!(logged.trace_events.iter().any(|event| {
            event.kind == OrganizationReconTraceKind::StepLog
                && event.message == "provider finished one batch"
        }));

        let completed = state
            .finish_run(
                "run-state-machine",
                RunStateUpdate {
                    status: OrganizationReconRunStatus::Completed,
                    error: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(completed.status, OrganizationReconRunStatus::Completed);
        assert_eq!(
            completed.trace_events.last().map(|event| &event.kind),
            Some(&OrganizationReconTraceKind::RunCompleted)
        );
    }

    #[tokio::test]
    async fn active_stage_summary_task_can_complete_after_child_failure() {
        let state = OrganizationReconState::default();
        state
            .insert(initial_snapshot(
                "run-active-summary".into(),
                "org-active-summary".into(),
                "/tmp/project".into(),
            ))
            .await;

        state
            .start_task(
                "run-active-summary",
                OrganizationReconStageName::ActiveCollection,
                "active-collection",
            )
            .await
            .unwrap();
        let with_child_failure = state
            .upsert_task_progress(TaskProgressUpdate {
                run_id: "run-active-summary",
                stage: OrganizationReconStageName::ActiveCollection,
                task_id: "nmap-example.com",
                source_id: "nmap",
                status: ReconTaskStatus::Failed,
                record_count: 0,
                errors: vec![ReconTaskError::new("active_tool_timeout", "nmap timed out")],
            })
            .await
            .unwrap();
        assert_eq!(
            with_child_failure
                .stages
                .iter()
                .find(|stage| stage.stage == OrganizationReconStageName::ActiveCollection)
                .unwrap()
                .status,
            ReconTaskStatus::Running
        );

        let completed = state
            .finish_task(TaskStateUpdate {
                run_id: "run-active-summary",
                stage: OrganizationReconStageName::ActiveCollection,
                task_id: "active-collection",
                status: ReconTaskStatus::CheckedEmpty,
                record_count: 0,
                artifacts: Vec::new(),
                errors: vec![ReconTaskError::new("active_tool_timeout", "nmap timed out")],
            })
            .await
            .unwrap();

        assert_eq!(
            completed
                .stages
                .iter()
                .find(|stage| stage.stage == OrganizationReconStageName::ActiveCollection)
                .unwrap()
                .status,
            ReconTaskStatus::CheckedEmpty
        );
    }
}
