use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use thiserror::Error;

use crate::agentic_loop::StageRunReentryGuard;

/// Stable per-session ownership slot for every top-level agent request.
///
/// GUI sessions keep this slot in `AiState` across `AgentBridge` replacement.
/// Each bridge is bound to one generation, so a late clone of a removed bridge
/// can never acquire after init/shutdown advances the slot. Standalone bridges
/// (CLI/tests) own a private active slot.
#[derive(Debug)]
pub struct SessionRequestSlot {
    in_flight: AtomicBool,
    current_generation: AtomicU64,
    accepting: AtomicBool,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("another request is already running for this agent session")]
pub struct SessionRequestBusy;

/// Exclusive lifecycle token used while installing a new bridge generation.
///
/// It shares the same `in_flight` bit as normal requests. Therefore init is
/// fail-fast while an old generation is running, and a request cannot start on
/// either generation until the atomic replacement is complete.
#[derive(Debug)]
pub struct SessionRequestTransitionLease {
    slot: Arc<SessionRequestSlot>,
    activated: bool,
}

/// Shared RAII ownership token for one bridge's top-level request boundary.
///
/// Clones belong to the same request. The gate is released only when the last
/// clone is dropped, which lets a Task/profile lead hand the token to
/// `BridgeAgentExecutor` without reopening or recursively acquiring the gate.
#[derive(Clone, Debug)]
pub struct TopLevelRequestLease {
    inner: Arc<TopLevelRequestLeaseInner>,
}

#[derive(Debug)]
struct TopLevelRequestLeaseInner {
    slot: Arc<SessionRequestSlot>,
    generation: u64,
    task_initialized: AtomicBool,
}

impl Default for SessionRequestSlot {
    fn default() -> Self {
        Self {
            in_flight: AtomicBool::new(false),
            current_generation: AtomicU64::new(0),
            accepting: AtomicBool::new(false),
        }
    }
}

impl SessionRequestSlot {
    /// Create the private active slot used by standalone bridges.
    pub(crate) fn new_active() -> (Arc<Self>, u64) {
        let slot = Arc::new(Self {
            in_flight: AtomicBool::new(false),
            current_generation: AtomicU64::new(1),
            accepting: AtomicBool::new(true),
        });
        (slot, 1)
    }

    /// Acquire a lifecycle transition without waiting.
    pub fn try_begin_transition(
        self: &Arc<Self>,
    ) -> Result<SessionRequestTransitionLease, SessionRequestBusy> {
        self.in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| SessionRequestBusy)?;

        Ok(SessionRequestTransitionLease {
            slot: self.clone(),
            activated: false,
        })
    }

    /// Acquire top-level ownership for one exact bridge generation.
    pub(crate) fn try_begin_request(
        self: &Arc<Self>,
        generation: u64,
    ) -> Result<TopLevelRequestLease, SessionRequestBusy> {
        self.in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| SessionRequestBusy)?;

        if !self.accepts_generation(generation) {
            self.in_flight.store(false, Ordering::Release);
            return Err(SessionRequestBusy);
        }

        Ok(TopLevelRequestLease {
            inner: Arc::new(TopLevelRequestLeaseInner {
                slot: self.clone(),
                generation,
                task_initialized: AtomicBool::new(false),
            }),
        })
    }

    pub(crate) fn accepts_generation(&self, generation: u64) -> bool {
        self.accepting.load(Ordering::Acquire)
            && self.current_generation.load(Ordering::Acquire) == generation
    }

    /// Invalidate the current generation immediately. An already-running owner
    /// may unwind under cancellation, but no late clone can start afterward.
    pub fn invalidate(&self) {
        self.accepting.store(false, Ordering::Release);
        self.current_generation.fetch_add(1, Ordering::AcqRel);
    }
}

impl TopLevelRequestLease {
    pub(crate) fn belongs_to(&self, slot: &Arc<SessionRequestSlot>, generation: u64) -> bool {
        Arc::ptr_eq(&self.inner.slot, slot) && self.inner.generation == generation
    }

    pub(crate) fn is_current_and_accepting(&self) -> bool {
        self.inner.slot.accepts_generation(self.inner.generation)
    }

