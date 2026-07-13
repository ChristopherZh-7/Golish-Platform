//! Pure Candidate-attempt state machine and terminal-result validation.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

use super::types::{AttemptDisposition, CandidateAttemptResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Queued,
    Running,
    Submitted,
    Verified,
    Refuted,
    Blocked,
    RetryableFailed,
    Abandoned,
}

impl AttemptStatus {
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Running,
        Self::Submitted,
        Self::Verified,
        Self::Refuted,
        Self::Blocked,
        Self::RetryableFailed,
        Self::Abandoned,
    ];

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Verified
                | Self::Refuted
                | Self::Blocked
                | Self::RetryableFailed
                | Self::Abandoned
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptEvent {
    Started,
    Submitted,
    Verified,
    Refuted,
    Blocked,
    RetryableFailure,
    Retried,
    Abandoned,
    /// Deliberately unsupported: P1 WorkerRun owns recovery and MVP verifier
    /// execution is foreground-only.
    Backgrounded,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AttackExecutionError {
    code: &'static str,
    message: String,
}

impl AttackExecutionError {
    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn transition_attempt(
    from: AttemptStatus,
    event: AttemptEvent,
) -> Result<AttemptStatus, AttackExecutionError> {
    let next = match (from, event) {
        (AttemptStatus::Queued, AttemptEvent::Started) => AttemptStatus::Running,
        (AttemptStatus::Running, AttemptEvent::Submitted) => AttemptStatus::Submitted,
        (AttemptStatus::Submitted, AttemptEvent::Verified) => AttemptStatus::Verified,
        (AttemptStatus::Submitted, AttemptEvent::Refuted) => AttemptStatus::Refuted,
        (AttemptStatus::Submitted, AttemptEvent::Blocked) => AttemptStatus::Blocked,
        (AttemptStatus::Running, AttemptEvent::RetryableFailure) => AttemptStatus::RetryableFailed,
        (
            AttemptStatus::Queued | AttemptStatus::Running | AttemptStatus::Submitted,
            AttemptEvent::Abandoned,
        ) => AttemptStatus::Abandoned,
        _ => {
            return Err(AttackExecutionError::new(
                "ATTACK_INVALID_ATTEMPT_TRANSITION",
                format!("cannot apply {event:?} to {from:?}"),
            ))
        }
    };
    Ok(next)
}

fn stable_reason_code(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn evidence_ids_are_valid(ids: &[i64]) -> bool {
    let mut unique = HashSet::with_capacity(ids.len());
    ids.iter().all(|id| *id > 0 && unique.insert(*id))
}

fn finding_draft_is_valid(result: &CandidateAttemptResult) -> bool {
    let Some(finding) = result.finding.as_ref() else {
        return true;
    };
    !finding.title.trim().is_empty()
        && finding
            .cvss
            .is_none_or(|score| score.is_finite() && (0.0..=10.0).contains(&score))
        && !finding.affected_target.trim().is_empty()
        && !finding.description.trim().is_empty()
        && !finding.reproduction_steps.is_empty()
        && finding
            .reproduction_steps
            .iter()
            .all(|step| !step.trim().is_empty())
        && !finding.remediation.trim().is_empty()
}

fn fact_deltas_are_valid(result: &CandidateAttemptResult) -> bool {
    result.fact_deltas.iter().all(|delta| {
        !delta.canonical_ref_kind.trim().is_empty()
            && !delta.canonical_ref_id.is_nil()
            && delta.canonical_ref_version > 0
            && !delta.canonical_ref_hash.trim().is_empty()
            && !delta.summary.trim().is_empty()
            && !delta.evidence_ids.is_empty()
            && evidence_ids_are_valid(&delta.evidence_ids)
    })
}

pub fn validate_terminal_result(
    result: &CandidateAttemptResult,
) -> Result<(), AttackExecutionError> {
    if result.attempt_id.is_nil() {
        return Err(AttackExecutionError::new(
            "ATTACK_ATTEMPT_ID_REQUIRED",
            "attempt_id must be non-nil",
        ));
    }
    if result.candidate_plan_hash.trim().is_empty() {
        return Err(AttackExecutionError::new(
            "ATTACK_PLAN_HASH_REQUIRED",
            "candidate_plan_hash is required",
        ));
    }
    if !evidence_ids_are_valid(&result.proof_evidence_ids)
        || !evidence_ids_are_valid(&result.refutation_evidence_ids)
        || !evidence_ids_are_valid(&result.blocker_evidence_ids)
    {
        return Err(AttackExecutionError::new(
            "ATTACK_EVIDENCE_ID_INVALID",
            "evidence ids must be positive",
        ));
    }
    let mut role_ownership = HashSet::new();
    if result
        .proof_evidence_ids
        .iter()
        .chain(&result.refutation_evidence_ids)
        .chain(&result.blocker_evidence_ids)
        .any(|id| !role_ownership.insert(*id))
    {
        return Err(AttackExecutionError::new(
            "ATTACK_EVIDENCE_ROLE_CONFLICT",
            "one evidence row cannot satisfy multiple terminal roles",
        ));
    }
    if !finding_draft_is_valid(result) {
        return Err(AttackExecutionError::new(
            "ATTACK_FINDING_DRAFT_INVALID",
            "finding draft fields must be complete and bounded",
        ));
    }
    if !fact_deltas_are_valid(result) {
        return Err(AttackExecutionError::new(
            "ATTACK_FACT_DELTA_INVALID",
            "FactDelta requires an exact canonical reference and evidence",
        ));
    }

    match result.disposition {
        AttemptDisposition::Verified => {
            if result.proof_evidence_ids.is_empty() {
                return Err(AttackExecutionError::new(
                    "ATTACK_PROOF_REQUIRED",
                    "verified requires proof evidence",
                ));
            }
            if result.finding.is_none() {
                return Err(AttackExecutionError::new(
                    "ATTACK_VERIFIED_FINDING_REQUIRED",
                    "verified requires a finding draft",
                ));
            }
            if !result.refutation_evidence_ids.is_empty()
                || !result.blocker_evidence_ids.is_empty()
                || result.blocker_reason_code.is_some()
            {
                return Err(AttackExecutionError::new(
                    "ATTACK_EVIDENCE_ROLE_CONFLICT",
                    "verified cannot carry refutation or blocker evidence",
                ));
            }
        }
        AttemptDisposition::Refuted => {
            if result.finding.is_some() {
                return Err(AttackExecutionError::new(
                    "ATTACK_REFUTED_FINDING_FORBIDDEN",
                    "refuted cannot create a finding",
                ));
            }
            if result.refutation_evidence_ids.is_empty() {
                return Err(AttackExecutionError::new(
                    "ATTACK_REFUTATION_EVIDENCE_REQUIRED",
                    "refuted requires refutation evidence",
                ));
            }
            if !result.proof_evidence_ids.is_empty()
                || !result.blocker_evidence_ids.is_empty()
                || result.blocker_reason_code.is_some()
            {
                return Err(AttackExecutionError::new(
                    "ATTACK_EVIDENCE_ROLE_CONFLICT",
                    "refuted cannot carry proof or blocker evidence",
                ));
            }
        }
        AttemptDisposition::Blocked => {
            if result.finding.is_some() {
                return Err(AttackExecutionError::new(
                    "ATTACK_BLOCKED_FINDING_FORBIDDEN",
                    "blocked cannot create a finding",
                ));
            }
            let has_reason = result
                .blocker_reason_code
                .as_deref()
                .is_some_and(stable_reason_code);
            if !has_reason && result.blocker_evidence_ids.is_empty() {
                return Err(AttackExecutionError::new(
                    "ATTACK_BLOCK_REASON_REQUIRED",
                    "blocked requires a stable reason code or blocker evidence",
                ));
            }
            if result
                .blocker_reason_code
                .as_deref()
                .is_some_and(|reason| !stable_reason_code(reason))
            {
                return Err(AttackExecutionError::new(
                    "ATTACK_BLOCK_REASON_INVALID",
                    "blocker reason must be a stable snake-case code",
                ));
            }
            if !result.proof_evidence_ids.is_empty() || !result.refutation_evidence_ids.is_empty() {
                return Err(AttackExecutionError::new(
                    "ATTACK_EVIDENCE_ROLE_CONFLICT",
                    "blocked cannot carry proof or refutation evidence",
                ));
            }
        }
    }
    Ok(())
}

/// Validate model-authored terminal business fields against scheduler-owned
/// Attempt identity. Callers must still reload approval, evidence ownership and
/// action-journal truth in the same database transaction before persisting.
pub fn validate_bound_terminal_result(
    result: &CandidateAttemptResult,
    expected_attempt_id: Uuid,
    expected_candidate_plan_hash: &str,
) -> Result<(), AttackExecutionError> {
    if result.attempt_id != expected_attempt_id {
        return Err(AttackExecutionError::new(
            "ATTACK_ATTEMPT_IDENTITY_MISMATCH",
            "submitted attempt_id does not match the scheduler-bound Attempt",
        ));
    }
    if result.candidate_plan_hash != expected_candidate_plan_hash {
        return Err(AttackExecutionError::new(
            "ATTACK_PLAN_HASH_MISMATCH",
            "submitted candidate_plan_hash does not match the approved plan",
        ));
    }
    validate_terminal_result(result)
}

#[cfg(test)]
mod task8_tests {
    use uuid::Uuid;

    use super::*;
    use crate::harness::attack_execution::{
        AttemptDisposition, CandidateAttemptResult, FindingSeverity, VerifiedFindingDraft,
    };

    fn result(disposition: AttemptDisposition) -> CandidateAttemptResult {
        CandidateAttemptResult {
            attempt_id: Uuid::new_v4(),
            candidate_plan_hash: "sha256:approved-plan".to_string(),
            disposition,
            proof_evidence_ids: Vec::new(),
            refutation_evidence_ids: Vec::new(),
            blocker_evidence_ids: Vec::new(),
            blocker_reason_code: None,
            finding: None,
            fact_deltas: Vec::new(),
        }
    }

    #[test]
    fn verified_submission_requires_exact_attempt_proof_and_finding_draft() {
        let expected_attempt_id = Uuid::new_v4();
        let mut submitted = result(AttemptDisposition::Verified);
        submitted.attempt_id = expected_attempt_id;
        submitted.proof_evidence_ids = vec![41];
        submitted.finding = Some(VerifiedFindingDraft {
            title: "Bounded proof".to_string(),
            severity: FindingSeverity::High,
            cvss: Some(8.1),
            affected_target: "https://example.test/login".to_string(),
            description: "The approved action reproduced the hypothesis.".to_string(),
            reproduction_steps: vec!["Replay approved action zero.".to_string()],
            remediation: "Use parameterized queries.".to_string(),
        });

        assert!(validate_bound_terminal_result(
            &submitted,
            expected_attempt_id,
            "sha256:approved-plan"
        )
        .is_ok());

        submitted.attempt_id = Uuid::new_v4();
        assert_eq!(
            validate_bound_terminal_result(&submitted, expected_attempt_id, "sha256:approved-plan")
                .unwrap_err()
                .code(),
            "ATTACK_ATTEMPT_IDENTITY_MISMATCH"
        );
    }

    #[test]
    fn refuted_and_blocked_terminalize_without_finding_and_with_correct_evidence_role() {
        let attempt_id = Uuid::new_v4();
        let mut refuted = result(AttemptDisposition::Refuted);
        refuted.attempt_id = attempt_id;
        refuted.refutation_evidence_ids = vec![51];
        assert!(
            validate_bound_terminal_result(&refuted, attempt_id, "sha256:approved-plan").is_ok()
        );

        let mut blocked = result(AttemptDisposition::Blocked);
        blocked.attempt_id = attempt_id;
        blocked.blocker_evidence_ids = vec![61];
        blocked.blocker_reason_code = Some("approved_action_unavailable".to_string());
        assert!(
            validate_bound_terminal_result(&blocked, attempt_id, "sha256:approved-plan").is_ok()
        );

        blocked.proof_evidence_ids.push(62);
        assert_eq!(
            validate_bound_terminal_result(&blocked, attempt_id, "sha256:approved-plan")
                .unwrap_err()
                .code(),
            "ATTACK_EVIDENCE_ROLE_CONFLICT"
        );
    }

    #[test]
    fn sibling_attempt_proof_cannot_terminalize_candidate() {
        let expected_attempt_id = Uuid::new_v4();
        let mut sibling = result(AttemptDisposition::Verified);
        sibling.proof_evidence_ids = vec![71];
        sibling.finding = Some(VerifiedFindingDraft {
            title: "Sibling proof".to_string(),
            severity: FindingSeverity::Medium,
            cvss: None,
            affected_target: "https://example.test".to_string(),
            description: "Evidence belongs to another attempt.".to_string(),
            reproduction_steps: vec!["Do not accept this sibling result.".to_string()],
            remediation: "Keep attempt ownership exact.".to_string(),
        });

        assert_eq!(
            validate_bound_terminal_result(&sibling, expected_attempt_id, "sha256:approved-plan")
                .unwrap_err()
                .code(),
            "ATTACK_ATTEMPT_IDENTITY_MISMATCH"
        );
    }
}
