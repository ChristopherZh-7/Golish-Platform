//! Pure, DB-free Verification Gate over exact persisted Candidate truth.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptTerminalTruth {
    pub candidate_id: Uuid,
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub status: String,
    pub proof_evidence_ids: Vec<i64>,
    pub refutation_evidence_ids: Vec<i64>,
    pub blocker_evidence_ids: Vec<i64>,
    pub blocker_reason_code: Option<String>,
    pub finding_id: Option<Uuid>,
    pub finding_lineage_exact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResidualRiskTruth {
    pub residual_risk_id: Uuid,
    pub reason_code: String,
    pub disclosure_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationTruthSnapshot {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub review_closed: bool,
    /// Work items that are neither evidence-backed no-candidate decisions nor
    /// Candidates with an ever-approved exact plan and terminal Attempt.
    pub pending_work_items: u32,
    pub approved_ever: u32,
    pub attempts: Vec<AttemptTerminalTruth>,
    pub residual_risks: Vec<ResidualRiskTruth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationUnitAuthority {
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationTruthAuthority {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub expected_units: Vec<VerificationUnitAuthority>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationTruthSet {
    pub authority: VerificationTruthAuthority,
    pub snapshots: Vec<VerificationTruthSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VerificationGateError {
    #[error("verification truth snapshot is missing")]
    MissingSnapshot,
    #[error("verification truth snapshot identity is invalid")]
    InvalidIdentity,
    #[error("candidate review is not closed")]
    ReviewOpen,
    #[error("candidate manifest still has pending work items")]
    PendingWorkItems,
    #[error("approved candidate terminal Attempt set is incomplete")]
    AttemptSetIncomplete,
    #[error("verified Attempt has no exact proof and Finding lineage")]
    VerifiedProofMissing,
    #[error("refuted Attempt has no exact refutation evidence")]
    RefutationMissing,
    #[error("blocked Attempt has neither blocker evidence nor stable reason")]
    BlockerMissing,
    #[error("terminal Attempt evidence roles conflict")]
    EvidenceRoleConflict,
}

pub fn validate_verification_truth(
    snapshot: Option<&VerificationTruthSnapshot>,
) -> Result<(), VerificationGateError> {
    let snapshot = snapshot.ok_or(VerificationGateError::MissingSnapshot)?;
    if snapshot.operation_id.is_nil()
        || snapshot.scope_snapshot_id.is_nil()
        || snapshot.wave_run_id.is_nil()
        || snapshot.wave_unit_id.is_nil()
        || snapshot.organization_id.is_nil()
    {
        return Err(VerificationGateError::InvalidIdentity);
    }
    if !snapshot.review_closed {
        return Err(VerificationGateError::ReviewOpen);
    }
    if snapshot.pending_work_items != 0 {
        return Err(VerificationGateError::PendingWorkItems);
    }
    if snapshot.approved_ever == 0 {
        return if snapshot.attempts.is_empty() {
            Ok(())
        } else {
            Err(VerificationGateError::AttemptSetIncomplete)
        };
    }
    if usize::try_from(snapshot.approved_ever).ok() != Some(snapshot.attempts.len()) {
        return Err(VerificationGateError::AttemptSetIncomplete);
    }
    let mut candidate_ids = HashSet::with_capacity(snapshot.attempts.len());
    let mut attempt_ids = HashSet::with_capacity(snapshot.attempts.len());
    for attempt in &snapshot.attempts {
        if attempt.candidate_id.is_nil()
            || attempt.attempt_id.is_nil()
            || attempt.candidate_plan_hash.trim().is_empty()
            || !candidate_ids.insert(attempt.candidate_id)
            || !attempt_ids.insert(attempt.attempt_id)
        {
            return Err(VerificationGateError::AttemptSetIncomplete);
        }
        validate_terminal_attempt(attempt)?;
    }
    Ok(())
}

/// Validate the complete current-wave snapshot set against the server-owned
/// operation identity. All units must belong to one frozen scope/wave and each
/// organization/unit pair must appear exactly once; a foreign row cannot be
/// hidden inside an otherwise valid vector.
pub fn validate_verification_truth_set(
    truth: &VerificationTruthSet,
) -> Result<(), VerificationGateError> {
    let authority = &truth.authority;
    if authority.operation_id.is_nil()
        || authority.scope_snapshot_id.is_nil()
        || authority.wave_run_id.is_nil()
        || authority.expected_units.is_empty()
        || truth.snapshots.is_empty()
    {
        return Err(VerificationGateError::MissingSnapshot);
    }
    let mut expected_unit_ids = HashSet::with_capacity(authority.expected_units.len());
    let mut expected_organization_ids = HashSet::with_capacity(authority.expected_units.len());
    let mut expected_pairs = HashSet::with_capacity(authority.expected_units.len());
    for unit in &authority.expected_units {
        if unit.wave_unit_id.is_nil()
            || unit.organization_id.is_nil()
            || !expected_unit_ids.insert(unit.wave_unit_id)
            || !expected_organization_ids.insert(unit.organization_id)
            || !expected_pairs.insert((unit.wave_unit_id, unit.organization_id))
        {
            return Err(VerificationGateError::InvalidIdentity);
        }
    }
    if truth.snapshots.len() < authority.expected_units.len() {
        return Err(VerificationGateError::MissingSnapshot);
    }
    if truth.snapshots.len() > authority.expected_units.len() {
        return Err(VerificationGateError::InvalidIdentity);
    }
    let mut actual_pairs = HashSet::with_capacity(truth.snapshots.len());
    for snapshot in &truth.snapshots {
        if snapshot.operation_id != authority.operation_id
            || snapshot.scope_snapshot_id != authority.scope_snapshot_id
            || snapshot.wave_run_id != authority.wave_run_id
            || !expected_pairs.contains(&(snapshot.wave_unit_id, snapshot.organization_id))
            || !actual_pairs.insert((snapshot.wave_unit_id, snapshot.organization_id))
        {
            return Err(VerificationGateError::InvalidIdentity);
        }
        validate_verification_truth(Some(snapshot))?;
    }
    if actual_pairs != expected_pairs {
        return Err(VerificationGateError::MissingSnapshot);
    }
    Ok(())
}

fn valid_evidence(ids: &[i64]) -> bool {
    let mut seen = HashSet::with_capacity(ids.len());
    ids.iter().all(|id| *id > 0 && seen.insert(*id))
}

fn stable_reason_code(reason: &str) -> bool {
    let reason = reason.trim();
    !reason.is_empty()
        && reason.len() <= 64
        && reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn validate_terminal_attempt(attempt: &AttemptTerminalTruth) -> Result<(), VerificationGateError> {
    if !valid_evidence(&attempt.proof_evidence_ids)
        || !valid_evidence(&attempt.refutation_evidence_ids)
        || !valid_evidence(&attempt.blocker_evidence_ids)
    {
        return Err(VerificationGateError::EvidenceRoleConflict);
    }
    let mut role_ids = HashSet::new();
    if attempt
        .proof_evidence_ids
        .iter()
        .chain(&attempt.refutation_evidence_ids)
        .chain(&attempt.blocker_evidence_ids)
        .any(|id| !role_ids.insert(*id))
    {
        return Err(VerificationGateError::EvidenceRoleConflict);
    }
    match attempt.status.as_str() {
        "verified" => {
            if !attempt.refutation_evidence_ids.is_empty()
                || !attempt.blocker_evidence_ids.is_empty()
                || attempt.blocker_reason_code.is_some()
            {
                return Err(VerificationGateError::EvidenceRoleConflict);
            }
            if attempt.proof_evidence_ids.is_empty()
                || attempt.finding_id.is_none()
                || !attempt.finding_lineage_exact
            {
                return Err(VerificationGateError::VerifiedProofMissing);
            }
        }
        "refuted" => {
            if !attempt.proof_evidence_ids.is_empty()
                || !attempt.blocker_evidence_ids.is_empty()
                || attempt.blocker_reason_code.is_some()
                || attempt.finding_id.is_some()
                || attempt.finding_lineage_exact
            {
                return Err(VerificationGateError::EvidenceRoleConflict);
            }
            if attempt.refutation_evidence_ids.is_empty() {
                return Err(VerificationGateError::RefutationMissing);
            }
        }
        "blocked" => {
            if !attempt.proof_evidence_ids.is_empty()
                || !attempt.refutation_evidence_ids.is_empty()
                || attempt.finding_id.is_some()
                || attempt.finding_lineage_exact
            {
                return Err(VerificationGateError::EvidenceRoleConflict);
            }
            let has_reason = attempt
                .blocker_reason_code
                .as_deref()
                .is_some_and(stable_reason_code);
            if !has_reason && attempt.blocker_evidence_ids.is_empty() {
                return Err(VerificationGateError::BlockerMissing);
            }
            if attempt
                .blocker_reason_code
                .as_deref()
                .is_some_and(|reason| !stable_reason_code(reason))
            {
                return Err(VerificationGateError::BlockerMissing);
            }
        }
        _ => return Err(VerificationGateError::AttemptSetIncomplete),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(status: &str) -> AttemptTerminalTruth {
        AttemptTerminalTruth {
            candidate_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            candidate_plan_hash: "sha256:approved-plan".to_string(),
            status: status.to_string(),
            proof_evidence_ids: Vec::new(),
            refutation_evidence_ids: Vec::new(),
            blocker_evidence_ids: Vec::new(),
            blocker_reason_code: None,
            finding_id: None,
            finding_lineage_exact: false,
        }
    }

    fn snapshot(attempts: Vec<AttemptTerminalTruth>) -> VerificationTruthSnapshot {
        VerificationTruthSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            review_closed: true,
            pending_work_items: 0,
            approved_ever: attempts.len() as u32,
            attempts,
            residual_risks: Vec::new(),
        }
    }

    fn truth_set(snapshots: Vec<VerificationTruthSnapshot>) -> VerificationTruthSet {
        let first = snapshots.first().expect("truth set needs a snapshot");
        VerificationTruthSet {
            authority: VerificationTruthAuthority {
                operation_id: first.operation_id,
                scope_snapshot_id: first.scope_snapshot_id,
                wave_run_id: first.wave_run_id,
                expected_units: snapshots
                    .iter()
                    .map(|snapshot| VerificationUnitAuthority {
                        wave_unit_id: snapshot.wave_unit_id,
                        organization_id: snapshot.organization_id,
                    })
                    .collect(),
            },
            snapshots,
        }
    }

    #[test]
    fn verified_without_proof_blocks() {
        assert_eq!(
            validate_verification_truth(Some(&snapshot(vec![attempt("verified")]))),
            Err(VerificationGateError::VerifiedProofMissing)
        );
    }

    #[test]
    fn refuted_without_refutation_blocks() {
        assert_eq!(
            validate_verification_truth(Some(&snapshot(vec![attempt("refuted")]))),
            Err(VerificationGateError::RefutationMissing)
        );
    }

    #[test]
    fn blocked_without_reason_or_blocker_evidence_blocks() {
        assert_eq!(
            validate_verification_truth(Some(&snapshot(vec![attempt("blocked")]))),
            Err(VerificationGateError::BlockerMissing)
        );
    }

    #[test]
    fn empty_verification_passes_only_for_existing_closed_complete_no_candidate_manifest() {
        let empty = snapshot(Vec::new());
        assert_eq!(validate_verification_truth(Some(&empty)), Ok(()));

        let mut open = empty.clone();
        open.review_closed = false;
        assert_eq!(
            validate_verification_truth(Some(&open)),
            Err(VerificationGateError::ReviewOpen)
        );

        let mut pending = empty;
        pending.pending_work_items = 1;
        assert_eq!(
            validate_verification_truth(Some(&pending)),
            Err(VerificationGateError::PendingWorkItems)
        );
    }

    #[test]
    fn missing_or_foreign_db_snapshot_blocks_instead_of_falling_back_to_deliverable() {
        assert_eq!(
            validate_verification_truth(None),
            Err(VerificationGateError::MissingSnapshot)
        );
        let mut foreign = snapshot(Vec::new());
        foreign.operation_id = Uuid::nil();
        assert_eq!(
            validate_verification_truth(Some(&foreign)),
            Err(VerificationGateError::InvalidIdentity)
        );

        let local = snapshot(Vec::new());
        let mut foreign = local.clone();
        foreign.operation_id = Uuid::new_v4();
        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority: truth_set(vec![local]).authority,
                snapshots: vec![foreign],
            }),
            Err(VerificationGateError::InvalidIdentity)
        );
    }

    #[test]
    fn mixed_scope_wave_or_duplicate_unit_snapshot_set_blocks() {
        let first = snapshot(Vec::new());
        let mut second = first.clone();
        second.wave_unit_id = Uuid::new_v4();
        second.organization_id = Uuid::new_v4();
        let exact = truth_set(vec![first.clone(), second.clone()]);
        assert_eq!(validate_verification_truth_set(&exact), Ok(()));

        second.scope_snapshot_id = Uuid::new_v4();
        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority: exact.authority.clone(),
                snapshots: vec![first.clone(), second],
            }),
            Err(VerificationGateError::InvalidIdentity)
        );

        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority: exact.authority,
                snapshots: vec![first.clone(), first],
            }),
            Err(VerificationGateError::InvalidIdentity)
        );
    }

    #[test]
    fn server_authority_rejects_all_foreign_missing_and_extra_units() {
        let local = snapshot(Vec::new());
        let authority = VerificationTruthAuthority {
            operation_id: local.operation_id,
            scope_snapshot_id: local.scope_snapshot_id,
            wave_run_id: local.wave_run_id,
            expected_units: vec![VerificationUnitAuthority {
                wave_unit_id: local.wave_unit_id,
                organization_id: local.organization_id,
            }],
        };
        let exact = VerificationTruthSet {
            authority: authority.clone(),
            snapshots: vec![local.clone()],
        };
        assert_eq!(validate_verification_truth_set(&exact), Ok(()));

        let mut foreign = local.clone();
        foreign.scope_snapshot_id = Uuid::new_v4();
        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority: authority.clone(),
                snapshots: vec![foreign],
            }),
            Err(VerificationGateError::InvalidIdentity)
        );
        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority: authority.clone(),
                snapshots: Vec::new(),
            }),
            Err(VerificationGateError::MissingSnapshot)
        );
        let mut extra = local;
        extra.wave_unit_id = Uuid::new_v4();
        extra.organization_id = Uuid::new_v4();
        assert_eq!(
            validate_verification_truth_set(&VerificationTruthSet {
                authority,
                snapshots: vec![exact.snapshots[0].clone(), extra],
            }),
            Err(VerificationGateError::InvalidIdentity)
        );
    }
}