    /// Upgrade this already-owned top-level request into a harness Task once.
    /// The lead→orchestrator handoff and any nested executor construction reuse
    /// the same token, so only the first upgrade refreshes the retry budget.
    pub(crate) fn initialize_task(
        &self,
        slot: &Arc<SessionRequestSlot>,
        generation: u64,
        reentry_guard: &StageRunReentryGuard,
    ) -> Result<(), SessionRequestBusy> {
        if !self.belongs_to(slot, generation) || !self.is_current_and_accepting() {
            return Err(SessionRequestBusy);
        }

        if self
            .inner
            .task_initialized
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            reentry_guard.reset();
        }
        Ok(())
    }
}

impl Drop for TopLevelRequestLeaseInner {
    fn drop(&mut self) {
        self.slot.in_flight.store(false, Ordering::Release);
    }
}

impl SessionRequestTransitionLease {
    /// Publish exactly one new current generation while transition ownership is
    /// held. The returned generation must be bound to the candidate bridge
    /// before it is inserted into `AiState`.
    pub fn activate_next_generation(&mut self) -> Result<u64, SessionRequestBusy> {
        if self.activated {
            return Err(SessionRequestBusy);
        }
        let generation = self
            .slot
            .current_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        self.slot.accepting.store(true, Ordering::Release);
        self.activated = true;
        Ok(generation)
    }
}

