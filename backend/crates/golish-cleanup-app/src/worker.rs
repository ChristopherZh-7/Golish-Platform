//! Process-global Cleanup closeout owner.
//!
//! DB leases, not an in-memory singleton, are the concurrency authority. GUI
//! and CLI may both start this runtime safely: only one process can claim a
//! deletion job, and an expired claim is re-run from the immutable artifact
//! snapshot. External/file work runs after `claim_next_artifact_cleanup`
//! commits and before the final hard-delete transaction begins.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use golish_db::repo::organization_deletion_jobs::{
    self, ArtifactCleanupFailure, ArtifactCleanupPlan, ClaimOrganizationArtifactCleanup,
    CompleteOrganizationArtifactCleanup, DEFAULT_JOB_LEASE_SECONDS,
};
use golish_db::PgPool;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupWorkerRunState {
    Idle,
    Processed,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupWorkerError {
    #[error("cleanup worker repository failed: {0}")]
    Repository(String),
    #[error("cleanup worker is already shutting down")]
    ShuttingDown,
}

impl CleanupWorkerError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Repository(_) => "cleanup_worker_repository_failed",
            Self::ShuttingDown => "cleanup_worker_shutting_down",
        }
    }
}

#[async_trait]
pub trait OrganizationArtifactCleaner: Send + Sync {
    async fn cleanup(&self, plan: &ArtifactCleanupPlan) -> Result<(), ArtifactCleanupFailure>;
}

#[derive(Clone)]
pub struct CleanupCloseoutRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    pool: Arc<PgPool>,
    cleaner: Arc<dyn OrganizationArtifactCleaner>,
    worker_id: String,
    lifecycle: tokio::sync::Mutex<()>,
    state: Mutex<RuntimeState>,
}

#[derive(Default)]
struct RuntimeState {
    running: bool,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl CleanupCloseoutRuntime {
    pub fn new(
        pool: Arc<PgPool>,
        cleaner: Arc<dyn OrganizationArtifactCleaner>,
        worker_id: impl Into<String>,
    ) -> Result<Self, CleanupWorkerError> {
        let worker_id = worker_id.into().trim().to_string();
        if worker_id.is_empty() || worker_id.len() > 256 || worker_id.chars().any(char::is_control)
        {
            return Err(CleanupWorkerError::Repository(
                "cleanup_worker_identity_invalid".to_string(),
            ));
        }
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                pool,
                cleaner,
                worker_id,
                lifecycle: tokio::sync::Mutex::new(()),
                state: Mutex::new(RuntimeState::default()),
            }),
        })
    }

    pub async fn start(&self) -> Result<bool, CleanupWorkerError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if self.state().running {
            return Ok(false);
        }
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let runtime = self.clone();
        let join = tokio::spawn(async move { runtime.run_loop(receiver).await });
        let mut state = self.state();
        state.running = true;
        state.cancel = Some(cancel);
        state.join = Some(join);
        Ok(true)
    }

    pub async fn shutdown(&self) -> Result<(), CleanupWorkerError> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        let (cancel, join) = {
            let mut state = self.state();
            if !state.running {
                return Ok(());
            }
            (state.cancel.take(), state.join.take())
        };
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        }
        if let Some(join) = join {
            if let Err(error) = join.await {
                tracing::warn!(error = %error, "cleanup closeout worker join failed");
            }
        }
        self.state().running = false;
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.state().running
    }

    pub async fn run_once(&self) -> Result<CleanupWorkerRunState, CleanupWorkerError> {
        // A process may stop after artifact cleanup commits but before the
        // independent hard-delete transaction starts. Resume that DB-only
        // continuation before claiming more external cleanup work.
        if let Some(job_id) = organization_deletion_jobs::next_hard_delete_ready(&self.inner.pool)
            .await
            .map_err(repository_error)?
        {
            if let Err(error) =
                organization_deletion_jobs::hard_delete(&self.inner.pool, job_id).await
            {
                let message = error.to_string();
                let _ = organization_deletion_jobs::record_hard_delete_error(
                    &self.inner.pool,
                    job_id,
                    "organization_hard_delete_failed",
                    &message,
                )
                .await;
                return Err(repository_error(error));
            }
            return Ok(CleanupWorkerRunState::Processed);
        }
        let reaped = organization_deletion_jobs::reap_expired_cleanup_attempts(&self.inner.pool)
            .await
            .map_err(repository_error)?;
        let claim = organization_deletion_jobs::claim_next_artifact_cleanup(
            &self.inner.pool,
            &ClaimOrganizationArtifactCleanup {
                worker_id: self.inner.worker_id.clone(),
                lease_seconds: DEFAULT_JOB_LEASE_SECONDS,
            },
        )
        .await
        .map_err(repository_error)?;
        let Some((job, plan)) = claim else {
            return Ok(if reaped == 0 {
                CleanupWorkerRunState::Idle
            } else {
                CleanupWorkerRunState::Processed
            });
        };
        let lease_token = job.lease_token.ok_or_else(|| {
            CleanupWorkerError::Repository("organization_cleanup_claim_unfenced".to_string())
        })?;
        let cleanup_result = self.inner.cleaner.cleanup(&plan).await;
        let completed = organization_deletion_jobs::complete_artifact_cleanup(
            &self.inner.pool,
            &CompleteOrganizationArtifactCleanup {
                job_id: job.id,
                worker_id: self.inner.worker_id.clone(),
                lease_token,
                expected_row_version: job.row_version,
                result: cleanup_result,
            },
        )
        .await
        .map_err(repository_error)?;
        if completed.state == "artifact_cleanup_succeeded" {
            if let Err(error) =
                organization_deletion_jobs::hard_delete(&self.inner.pool, completed.id).await
            {
                let message = error.to_string();
                let _ = organization_deletion_jobs::record_hard_delete_error(
                    &self.inner.pool,
                    completed.id,
                    "organization_hard_delete_failed",
                    &message,
                )
                .await;
                return Err(repository_error(error));
            }
        }
        Ok(CleanupWorkerRunState::Processed)
    }

    async fn run_loop(&self, mut cancel: tokio::sync::watch::Receiver<bool>) {
        loop {
            if *cancel.borrow() {
                break;
            }
            let delay = match self.run_once().await {
                Ok(CleanupWorkerRunState::Processed) => Duration::ZERO,
                Ok(CleanupWorkerRunState::Idle) => Duration::from_millis(250),
                Err(error) => {
                    tracing::warn!(
                        error_code = error.code(),
                        error = %error,
                        "cleanup closeout worker tick failed; durable lease/state retained"
                    );
                    Duration::from_secs(1)
                }
            };
            if delay.is_zero() {
                tokio::task::yield_now().await;
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        break;
                    }
                }
            }
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RuntimeState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn repository_error(error: golish_db::DbError) -> CleanupWorkerError {
    CleanupWorkerError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn worker_identity_is_not_optional_or_model_selected_per_call() {
        let source = include_str!("worker.rs");
        let runtime_inner = source
            .split("struct RuntimeInner")
            .nth(1)
            .expect("runtime inner declaration")
            .split('}')
            .next()
            .expect("runtime inner fields");
        assert!(runtime_inner.contains("worker_id: String"));
        assert!(!runtime_inner.contains("pub worker_id"));
        assert!(source.contains("claim_next_artifact_cleanup"));
    }
}
