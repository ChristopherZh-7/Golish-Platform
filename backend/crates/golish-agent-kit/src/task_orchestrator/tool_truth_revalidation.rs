//! Bounded background loop for Tool Truth revalidation.
//!
//! Consumer paths never receive an executor. They only persist obligations via
//! the DB port; this orchestrator is the sole owner of claim + execute.

use async_trait::async_trait;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RevalidationRiskTier {
    T0,
    T1,
    T2,
    T3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidationDispatchMode {
    ManualOnly,
    AutoPassiveT0T1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenRevalidationPolicy {
    pub dispatch_mode: RevalidationDispatchMode,
    pub max_risk_tier: RevalidationRiskTier,
    pub max_attempts: u32,
    pub max_retries: u32,
    pub max_no_progress: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevalidationDispatchHead {
    pub released: bool,
    pub generation: u64,
    pub row_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevalidationCandidate {
    pub obligation_id: Uuid,
    pub operation_active: bool,
    pub risk_tier: RevalidationRiskTier,
    pub requires_prepared_action: bool,
    pub scope_current: bool,
    pub destination_policy_current: bool,
    pub temporal_policy_current: bool,
    pub budget_available: bool,
    pub continuation_allowed: bool,
    pub attempt_count: u32,
    pub retry_count: u32,
    pub no_progress_count: u32,
    pub deadline_elapsed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidationDispatchDecision {
    Dispatch,
    Hold(&'static str),
    Exhausted(&'static str),
}

pub fn evaluate_dispatch(
    policy: &FrozenRevalidationPolicy,
    head: &RevalidationDispatchHead,
    candidate: &RevalidationCandidate,
) -> RevalidationDispatchDecision {
    if !candidate.operation_active {
        return RevalidationDispatchDecision::Hold("operation_inactive");
    }
    if !head.released {
        return RevalidationDispatchDecision::Hold("dispatch_held");
    }
    if policy.dispatch_mode == RevalidationDispatchMode::ManualOnly {
        return RevalidationDispatchDecision::Hold("manual_only");
    }
    if candidate.risk_tier >= RevalidationRiskTier::T2
        || candidate.requires_prepared_action
        || candidate.risk_tier > policy.max_risk_tier
    {
        return RevalidationDispatchDecision::Hold("prepared_action_required");
    }
    if !candidate.scope_current
        || !candidate.destination_policy_current
        || !candidate.temporal_policy_current
        || !candidate.budget_available
        || !candidate.continuation_allowed
    {
        return RevalidationDispatchDecision::Hold("authority_or_budget_blocked");
    }
    if candidate.deadline_elapsed
        || candidate.attempt_count >= policy.max_attempts
        || candidate.retry_count > policy.max_retries
        || candidate.no_progress_count >= policy.max_no_progress
    {
        return RevalidationDispatchDecision::Exhausted("bounded_revalidation_exhausted");
    }
    RevalidationDispatchDecision::Dispatch
}

pub const fn can_risk_accept_exhaustion(
    mandatory_axis: bool,
    exact_typed_residual: bool,
    frozen_continuation_policy_allows: bool,
) -> bool {
    !mandatory_axis && exact_typed_residual && frozen_continuation_policy_allows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRevalidation {
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub claim_token: Uuid,
    pub row_version: i64,
    pub source_receipt_id: Uuid,
    pub source_input_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevalidationExecutionOutcome {
    Succeeded {
        replacement_denominator_id: Uuid,
        replacement_receipt_id: Uuid,
    },
    Failed {
        progress_fingerprint: String,
        reason_code: String,
    },
}

#[async_trait]
pub trait ToolTruthRevalidationStore: Send + Sync {
    async fn claim_next(&self, owner: &str) -> anyhow::Result<Option<ClaimedRevalidation>>;
    async fn complete_success(
        &self,
        owner: &str,
        claim: &ClaimedRevalidation,
        replacement_denominator_id: Uuid,
        replacement_receipt_id: Uuid,
    ) -> anyhow::Result<()>;
    async fn record_failure(
        &self,
        owner: &str,
        claim: &ClaimedRevalidation,
        progress_fingerprint: &str,
        reason_code: &str,
    ) -> anyhow::Result<()>;
}

#[async_trait]
pub trait ToolTruthRevalidationExecutor: Send + Sync {
    async fn execute(
        &self,
        claim: &ClaimedRevalidation,
    ) -> anyhow::Result<RevalidationExecutionOutcome>;
}

pub struct ToolTruthRevalidationOrchestrator<S, E> {
    owner: String,
    store: S,
    executor: E,
}

impl<S, E> ToolTruthRevalidationOrchestrator<S, E>
where
    S: ToolTruthRevalidationStore,
    E: ToolTruthRevalidationExecutor,
{
    pub fn new(owner: impl Into<String>, store: S, executor: E) -> Self {
        Self {
            owner: owner.into(),
            store,
            executor,
        }
    }

    pub async fn run_once(&self) -> anyhow::Result<bool> {
        let Some(claim) = self.store.claim_next(&self.owner).await? else {
            return Ok(false);
        };
        match self.executor.execute(&claim).await? {
            RevalidationExecutionOutcome::Succeeded {
                replacement_denominator_id,
                replacement_receipt_id,
            } => {
                self.store
                    .complete_success(
                        &self.owner,
                        &claim,
                        replacement_denominator_id,
                        replacement_receipt_id,
                    )
                    .await?;
            }
            RevalidationExecutionOutcome::Failed {
                progress_fingerprint,
                reason_code,
            } => {
                self.store
                    .record_failure(&self.owner, &claim, &progress_fingerprint, &reason_code)
                    .await?;
            }
        }
        Ok(true)
    }
}
