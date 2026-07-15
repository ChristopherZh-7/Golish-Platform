//! Host-owned lease heartbeat for one claimed V2 stage worker.

use std::sync::Arc;
use std::time::Duration;

use golish_agent_kit::db_traits::{RuntimeMemoryRepository, RuntimeWorkerFence};
use golish_sub_agents::BoundWorkerChainContext;

pub(crate) const WORKER_HEARTBEAT_INTERVAL_SECS: u64 = 10;
pub(crate) const WORKER_LEASE_TTL_SECS: i32 = 30;

/// Keeps a claimed worker lease alive for the lifetime of one live dispatch.
///
/// Any heartbeat failure is fail-closed: the shared bound context is marked
/// lost, which prevents subsequent provider/tool/checkpoint work. Dropping the
/// supervisor aborts the host-owned heartbeat task immediately.
pub(crate) struct WorkerLeaseSupervisor {
    task: tokio::task::JoinHandle<()>,
}

impl WorkerLeaseSupervisor {
    pub(crate) fn start(
        repository: Arc<dyn RuntimeMemoryRepository>,
        bound: BoundWorkerChainContext,
    ) -> Self {
        Self::start_with_timing(
            repository,
            bound,
            Duration::from_secs(WORKER_HEARTBEAT_INTERVAL_SECS),
            WORKER_LEASE_TTL_SECS,
        )
    }

    fn start_with_timing(
        repository: Arc<dyn RuntimeMemoryRepository>,
        bound: BoundWorkerChainContext,
        interval: Duration,
        lease_ttl_secs: i32,
    ) -> Self {
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if bound.lease_is_lost() {
                    break;
                }

                // Checkpoints, tool fences and heartbeats all use the same CAS
                // version. Serialize them so a valid checkpoint cannot make an
                // in-flight heartbeat look stale.
                let _mutation_guard = bound.mutation_lock.lock().await;
                if bound.lease_is_lost() {
                    break;
                }
                let expected_checkpoint_version = bound.current_checkpoint_version();
                let fence = RuntimeWorkerFence {
                    operation_id: bound.operation_id,
                    stage_execution_id: bound.stage_execution_id,
                    stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                    worker_run_id: bound.worker_lease.worker_run_id,
                    lease_token: bound.worker_lease.lease_token,
                    attempt_epoch: bound.worker_lease.attempt_epoch,
                    expected_checkpoint_version,
                };
                match repository.heartbeat_worker(fence, lease_ttl_secs).await {
                    Ok(worker) if worker.checkpoint_version == expected_checkpoint_version => {}
                    Ok(worker) => {
                        tracing::warn!(
                            worker_run_id = %bound.worker_lease.worker_run_id,
                            expected_checkpoint_version,
                            actual_checkpoint_version = worker.checkpoint_version,
                            "worker heartbeat returned an unexpected checkpoint witness"
                        );
                        bound.mark_lease_lost();
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(
                            worker_run_id = %bound.worker_lease.worker_run_id,
                            attempt_epoch = bound.worker_lease.attempt_epoch,
                            error = %error,
                            "worker heartbeat failed; blocking subsequent worker work"
                        );
                        bound.mark_lease_lost();
                        break;
                    }
                }
            }
        });
        Self { task }
    }
}

impl Drop for WorkerLeaseSupervisor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use golish_agent_kit::db_traits::{
        CreateRuntimeOperation, CreatedRuntimeOperation, ProjectScopeRegistration,
        RuntimeMemoryError, RuntimeMemoryRepository, RuntimeWorkerFence, RuntimeWorkerView,
    };
    use golish_sub_agents::BoundWorkerChainContext;
    use uuid::Uuid;

    use super::{WorkerLeaseSupervisor, WORKER_HEARTBEAT_INTERVAL_SECS, WORKER_LEASE_TTL_SECS};

    struct FailingHeartbeatRepository {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl RuntimeMemoryRepository for FailingHeartbeatRepository {
        async fn project_scope_register_first_open(
            &self,
            _canonical_path: &str,
            _path_sha256: &str,
        ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn project_scope_rename(
            &self,
            _project_scope_id: Uuid,
            _expected_old_path: &str,
            _expected_row_version: i64,
            _new_path: &str,
            _new_path_sha256: &str,
        ) -> Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn create_runtime_operation(
            &self,
            _input: CreateRuntimeOperation,
        ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn heartbeat_worker(
            &self,
            fence: RuntimeWorkerFence,
            _extend_seconds: i32,
        ) -> Result<RuntimeWorkerView, RuntimeMemoryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(RuntimeMemoryError::LeaseLost {
                worker_run_id: fence.worker_run_id,
                attempt_epoch: fence.attempt_epoch,
            })
        }
    }

    fn bound_worker() -> BoundWorkerChainContext {
        BoundWorkerChainContext {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            worker_lease: golish_core::WorkerLeaseContext {
                worker_run_id: Uuid::new_v4(),
                stage_run_unit_id: Uuid::new_v4(),
                lease_token: Uuid::new_v4(),
                attempt_epoch: 4,
            },
            candidate_attempt: None,
            candidate_submit_only: false,
            return_on_first_durable_stage_submission: false,
            stage_team_leader: None,
            chain_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            agent_type: "recon".to_string(),
            runtime_memory_source: None,
            initial_chain: serde_json::json!([]),
            initial_prompt_already_checkpointed: false,
            checkpoint_version: Arc::new(AtomicI64::new(2)),
            checkpoint_body: Arc::new(std::sync::RwLock::new(serde_json::json!([]))),
            lease_lost: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            tool_lifecycle: None,
        }
    }

    #[test]
    fn worker_lease_supervisor_uses_ten_second_heartbeat_and_thirty_second_ttl() {
        assert_eq!(WORKER_HEARTBEAT_INTERVAL_SECS, 10);
        assert_eq!(WORKER_LEASE_TTL_SECS, 30);
        let _type_witness = std::any::TypeId::of::<WorkerLeaseSupervisor>();
    }

    #[tokio::test]
    async fn heartbeat_lease_loss_blocks_subsequent_worker_work() {
        let repository = Arc::new(FailingHeartbeatRepository {
            calls: AtomicUsize::new(0),
        });
        let repository_trait: Arc<dyn RuntimeMemoryRepository> = repository.clone();
        let bound = bound_worker();
        let _supervisor = WorkerLeaseSupervisor::start_with_timing(
            repository_trait,
            bound.clone(),
            Duration::from_millis(1),
            30,
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while !bound.lease_is_lost() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("heartbeat failure marks lease lost promptly");

        assert_eq!(repository.calls.load(Ordering::SeqCst), 1);
    }
}