impl Drop for SessionRequestTransitionLease {
    fn drop(&mut self) {
        self.slot.in_flight.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    use async_trait::async_trait;
    use golish_agent_kit::harness::StageKind;
    use golish_agent_kit::task_orchestrator::{AgentExecutor, ExecutionContext};
    use golish_core::runtime::{ApprovalResult, GolishRuntime, RuntimeError, RuntimeEvent};
    use golish_core::WorkerLeaseContext;

    use crate::agentic_loop::StageRunReentryGuard;
    use crate::bridge_executor::BridgeAgentExecutor;

    use super::{SessionRequestBusy, SessionRequestSlot};

    #[derive(Debug)]
    struct MockRuntime;

    #[async_trait]
    impl GolishRuntime for MockRuntime {
        fn emit(&self, _event: RuntimeEvent) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn request_approval(
            &self,
            _request_id: String,
            _tool_name: String,
            _args: serde_json::Value,
            _risk_level: String,
        ) -> Result<ApprovalResult, RuntimeError> {
            Err(RuntimeError::ApprovalTimeout(0))
        }

        fn is_interactive(&self) -> bool {
            false
        }

        fn auto_approve(&self) -> bool {
            false
        }

        async fn shutdown(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    async fn real_bridge() -> (tempfile::TempDir, Arc<super::super::AgentBridge>) {
        let workspace = tempfile::tempdir().expect("temp workspace");
        let bridge = super::super::AgentBridge::new_openrouter_with_runtime(
            workspace.path().to_path_buf(),
            "test-model",
            "test-key",
            None,
            Arc::new(MockRuntime),
        )
        .await
        .expect("test bridge");
        (workspace, Arc::new(bridge))
    }

    async fn clear_history_like_ipc(bridge: &super::super::AgentBridge) -> anyhow::Result<()> {
        let request = bridge.begin_top_level_request().await?;
        bridge.clear_conversation_history().await;
        bridge.clear_top_level_request_state(&request).await
    }

    async fn retry_compaction_like_ipc(bridge: &super::super::AgentBridge) -> anyhow::Result<()> {
        let request = bridge.begin_top_level_request().await?;
        let result = bridge.retry_compaction().await.map_err(anyhow::Error::msg);
        let cleanup = bridge.clear_top_level_request_state(&request).await;
        result.and(cleanup)
    }

    async fn restore_history_like_ipc(bridge: &super::super::AgentBridge) -> anyhow::Result<()> {
        let request = bridge.begin_top_level_request().await?;
        bridge
            .restore_conversation_history(vec![(
                "user".to_string(),
                "replacement history".to_string(),
            )])
            .await;
        bridge.clear_top_level_request_state(&request).await
    }

    fn active_slot() -> (Arc<SessionRequestSlot>, u64) {
        SessionRequestSlot::new_active()
    }

    #[test]
    fn concurrent_request_cannot_reset_exhausted_budget() {
        let (gate, generation) = active_slot();
        let guard = StageRunReentryGuard::default();
        let owner = gate
            .try_begin_request(generation)
            .expect("request A acquires");
        owner
            .initialize_task(&gate, generation, &guard)
            .expect("request A initializes Task");
        guard.mark_exhausted(StageKind::Enumeration);

        let busy = gate
            .try_begin_request(generation)
            .expect_err("request B must fail fast");

        assert_eq!(busy, SessionRequestBusy);
        assert!(guard.is_exhausted(StageKind::Enumeration));
        drop(owner);
    }

    #[test]
    fn cloned_owner_transfers_to_task_without_recursive_acquire_or_early_release() {
        let (gate, generation) = active_slot();
        let guard = StageRunReentryGuard::default();
        let lead_owner = gate.try_begin_request(generation).expect("lead acquires");
        let executor_owner = lead_owner.clone();

        executor_owner
            .initialize_task(&gate, generation, &guard)
            .expect("lead handoff initializes Task");
        guard.mark_exhausted(StageKind::Enumeration);
        executor_owner
            .initialize_task(&gate, generation, &guard)
            .expect("nested reuse does not reacquire");
        assert!(guard.is_exhausted(StageKind::Enumeration));

        drop(lead_owner);
        assert_eq!(
            gate.try_begin_request(generation)
                .expect_err("executor clone still owns gate"),
            SessionRequestBusy
        );
        drop(executor_owner);
        assert!(gate.try_begin_request(generation).is_ok());
    }

    #[test]
    fn completed_request_releases_and_next_task_gets_fresh_budget() {
        let (gate, generation) = active_slot();
        let guard = StageRunReentryGuard::default();
        let owner = gate
            .try_begin_request(generation)
            .expect("request A acquires");
        owner.initialize_task(&gate, generation, &guard).unwrap();
        guard.mark_exhausted(StageKind::Enumeration);
        drop(owner);

        let next = gate
            .try_begin_request(generation)
            .expect("request C acquires");
        next.initialize_task(&gate, generation, &guard).unwrap();

        assert!(!guard.is_exhausted(StageKind::Enumeration));
    }

    #[test]
    fn error_cancellation_and_unwind_drop_the_last_owner() {
        let (gate, generation) = active_slot();

        let failed: Result<(), &'static str> = (|| {
            let _lease = gate.try_begin_request(generation).map_err(|_| "busy")?;
            Err("request failed")
        })();
        assert_eq!(failed, Err("request failed"));
        drop(
            gate.try_begin_request(generation)
                .expect("error releases ownership"),
        );

        {
            let _cancelled_future_owner = gate
                .try_begin_request(generation)
                .expect("request acquires");
        }
        drop(
            gate.try_begin_request(generation)
                .expect("future drop releases ownership"),
        );

        let unwind = std::panic::catch_unwind({
            let gate = gate.clone();
            move || {
                let _lease = gate
                    .try_begin_request(generation)
                    .expect("request acquires");
                panic!("simulated request unwind");
            }
        });
        assert!(unwind.is_err());
        assert!(
            gate.try_begin_request(generation).is_ok(),
            "unwind releases ownership"
        );
    }

    #[test]
    fn invalidated_generation_never_reopens_after_a_new_generation_activates() {
        let slot = Arc::new(SessionRequestSlot::default());
        let mut first_install = slot.try_begin_transition().unwrap();
        let first_generation = first_install.activate_next_generation().unwrap();
        drop(first_install);
        let first_owner = slot.try_begin_request(first_generation).unwrap();

        slot.invalidate();
        drop(first_owner);
        assert!(slot.try_begin_request(first_generation).is_err());

        let mut second_install = slot.try_begin_transition().unwrap();
        let second_generation = second_install.activate_next_generation().unwrap();
        drop(second_install);
        assert!(second_generation > first_generation);
        assert!(slot.try_begin_request(first_generation).is_err());
        assert!(slot.try_begin_request(second_generation).is_ok());
    }

    #[tokio::test]
    async fn real_bridge_busy_request_cannot_reset_cancel_or_mutate_sidechannels() {
        let (_workspace, bridge) = real_bridge().await;
        bridge
            .restore_conversation_history(vec![("user".to_string(), "keep me".to_string())])
            .await;
        let owner = bridge.begin_top_level_request().await.expect("A acquires");
        *bridge.harness_active_stage.write().await = Some(StageKind::Enumeration);
        *bridge.harness_submit_only.write().await = true;
        *bridge.harness_forced_tool.write().await = Some("stage_run".to_string());
        *bridge.harness_last_deliverable.write().await = Some("request-a".to_string());
        *bridge.pending_plan_request.write().await = Some("request-a-plan".to_string());
        bridge.cancel();

        let busy = bridge
            .begin_top_level_request()
            .await
            .expect_err("B must fail before touching A state");
        let clear_busy = clear_history_like_ipc(&bridge)
            .await
            .expect_err("history clear must share the same busy boundary");
        let event_sequence_before = bridge.events.event_sequence.load(Ordering::Acquire);
        let compaction_busy = retry_compaction_like_ipc(&bridge)
            .await
            .expect_err("retry compaction must fail before emitting or mutating");
        let restore_busy = restore_history_like_ipc(&bridge)
            .await
            .expect_err("full restore history mutation must share the busy boundary");

        assert_eq!(
            busy.to_string(),
            "another request is already running for this agent session"
        );
        assert_eq!(clear_busy.to_string(), busy.to_string());
        assert_eq!(compaction_busy.to_string(), busy.to_string());
        assert_eq!(restore_busy.to_string(), busy.to_string());
        assert_eq!(bridge.conversation_history_len().await, 1);
        assert_eq!(
            bridge.events.event_sequence.load(Ordering::Acquire),
            event_sequence_before,
            "busy compaction must not emit CompactionStarted"
        );
        assert!(
            bridge.is_cancelled(),
            "busy B must not reset A cancellation"
        );
        assert_eq!(
            *bridge.harness_active_stage.read().await,
            Some(StageKind::Enumeration)
        );
        assert!(*bridge.harness_submit_only.read().await);
        assert_eq!(
            bridge.harness_forced_tool.read().await.as_deref(),
            Some("stage_run")
        );
        assert_eq!(
            bridge.harness_last_deliverable.read().await.as_deref(),
            Some("request-a")
        );
        assert_eq!(
            bridge.pending_plan_request.read().await.as_deref(),
            Some("request-a-plan")
        );
        drop(owner);
    }

    #[tokio::test]
    async fn real_bridge_next_acquire_scrubs_dropped_owner_and_resets_cancel() {
        let (_workspace, bridge) = real_bridge().await;
        let abandoned = bridge.begin_top_level_request().await.expect("A acquires");
        *bridge.harness_active_stage.write().await = Some(StageKind::Enumeration);
        *bridge.harness_active_stage_execution_id.write().await =
            Some(uuid::Uuid::from_u128(0x901));
        *bridge.harness_active_stage_run_unit_id.write().await = Some(uuid::Uuid::from_u128(0x902));
        *bridge.harness_active_worker_lease.write().await = Some(WorkerLeaseContext {
            worker_run_id: uuid::Uuid::from_u128(0x903),
            stage_run_unit_id: uuid::Uuid::from_u128(0x902),
            lease_token: uuid::Uuid::from_u128(0x904),
            attempt_epoch: 5,
        });
        *bridge.harness_submit_only.write().await = true;
        *bridge.harness_forced_tool.write().await = Some("stage_run".to_string());
        *bridge.harness_last_deliverable.write().await = Some("stale".to_string());
        *bridge.pending_plan_request.write().await = Some("stale-plan".to_string());
        bridge.cancel();
        drop(abandoned); // models async future drop/unwind before normal cleanup

        let next = bridge
            .begin_top_level_request()
            .await
            .expect("C acquires and scrubs");

        assert!(!bridge.is_cancelled());
        assert_eq!(*bridge.harness_active_stage.read().await, None);
        assert_eq!(*bridge.harness_active_stage_execution_id.read().await, None);
        assert_eq!(*bridge.harness_active_stage_run_unit_id.read().await, None);
        assert_eq!(*bridge.harness_active_worker_lease.read().await, None);
        assert!(!*bridge.harness_submit_only.read().await);
        assert!(bridge.harness_forced_tool.read().await.is_none());
        assert!(bridge.harness_last_deliverable.read().await.is_none());
        assert!(bridge.pending_plan_request.read().await.is_none());
        bridge
            .clear_top_level_request_state(&next)
            .await
            .expect("normal cleanup while owner is held");
    }

    #[tokio::test]
    async fn trusted_runtime_identity_is_cleared_between_subtasks_and_top_level_requests() {
        let (_workspace, bridge) = real_bridge().await;
        let owner = bridge
            .begin_top_level_request()
            .await
            .expect("request owns bridge");
        let first_unit = uuid::Uuid::from_u128(0x801);
        let first = ExecutionContext {
            operation_id: Some(uuid::Uuid::from_u128(0x802)),
            stage_execution_id: Some(uuid::Uuid::from_u128(0x803)),
            stage_run_unit_id: Some(first_unit),
            worker_lease: Some(WorkerLeaseContext {
                worker_run_id: uuid::Uuid::from_u128(0x804),
                stage_run_unit_id: first_unit,
                lease_token: uuid::Uuid::from_u128(0x805),
                attempt_epoch: 3,
            }),
            harness_stage: Some(StageKind::Enumeration),
            harness_org_id: Some(uuid::Uuid::from_u128(0x806)),
            ..Default::default()
        };

        bridge
            .publish_active_execution_context(&first)
            .await
            .expect("publish first subtask identity");
        assert_eq!(
            *bridge.harness_active_stage_execution_id.read().await,
            first.stage_execution_id
        );
        assert_eq!(
            *bridge.harness_active_stage_run_unit_id.read().await,
            first.stage_run_unit_id
        );
        assert_eq!(
            *bridge.harness_active_worker_lease.read().await,
            first.worker_lease
        );
        let loop_event_tx = bridge.get_or_create_event_tx();
        let loop_context = bridge.build_loop_context(&loop_event_tx).await;
        assert_eq!(loop_context.stage_execution_id, first.stage_execution_id);
        assert_eq!(loop_context.stage_run_unit_id, first.stage_run_unit_id);
        assert_eq!(loop_context.worker_lease, first.worker_lease);
        drop(loop_context);

        bridge.clear_active_subtask_context().await;
        assert_eq!(*bridge.harness_active_stage.read().await, None);
        assert_eq!(*bridge.harness_active_operation_id.read().await, None);
        assert_eq!(*bridge.harness_active_org_id.read().await, None);
        assert_eq!(*bridge.harness_active_stage_execution_id.read().await, None);
        assert_eq!(*bridge.harness_active_stage_run_unit_id.read().await, None);
        assert_eq!(*bridge.harness_active_worker_lease.read().await, None);

        let mismatched = ExecutionContext {
            stage_run_unit_id: Some(uuid::Uuid::from_u128(0x807)),
            worker_lease: Some(WorkerLeaseContext {
                worker_run_id: uuid::Uuid::from_u128(0x808),
                stage_run_unit_id: uuid::Uuid::from_u128(0x809),
                lease_token: uuid::Uuid::from_u128(0x80a),
                attempt_epoch: 4,
            }),
            ..Default::default()
        };
        assert!(bridge
            .publish_active_execution_context(&mismatched)
            .await
            .is_err());
        assert_eq!(*bridge.harness_active_stage_run_unit_id.read().await, None);
        assert_eq!(*bridge.harness_active_worker_lease.read().await, None);

        let second = ExecutionContext {
            operation_id: Some(uuid::Uuid::from_u128(0x811)),
            stage_execution_id: Some(uuid::Uuid::from_u128(0x812)),
            harness_stage: Some(StageKind::Verification),
            harness_org_id: Some(uuid::Uuid::from_u128(0x813)),
            ..Default::default()
        };
        bridge
            .publish_active_execution_context(&second)
            .await
            .expect("publish second subtask identity");
        assert_eq!(
            *bridge.harness_active_stage_execution_id.read().await,
            second.stage_execution_id
        );
        assert_ne!(second.stage_execution_id, first.stage_execution_id);

        bridge
            .clear_top_level_request_state(&owner)
            .await
            .expect("top-level cleanup");
        assert_eq!(*bridge.harness_active_stage.read().await, None);
        assert_eq!(*bridge.harness_active_operation_id.read().await, None);
        assert_eq!(*bridge.harness_active_org_id.read().await, None);
        assert_eq!(*bridge.harness_active_stage_execution_id.read().await, None);
        assert_eq!(*bridge.harness_active_stage_run_unit_id.read().await, None);
        assert_eq!(*bridge.harness_active_worker_lease.read().await, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_arriving_during_async_acquire_scrub_is_not_reset() {
        let (_workspace, bridge) = real_bridge().await;
        let stage_lock = bridge.harness_active_stage.write().await;
        let acquiring_bridge = bridge.clone();
        let acquisition = tokio::spawn(async move {
            acquiring_bridge
                .begin_top_level_request()
                .await
                .expect("request acquires after scrub unblocks")
        });

        // The first request owns the atomic gate and is blocked on `stage_lock`.
        // Giving it another scheduling turn makes the reset-before-scrub ordering
        // deterministic before Stop arrives.
        while !bridge
            .session_request_slot
            .in_flight
            .load(Ordering::Acquire)
        {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        bridge.cancel();
        drop(stage_lock);

        let owner = acquisition.await.expect("acquisition task joins");
        assert!(
            bridge.is_cancelled(),
            "Stop during async scrub must remain visible to the new request"
        );
        bridge
            .clear_top_level_request_state(&owner)
            .await
            .expect("cleanup");
    }

    #[tokio::test]
    async fn real_bridge_lead_to_executor_reuses_owner_and_resets_task_once() {
        let (_workspace, bridge) = real_bridge().await;
        bridge
            .stage_run_reentry_guard
            .mark_exhausted(StageKind::Enumeration);
        let lead_owner = bridge
            .begin_top_level_request()
            .await
            .expect("lead acquires");
        let executor = BridgeAgentExecutor::from_request(bridge.clone(), lead_owner.clone())
            .expect("lead transfers owner to executor");
        assert!(!bridge
            .stage_run_reentry_guard
            .is_exhausted(StageKind::Enumeration));

        bridge
            .stage_run_reentry_guard
            .mark_exhausted(StageKind::Enumeration);
        assert!(
            executor.stage_run_retry_budget_exhausted(StageKind::Enumeration),
            "the orchestrator-facing executor must observe exhaustion from this request"
        );
        let nested = BridgeAgentExecutor::from_request(bridge.clone(), lead_owner.clone())
            .expect("same request does not recursively acquire");
        assert!(bridge
            .stage_run_reentry_guard
            .is_exhausted(StageKind::Enumeration));
        assert!(bridge.begin_top_level_request().await.is_err());

        drop(nested);
        drop(executor);
        drop(lead_owner);
        let next_owner = bridge
            .begin_top_level_request()
            .await
            .expect("a separate user request acquires a fresh lease");
        let next_executor = BridgeAgentExecutor::from_request(bridge.clone(), next_owner)
            .expect("separate user request initializes a fresh Task budget");
        assert!(
            !next_executor.stage_run_retry_budget_exhausted(StageKind::Enumeration),
            "a separate user continuation must see the request-scoped guard reset"
        );
    }

    #[tokio::test]
    async fn foreign_generation_lease_cannot_build_executor_or_clear_other_bridge() {
        let (_workspace_a, bridge_a) = real_bridge().await;
        let (_workspace_b, bridge_b) = real_bridge().await;
        let owner_a = bridge_a.begin_top_level_request().await.unwrap();

        assert!(
            BridgeAgentExecutor::from_request(bridge_b.clone(), owner_a.clone()).is_err(),
            "foreign lease must not initialize the other bridge's Task guard"
        );
        assert!(bridge_b
            .clear_top_level_request_state(&owner_a)
            .await
            .is_err());
        assert!(
            bridge_a.begin_top_level_request().await.is_err(),
            "failed foreign handoff must not release the source owner clone"
        );

        let owner_b = bridge_b.begin_top_level_request().await.unwrap();
        drop(owner_b);
        drop(owner_a);
        assert!(bridge_a.begin_top_level_request().await.is_ok());
    }

    #[tokio::test]
    async fn cancellation_epoch_preserves_stop_racing_with_owner_acquisition() {
        let (_workspace, bridge) = real_bridge().await;

        let observed = bridge.cancel_epoch.load(Ordering::Acquire);
        bridge.cancel();
        bridge.reset_cancelled_unless_epoch_advanced(observed);
        assert!(
            bridge.is_cancelled(),
            "a Stop after the request epoch snapshot must survive acquisition reset"
        );

        let stale_epoch = bridge.cancel_epoch.load(Ordering::Acquire);
        bridge.reset_cancelled_unless_epoch_advanced(stale_epoch);
        assert!(
            !bridge.is_cancelled(),
            "a cancellation predating the new request is cleared"
        );
    }

    #[tokio::test]
    async fn background_listener_lifecycle_is_single_claim_and_permanently_retired() {
        let (_workspace, bridge) = real_bridge().await;
        let receiver = bridge
            .claim_background_listener_lifecycle()
            .expect("first post-publish claim succeeds");
        assert!(!*receiver.borrow());
        assert!(bridge.claim_background_listener_lifecycle().is_none());

        bridge.retire_session_generation();
        assert!(*receiver.borrow());
        assert!(bridge.claim_background_listener_lifecycle().is_none());
    }
}
