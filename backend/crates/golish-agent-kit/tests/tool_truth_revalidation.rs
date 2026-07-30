use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use golish_agent_kit::task_orchestrator::tool_truth_revalidation::{
    can_risk_accept_exhaustion, evaluate_dispatch, ClaimedRevalidation, FrozenRevalidationPolicy,
    RevalidationCandidate, RevalidationDispatchDecision, RevalidationDispatchHead,
    RevalidationDispatchMode, RevalidationExecutionOutcome, RevalidationRiskTier,
    ToolTruthRevalidationExecutor, ToolTruthRevalidationOrchestrator, ToolTruthRevalidationStore,
};
use uuid::Uuid;

fn policy() -> FrozenRevalidationPolicy {
    FrozenRevalidationPolicy {
        dispatch_mode: RevalidationDispatchMode::AutoPassiveT0T1,
        max_risk_tier: RevalidationRiskTier::T1,
        max_attempts: 3,
        max_retries: 2,
        max_no_progress: 2,
    }
}

fn head() -> RevalidationDispatchHead {
    RevalidationDispatchHead {
        released: true,
        generation: 1,
        row_version: 1,
    }
}

fn candidate() -> RevalidationCandidate {
    RevalidationCandidate {
        obligation_id: Uuid::new_v4(),
        operation_active: true,
        risk_tier: RevalidationRiskTier::T1,
        requires_prepared_action: false,
        scope_current: true,
        destination_policy_current: true,
        temporal_policy_current: true,
        budget_available: true,
        continuation_allowed: true,
        attempt_count: 0,
        retry_count: 0,
        no_progress_count: 0,
        deadline_elapsed: false,
    }
}

#[test]
fn tool_truth_revalidation_manual_or_held_policy_dispatches_nothing() {
    let candidate = candidate();
    let mut frozen_policy = policy();
    frozen_policy.dispatch_mode = RevalidationDispatchMode::ManualOnly;
    assert_eq!(
        evaluate_dispatch(&frozen_policy, &head(), &candidate),
        RevalidationDispatchDecision::Hold("manual_only")
    );
    let mut held = head();
    held.released = false;
    assert_eq!(
        evaluate_dispatch(&policy(), &held, &candidate),
        RevalidationDispatchDecision::Hold("dispatch_held")
    );
}

#[test]
fn tool_truth_revalidation_t2_t3_require_prepared_action_and_never_auto_dispatch() {
    for risk_tier in [RevalidationRiskTier::T2, RevalidationRiskTier::T3] {
        let mut candidate = candidate();
        candidate.risk_tier = risk_tier;
        assert_eq!(
            evaluate_dispatch(&policy(), &head(), &candidate),
            RevalidationDispatchDecision::Hold("prepared_action_required")
        );
    }
}

#[test]
fn tool_truth_revalidation_scope_budget_and_no_progress_fail_closed() {
    let mut blocked = candidate();
    blocked.budget_available = false;
    assert_eq!(
        evaluate_dispatch(&policy(), &head(), &blocked),
        RevalidationDispatchDecision::Hold("authority_or_budget_blocked")
    );
    let mut exhausted = candidate();
    exhausted.no_progress_count = 2;
    assert_eq!(
        evaluate_dispatch(&policy(), &head(), &exhausted),
        RevalidationDispatchDecision::Exhausted("bounded_revalidation_exhausted")
    );
}

#[test]
fn tool_truth_revalidation_mandatory_axis_cannot_be_risk_accepted() {
    assert!(!can_risk_accept_exhaustion(true, true, true));
    assert!(!can_risk_accept_exhaustion(false, false, true));
    assert!(can_risk_accept_exhaustion(false, true, true));
}

struct EmptyStore;

#[async_trait]
impl ToolTruthRevalidationStore for EmptyStore {
    async fn claim_next(&self, _owner: &str) -> anyhow::Result<Option<ClaimedRevalidation>> {
        Ok(None)
    }

    async fn complete_success(
        &self,
        _owner: &str,
        _claim: &ClaimedRevalidation,
        _replacement_denominator_id: Uuid,
        _replacement_receipt_id: Uuid,
    ) -> anyhow::Result<()> {
        panic!("no claim means no completion")
    }

    async fn record_failure(
        &self,
        _owner: &str,
        _claim: &ClaimedRevalidation,
        _progress_fingerprint: &str,
        _reason_code: &str,
    ) -> anyhow::Result<()> {
        panic!("no claim means no failure")
    }
}

struct PanicExecutor;

#[async_trait]
impl ToolTruthRevalidationExecutor for PanicExecutor {
    async fn execute(
        &self,
        _claim: &ClaimedRevalidation,
    ) -> anyhow::Result<RevalidationExecutionOutcome> {
        panic!("consumer/read path must never call an executor")
    }
}

#[tokio::test]
async fn tool_truth_revalidation_no_claim_means_zero_executor_calls() {
    let orchestrator = ToolTruthRevalidationOrchestrator::new("worker", EmptyStore, PanicExecutor);
    assert!(!orchestrator
        .run_once()
        .await
        .expect("empty loop is healthy"));
}

struct SuccessStore<'a> {
    completions: &'a AtomicUsize,
}

#[async_trait]
impl ToolTruthRevalidationStore for SuccessStore<'_> {
    async fn claim_next(&self, _owner: &str) -> anyhow::Result<Option<ClaimedRevalidation>> {
        Ok(Some(ClaimedRevalidation {
            obligation_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            claim_token: Uuid::new_v4(),
            row_version: 1,
            source_receipt_id: Uuid::new_v4(),
            source_input_key: "exact-input".to_string(),
        }))
    }

    async fn complete_success(
        &self,
        _owner: &str,
        claim: &ClaimedRevalidation,
        _replacement_denominator_id: Uuid,
        replacement_receipt_id: Uuid,
    ) -> anyhow::Result<()> {
        assert_ne!(claim.source_receipt_id, replacement_receipt_id);
        self.completions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn record_failure(
        &self,
        _owner: &str,
        _claim: &ClaimedRevalidation,
        _progress_fingerprint: &str,
        _reason_code: &str,
    ) -> anyhow::Result<()> {
        panic!("success executor does not record failure")
    }
}

struct SuccessExecutor;

#[async_trait]
impl ToolTruthRevalidationExecutor for SuccessExecutor {
    async fn execute(
        &self,
        _claim: &ClaimedRevalidation,
    ) -> anyhow::Result<RevalidationExecutionOutcome> {
        Ok(RevalidationExecutionOutcome::Succeeded {
            replacement_denominator_id: Uuid::new_v4(),
            replacement_receipt_id: Uuid::new_v4(),
        })
    }
}

#[tokio::test]
async fn tool_truth_revalidation_success_uses_a_replacement_receipt() {
    let completions = AtomicUsize::new(0);
    let orchestrator = ToolTruthRevalidationOrchestrator::new(
        "worker",
        SuccessStore {
            completions: &completions,
        },
        SuccessExecutor,
    );
    assert!(orchestrator
        .run_once()
        .await
        .expect("one bounded claim runs"));
    assert_eq!(completions.load(Ordering::SeqCst), 1);
}
