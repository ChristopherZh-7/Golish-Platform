//! Candidate V2 verifier claim/binding and lease supervision.
//!
//! Task 7 owns claim -> opaque context -> one foreground verifier and action
//! journal safety. Task 8 adds exact result submission and the compound
//! terminalizer invoked by the stage scheduler after the verifier returns.

use std::sync::atomic::{AtomicBool, AtomicI64};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;

use golish_agent_kit::db_traits::{
    ClaimCandidateAttempt, HeartbeatCandidateAttempt, RuntimeMemoryRepository, RuntimeWorkerFence,
};
use golish_sub_agents::{BoundWorkerChainContext, BoundWorkerToolLifecycle};

use crate::agentic_loop::worker_tool_lifecycle::RuntimeWorkerToolLifecycle;

const CANDIDATE_HEARTBEAT_INTERVAL_SECS: u64 = 10;
const CANDIDATE_LEASE_TTL_SECS: i32 = 30;

pub struct ClaimedCandidateVerifier {
    pub bound: BoundWorkerChainContext,
    _supervisor: CandidateLeaseSupervisor,
}

pub async fn claim_candidate_verifier(
    repository: Arc<dyn RuntimeMemoryRepository>,
    tracker: golish_agent_kit::db_tracking::DbTracker,
    operation_id: uuid::Uuid,
    verification_stage_execution_id: uuid::Uuid,
    verification_stage_run_unit_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    parent_request_id: &str,
) -> anyhow::Result<Option<ClaimedCandidateVerifier>> {
    let lease_owner = format!("candidate_verifier:{parent_request_id}");
    let Some(claimed) = repository
        .claim_candidate_attempt(ClaimCandidateAttempt {
            operation_id,
            organization_id,
            verification_stage_execution_id,
            verification_stage_run_unit_id,
            lease_owner: lease_owner.clone(),
            lease_seconds: CANDIDATE_LEASE_TTL_SECS,
        })
        .await?
    else {
        return Ok(None);
    };
    anyhow::ensure!(
        claimed.worker.operation_id == operation_id
            && claimed.worker.stage_execution_id == verification_stage_execution_id
            && claimed.worker.stage_run_unit_id == verification_stage_run_unit_id
            && claimed.worker.organization_id == organization_id
            && claimed.worker.specialist == "candidate_verifier"
            && claimed.worker.work_item_kind == "candidate_attempt"
            && claimed.worker.work_item_key == claimed.candidate_attempt.attempt_id.to_string(),
        "compound Candidate claim returned a mismatched WorkerRun"
    );
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| anyhow::anyhow!("claimed Candidate WorkerRun has no lease token"))?;
    let mut bound = BoundWorkerChainContext {
        operation_id,
        stage_execution_id: verification_stage_execution_id,
        organization_id,
        worker_lease: golish_core::WorkerLeaseContext {
            worker_run_id: claimed.worker.id,
            stage_run_unit_id: verification_stage_run_unit_id,
            lease_token,
            attempt_epoch: claimed.worker.attempt_epoch,
        },
        candidate_attempt: Some(claimed.candidate_attempt),
        chain_id: claimed.message_chain_id,
        session_id: tracker.session_uuid(),
        agent_type: "candidate_verifier".to_string(),
        initial_chain: claimed.worker.checkpoint.clone(),
        initial_prompt_already_checkpointed: claimed.worker.checkpoint_version > 0,
        checkpoint_version: Arc::new(AtomicI64::new(claimed.worker.checkpoint_version)),
        checkpoint_body: Arc::new(StdRwLock::new(claimed.worker.checkpoint)),
        lease_lost: Arc::new(AtomicBool::new(false)),
        mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
        tool_lifecycle: None,
    };
    let lifecycle: Arc<dyn BoundWorkerToolLifecycle> = Arc::new(RuntimeWorkerToolLifecycle::new(
        tracker,
        repository.clone(),
        bound.clone(),
    ));
    bound.tool_lifecycle = Some(lifecycle);
    let supervisor = CandidateLeaseSupervisor::start(
        repository,
        bound.clone(),
        lease_owner,
        CANDIDATE_HEARTBEAT_INTERVAL_SECS,
    );
    Ok(Some(ClaimedCandidateVerifier {
        bound,
        _supervisor: supervisor,
    }))
}

struct CandidateLeaseSupervisor {
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl CandidateLeaseSupervisor {
    fn start(
        repository: Arc<dyn RuntimeMemoryRepository>,
        bound: BoundWorkerChainContext,
        lease_owner: String,
        heartbeat_secs: u64,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_task = stop.clone();
        let bound_for_task = bound.clone();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(heartbeat_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                interval.tick().await;
                if stop_for_task.load(std::sync::atomic::Ordering::SeqCst)
                    || bound_for_task.lease_is_lost()
                {
                    break;
                }
                let Some(candidate_attempt) = bound_for_task.candidate_attempt.clone() else {
                    bound_for_task.mark_lease_lost();
                    break;
                };
                let _mutation_guard = bound_for_task.mutation_lock.lock().await;
                let heartbeat = repository
                    .heartbeat_candidate_attempt(HeartbeatCandidateAttempt {
                        candidate_attempt,
                        fence: RuntimeWorkerFence {
                            operation_id: bound_for_task.operation_id,
                            stage_execution_id: bound_for_task.stage_execution_id,
                            stage_run_unit_id: bound_for_task.worker_lease.stage_run_unit_id,
                            worker_run_id: bound_for_task.worker_lease.worker_run_id,
                            lease_token: bound_for_task.worker_lease.lease_token,
                            attempt_epoch: bound_for_task.worker_lease.attempt_epoch,
                            expected_checkpoint_version: bound_for_task
                                .current_checkpoint_version(),
                        },
                        organization_id: bound_for_task.organization_id,
                        lease_owner: lease_owner.clone(),
                        extend_seconds: CANDIDATE_LEASE_TTL_SECS,
                    })
                    .await;
                if heartbeat.is_err() {
                    bound_for_task.mark_lease_lost();
                    break;
                }
            }
        });
        Self { stop, task }
    }
}

impl Drop for CandidateLeaseSupervisor {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_lease_uses_ten_second_heartbeat_and_thirty_second_ttl() {
        assert_eq!(CANDIDATE_HEARTBEAT_INTERVAL_SECS, 10);
        assert_eq!(CANDIDATE_LEASE_TTL_SECS, 30);
    }
}
