//! Host-owned automatic verification admission and task identity contracts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const HYPOTHESIS_VERIFICATION_TASK_CONTRACT_V1: &str = "hypothesis_verification_task.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationAdmissionDispositionV1 {
    Scheduled,
    NeedsEnrichment,
    Deferred,
    OutOfScope,
    Unsafe,
    AlreadyTerminal,
    NoNewObligation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HypothesisVerificationTaskHeaderV1 {
    pub task_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_sha256: String,
    pub relevant_evidence_snapshot_id: Uuid,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub semantic_attempt_fingerprint: String,
    pub task_contract_version: String,
    pub first_admission_generation_id: Uuid,
    pub host_rerun_receipt_id: Option<Uuid>,
    pub host_rerun_receipt_sha256: Option<String>,
    pub rerun_contract_version: Option<u32>,
    pub stable_task_key_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHypothesisVerificationTaskV1 {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_sha256: String,
    pub relevant_evidence_snapshot_id: Uuid,
    pub semantic_evidence_set_sha256: String,
    pub open_obligation_set_sha256: String,
    pub semantic_attempt_fingerprint: String,
    pub first_admission_generation_id: Uuid,
    pub host_rerun_receipt_id: Option<Uuid>,
    pub host_rerun_receipt_sha256: Option<String>,
    pub rerun_contract_version: Option<u32>,
}

#[derive(Debug, Serialize)]
struct StableTaskKeyMaterial<'a> {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
    scope_snapshot_id: Uuid,
    hypothesis_revision_id: Uuid,
    hypothesis_revision_sha256: &'a str,
    verification_plan_sha256: &'a str,
    semantic_evidence_set_sha256: &'a str,
    open_obligation_set_sha256: &'a str,
    semantic_attempt_fingerprint: &'a str,
    task_contract_version: &'static str,
    host_rerun_receipt_id: Option<Uuid>,
    host_rerun_receipt_sha256: Option<&'a str>,
    rerun_contract_version: Option<u32>,
}

impl HypothesisVerificationTaskHeaderV1 {
    pub fn host_create(
        input: NewHypothesisVerificationTaskV1,
    ) -> Result<Self, HypothesisVerificationTaskError> {
        for (field, value) in [
            ("operation_id", input.operation_id),
            ("stage_execution_id", input.stage_execution_id),
            ("stage_run_unit_id", input.stage_run_unit_id),
            ("organization_id", input.organization_id),
            ("scope_snapshot_id", input.scope_snapshot_id),
            ("hypothesis_revision_id", input.hypothesis_revision_id),
            (
                "relevant_evidence_snapshot_id",
                input.relevant_evidence_snapshot_id,
            ),
            (
                "first_admission_generation_id",
                input.first_admission_generation_id,
            ),
        ] {
            if value.is_nil() {
                return Err(HypothesisVerificationTaskError::InvalidIdentity(field));
            }
        }
        for (field, value) in [
            (
                "hypothesis_revision_sha256",
                input.hypothesis_revision_sha256.as_str(),
            ),
            (
                "verification_plan_sha256",
                input.verification_plan_sha256.as_str(),
            ),
            (
                "semantic_evidence_set_sha256",
                input.semantic_evidence_set_sha256.as_str(),
            ),
            (
                "open_obligation_set_sha256",
                input.open_obligation_set_sha256.as_str(),
            ),
            (
                "semantic_attempt_fingerprint",
                input.semantic_attempt_fingerprint.as_str(),
            ),
        ] {
            validate_sha256(value, field)?;
        }
        match (
            input.host_rerun_receipt_id,
            input.host_rerun_receipt_sha256.as_deref(),
            input.rerun_contract_version,
        ) {
            (None, None, None) => {}
            (Some(id), Some(hash), Some(version)) if !id.is_nil() && version > 0 => {
                validate_sha256(hash, "host_rerun_receipt_sha256")?;
            }
            _ => return Err(HypothesisVerificationTaskError::InvalidRerunReceipt),
        }
        let material = StableTaskKeyMaterial {
            operation_id: input.operation_id,
            stage_execution_id: input.stage_execution_id,
            stage_run_unit_id: input.stage_run_unit_id,
            organization_id: input.organization_id,
            scope_snapshot_id: input.scope_snapshot_id,
            hypothesis_revision_id: input.hypothesis_revision_id,
            hypothesis_revision_sha256: &input.hypothesis_revision_sha256,
            verification_plan_sha256: &input.verification_plan_sha256,
            semantic_evidence_set_sha256: &input.semantic_evidence_set_sha256,
            open_obligation_set_sha256: &input.open_obligation_set_sha256,
            semantic_attempt_fingerprint: &input.semantic_attempt_fingerprint,
            task_contract_version: HYPOTHESIS_VERIFICATION_TASK_CONTRACT_V1,
            host_rerun_receipt_id: input.host_rerun_receipt_id,
            host_rerun_receipt_sha256: input.host_rerun_receipt_sha256.as_deref(),
            rerun_contract_version: input.rerun_contract_version,
        };
        let stable_task_key_sha256 = sha256_json(&material);
        let task_id = Uuid::new_v5(&input.operation_id, stable_task_key_sha256.as_bytes());
        Ok(Self {
            task_id,
            operation_id: input.operation_id,
            stage_execution_id: input.stage_execution_id,
            stage_run_unit_id: input.stage_run_unit_id,
            organization_id: input.organization_id,
            scope_snapshot_id: input.scope_snapshot_id,
            hypothesis_revision_id: input.hypothesis_revision_id,
            hypothesis_revision_sha256: input.hypothesis_revision_sha256,
            verification_plan_sha256: input.verification_plan_sha256,
            relevant_evidence_snapshot_id: input.relevant_evidence_snapshot_id,
            semantic_evidence_set_sha256: input.semantic_evidence_set_sha256,
            open_obligation_set_sha256: input.open_obligation_set_sha256,
            semantic_attempt_fingerprint: input.semantic_attempt_fingerprint,
            task_contract_version: HYPOTHESIS_VERIFICATION_TASK_CONTRACT_V1.into(),
            first_admission_generation_id: input.first_admission_generation_id,
            host_rerun_receipt_id: input.host_rerun_receipt_id,
            host_rerun_receipt_sha256: input.host_rerun_receipt_sha256,
            rerun_contract_version: input.rerun_contract_version,
            stable_task_key_sha256,
        })
    }

