//! Pure decision contract for the durable Candidate review barrier.
//!
//! The caller must build [`ReviewBarrierSnapshot`] from one exact, locked DB
//! wave snapshot. No trace, in-memory wake flag, or model assertion participates.

use super::AttackExecutionError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBarrierSnapshot {
    pub wave_unit_count: usize,
    pub review_closed_unit_count: usize,
    pub candidate_count: usize,
    pub proposed_candidate_count: usize,
    pub durable_status: String,
    pub dispatch_is_stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewBarrierAction {
    KeepOpen,
    SetResumePending,
    AwaitResume,
    KeepDispatching,
    ResetStaleDispatch,
    Resumed,
    Terminal,
}

pub fn decide_review_barrier(
    snapshot: &ReviewBarrierSnapshot,
) -> Result<ReviewBarrierAction, AttackExecutionError> {
    if snapshot.wave_unit_count == 0
        || snapshot.review_closed_unit_count > snapshot.wave_unit_count
        || snapshot.proposed_candidate_count > snapshot.candidate_count
    {
        return Err(AttackExecutionError::new(
            "ATTACK_REVIEW_SCOPE_MISMATCH",
            "Candidate review barrier snapshot is incomplete or inconsistent",
        ));
    }

    if snapshot.proposed_candidate_count > 0
        || snapshot.review_closed_unit_count != snapshot.wave_unit_count
    {
        return Ok(ReviewBarrierAction::KeepOpen);
    }

    match snapshot.durable_status.as_str() {
        "open" => Ok(ReviewBarrierAction::SetResumePending),
        "resume_pending" => Ok(ReviewBarrierAction::AwaitResume),
        "dispatching" if snapshot.dispatch_is_stale => Ok(ReviewBarrierAction::ResetStaleDispatch),
        "dispatching" => Ok(ReviewBarrierAction::KeepDispatching),
        "resumed" => Ok(ReviewBarrierAction::Resumed),
        "terminal" => Ok(ReviewBarrierAction::Terminal),
        _ => Err(AttackExecutionError::new(
            "ATTACK_REVIEW_SCOPE_MISMATCH",
            "Unknown durable Candidate review barrier status",
        )),
    }
}