    pub fn same_semantic_task(&self, other: &Self) -> bool {
        self.stable_task_key_sha256 == other.stable_task_key_sha256 && self.task_id == other.task_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisVerificationTaskStateV1 {
    Admitted,
    Queued,
    Planning,
    Running,
    AwaitingAuthorization,
    Consolidating,
    StopPending,
    Draining,
    Cancelled,
    Blocked,
    RecoveryRequired,
    Terminal,
}

impl HypothesisVerificationTaskStateV1 {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Blocked | Self::Terminal)
    }

    pub const fn can_transition_to(self, next: Self) -> bool {
        use HypothesisVerificationTaskStateV1 as State;
        matches!(
            (self, next),
            (State::Admitted, State::Queued)
                | (
                    State::Queued,
                    State::Planning | State::Cancelled | State::RecoveryRequired
                )
                | (
                    State::Planning,
                    State::Running | State::Blocked | State::RecoveryRequired
                )
                | (
                    State::Running,
                    State::AwaitingAuthorization
                        | State::Consolidating
                        | State::StopPending
                        | State::Blocked
                        | State::RecoveryRequired
                )
                | (
                    State::AwaitingAuthorization,
                    State::Running | State::StopPending | State::Blocked
                )
                | (
                    State::Consolidating,
                    State::Terminal | State::Blocked | State::RecoveryRequired | State::StopPending
                )
                | (State::StopPending, State::Draining)
                | (
                    State::Draining,
                    State::Cancelled | State::RecoveryRequired | State::Consolidating
                )
                | (State::RecoveryRequired, State::Queued | State::Blocked)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskObjectiveAssignmentV1 {
    Campaign {
        campaign_id: Uuid,
    },
    AlreadySatisfied {
        objective_adjudication_id: Uuid,
        adjudication_sha256: String,
        semantic_evidence_set_sha256: String,
    },
    Residual {
        residual_kind: TaskObjectiveResidualKindV1,
        reason_code: String,
        owner: String,
        next_action: String,
        residual_receipt_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskObjectiveResidualKindV1 {
    NoKnownCapability,
    NeedsEnrichment,
    Deferred,
    OutOfScope,
    Unsafe,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskObjectiveOutcomeV1 {
    CampaignTerminal {
        campaign_id: Uuid,
        terminal_receipt_id: Uuid,
    },
    CancelledBeforeStart {
        campaign_id: Uuid,
        stop_receipt_id: Uuid,
    },
    RecoveryRequired {
        campaign_id: Uuid,
        recovery_receipt_id: Uuid,
    },
}

impl TaskObjectiveOutcomeV1 {
    pub const fn campaign_id(&self) -> Uuid {
        match self {
            Self::CampaignTerminal { campaign_id, .. }
            | Self::CancelledBeforeStart { campaign_id, .. }
            | Self::RecoveryRequired { campaign_id, .. } => *campaign_id,
        }
    }
}

pub fn validate_task_objective_closure(
    assignments: &[TaskObjectiveAssignmentV1],
    outcomes: &[TaskObjectiveOutcomeV1],
) -> Result<(), HypothesisVerificationTaskError> {
    let mut campaign_assignments = BTreeSet::new();
    for assignment in assignments {
        match assignment {
            TaskObjectiveAssignmentV1::Campaign { campaign_id } => {
                if campaign_id.is_nil() || !campaign_assignments.insert(*campaign_id) {
                    return Err(HypothesisVerificationTaskError::DuplicateCampaign);
                }
            }
            TaskObjectiveAssignmentV1::AlreadySatisfied {
                objective_adjudication_id,
                adjudication_sha256,
                semantic_evidence_set_sha256,
            } => {
                if objective_adjudication_id.is_nil() {
                    return Err(HypothesisVerificationTaskError::InvalidObjectiveAssignment);
                }
                validate_sha256(adjudication_sha256, "adjudication_sha256")?;
                validate_sha256(semantic_evidence_set_sha256, "semantic_evidence_set_sha256")?;
            }
            TaskObjectiveAssignmentV1::Residual {
                reason_code,
                owner,
                next_action,
                residual_receipt_id,
                ..
            } => {
                if residual_receipt_id.is_nil()
                    || [reason_code, owner, next_action]
                        .iter()
                        .any(|value| value.trim().is_empty() || value.len() > 1_024)
                {
                    return Err(HypothesisVerificationTaskError::InvalidObjectiveAssignment);
                }
            }
        }
    }
    let mut outcome_campaigns = BTreeSet::new();
    for outcome in outcomes {
        let campaign_id = outcome.campaign_id();
        let receipt_id = match outcome {
            TaskObjectiveOutcomeV1::CampaignTerminal {
                terminal_receipt_id,
                ..
            } => *terminal_receipt_id,
            TaskObjectiveOutcomeV1::CancelledBeforeStart {
                stop_receipt_id, ..
            } => *stop_receipt_id,
            TaskObjectiveOutcomeV1::RecoveryRequired {
                recovery_receipt_id,
                ..
            } => *recovery_receipt_id,
        };
        if campaign_id.is_nil() || receipt_id.is_nil() || !outcome_campaigns.insert(campaign_id) {
            return Err(HypothesisVerificationTaskError::DuplicateCampaignOutcome);
        }
    }
    if campaign_assignments != outcome_campaigns {
        return Err(HypothesisVerificationTaskError::CampaignOutcomeSetMismatch);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum HypothesisVerificationTaskError {
    #[error("invalid hypothesis task identity: {0}")]
    InvalidIdentity(&'static str),
    #[error("invalid hypothesis task hash: {0}")]
    InvalidHash(&'static str),
    #[error("host rerun receipt fields must be present together")]
    InvalidRerunReceipt,
    #[error("invalid objective assignment")]
    InvalidObjectiveAssignment,
    #[error("duplicate campaign assignment")]
    DuplicateCampaign,
    #[error("duplicate campaign outcome")]
    DuplicateCampaignOutcome,
    #[error("campaign outcome set differs from assignment set")]
    CampaignOutcomeSetMismatch,
}

fn validate_sha256(
    value: &str,
    field: &'static str,
) -> Result<(), HypothesisVerificationTaskError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(HypothesisVerificationTaskError::InvalidHash(field));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(HypothesisVerificationTaskError::InvalidHash(field));
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("hypothesis task identity material is serializable"),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect::<String>();
    format!("sha256:{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn input() -> NewHypothesisVerificationTaskV1 {
        NewHypothesisVerificationTaskV1 {
            operation_id: Uuid::from_u128(1),
            stage_execution_id: Uuid::from_u128(2),
            stage_run_unit_id: Uuid::from_u128(3),
            organization_id: Uuid::from_u128(4),
            scope_snapshot_id: Uuid::from_u128(5),
            hypothesis_revision_id: Uuid::from_u128(6),
            hypothesis_revision_sha256: hash('a'),
            verification_plan_sha256: hash('b'),
            relevant_evidence_snapshot_id: Uuid::from_u128(7),
            semantic_evidence_set_sha256: hash('c'),
            open_obligation_set_sha256: hash('d'),
            semantic_attempt_fingerprint: hash('e'),
            first_admission_generation_id: Uuid::from_u128(8),
            host_rerun_receipt_id: None,
            host_rerun_receipt_sha256: None,
            rerun_contract_version: None,
        }
    }

    #[test]
    fn hypothesis_verification_task_reuses_identity_across_admission_generations_and_snapshots() {
        let first = HypothesisVerificationTaskHeaderV1::host_create(input()).unwrap();
        let mut replay = input();
        replay.first_admission_generation_id = Uuid::from_u128(99);
        replay.relevant_evidence_snapshot_id = Uuid::from_u128(100);
        let replay = HypothesisVerificationTaskHeaderV1::host_create(replay).unwrap();
        assert!(first.same_semantic_task(&replay));
    }

    #[test]
    fn hypothesis_verification_task_material_change_or_host_rerun_changes_identity() {
        let first = HypothesisVerificationTaskHeaderV1::host_create(input()).unwrap();
        let mut material = input();
        material.semantic_evidence_set_sha256 = hash('f');
        let material = HypothesisVerificationTaskHeaderV1::host_create(material).unwrap();
        assert!(!first.same_semantic_task(&material));

        let mut rerun = input();
        rerun.host_rerun_receipt_id = Some(Uuid::from_u128(9));
        rerun.host_rerun_receipt_sha256 = Some(hash('1'));
        rerun.rerun_contract_version = Some(1);
        let rerun = HypothesisVerificationTaskHeaderV1::host_create(rerun).unwrap();
        assert!(!first.same_semantic_task(&rerun));
    }

    #[test]
    fn hypothesis_verification_task_state_machine_rejects_post_terminal_append() {
        assert!(HypothesisVerificationTaskStateV1::Admitted
            .can_transition_to(HypothesisVerificationTaskStateV1::Queued));
        assert!(!HypothesisVerificationTaskStateV1::Terminal
            .can_transition_to(HypothesisVerificationTaskStateV1::Queued));
        assert!(!HypothesisVerificationTaskStateV1::Cancelled
            .can_transition_to(HypothesisVerificationTaskStateV1::Running));
    }

    #[test]
    fn hypothesis_verification_task_outcome_set_exactly_matches_campaign_assignments() {
        let campaign = Uuid::from_u128(10);
        let assignments = vec![TaskObjectiveAssignmentV1::Campaign {
            campaign_id: campaign,
        }];
        let outcomes = vec![TaskObjectiveOutcomeV1::CampaignTerminal {
            campaign_id: campaign,
            terminal_receipt_id: Uuid::from_u128(11),
        }];
        validate_task_objective_closure(&assignments, &outcomes).unwrap();
        assert_eq!(
            validate_task_objective_closure(&assignments, &[]),
            Err(HypothesisVerificationTaskError::CampaignOutcomeSetMismatch)
        );
    }

    #[test]
    fn hypothesis_verification_task_zero_campaign_requires_only_explicit_residuals() {
        let assignments = vec![TaskObjectiveAssignmentV1::Residual {
            residual_kind: TaskObjectiveResidualKindV1::NoKnownCapability,
            reason_code: "no_known_capability".into(),
            owner: "coverage_reviewer".into(),
            next_action: "install_or_register_a_scoped_capability".into(),
            residual_receipt_id: Uuid::from_u128(12),
        }];
        validate_task_objective_closure(&assignments, &[]).unwrap();
    }
}
