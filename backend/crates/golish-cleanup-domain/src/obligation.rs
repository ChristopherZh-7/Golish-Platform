use chrono::{DateTime, Utc};
use golish_post_exploit_domain::{ActionId, PostExploitAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::ResidualRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupObligationStatus {
    Open,
    InProgress,
    VerifiedAbsent,
    Blocked,
    WaivedByUser,
}

impl CleanupObligationStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::VerifiedAbsent | Self::Blocked | Self::WaivedByUser
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CleanupObligationId(pub Uuid);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbsenceProofRequirement {
    pub kind: String,
    pub independent_verifier_required: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupObligation {
    pub id: CleanupObligationId,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source_action_id: ActionId,
    pub source_action_plan_hash: String,
    pub affected_resource_snapshot: Value,
    pub resource_identity_hash: String,
    pub cleanup_strategy: Value,
    pub proof_requirements: Vec<AbsenceProofRequirement>,
    pub deadline: DateTime<Utc>,
    pub status: CleanupObligationStatus,
    pub residual_risk: Option<ResidualRisk>,
    pub row_version: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewCleanupObligation {
    pub id: CleanupObligationId,
    pub source_action_id: ActionId,
    pub source_action_plan_hash: String,
    pub affected_resource_snapshot: Value,
    pub resource_identity_hash: String,
    pub cleanup_strategy: Value,
    pub proof_requirements: Vec<AbsenceProofRequirement>,
    pub deadline: DateTime<Utc>,
    pub evidence_ids: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingSideEffectAction {
    pub action: PostExploitAction,
    pub scope_snapshot_id: Uuid,
    pub evidence_ids: Vec<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WaiverRequest {
    pub id: Uuid,
    pub obligation_id: CleanupObligationId,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub expected_row_version: i64,
    pub reason: String,
    pub residual_risk: ResidualRisk,
    pub evidence_ids: Vec<i64>,
}

/// Opaque principal formed only after a server-owned principal row was loaded.
/// It intentionally has no serde implementation and cannot cross IPC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedOperatorPrincipal {
    id: Uuid,
}

impl TrustedOperatorPrincipal {
    pub fn from_server_record(id: Uuid) -> Self {
        Self { id }
    }

    pub const fn id(&self) -> Uuid {
        self.id
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CleanupError {
    #[error("cleanup attempt transition is invalid")]
    InvalidAttemptTransition,
    #[error("cleanup obligation is terminal")]
    TerminalObligation,
    #[error("cleanup action and obligation do not describe one exact side effect")]
    ActionObligationMismatch,
    #[error("cleanup evidence is missing or malformed")]
    InvalidEvidence,
    #[error("cleanup resource snapshot is missing or malformed")]
    InvalidResourceSnapshot,
    #[error("cleanup operator is not a trusted active principal")]
    UntrustedOperator,
    #[error("cleanup canonical scope is not authorized")]
    ScopeNotAuthorized,
    #[error("cleanup repository failed: {0}")]
    Repository(String),
}

impl CleanupError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidAttemptTransition => "cleanup_attempt_transition_invalid",
            Self::TerminalObligation => "cleanup_obligation_terminal",
            Self::ActionObligationMismatch => "cleanup_action_obligation_mismatch",
            Self::InvalidEvidence => "cleanup_evidence_invalid",
            Self::InvalidResourceSnapshot => "cleanup_resource_snapshot_invalid",
            Self::UntrustedOperator => "cleanup_operator_untrusted",
            Self::ScopeNotAuthorized => "cleanup_scope_not_authorized",
            Self::Repository(_) => "cleanup_repository_failed",
        }
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn evidence_is_valid(evidence: &[i64]) -> bool {
    if evidence.is_empty() || evidence.len() > 1024 || evidence.iter().any(|id| *id <= 0) {
        return false;
    }
    let mut unique = evidence.to_vec();
    unique.sort_unstable();
    unique.dedup();
    unique.len() == evidence.len()
}

pub fn validate_action_obligation_pair(
    action: &PendingSideEffectAction,
    obligation: &NewCleanupObligation,
) -> Result<(), CleanupError> {
    if !action.action.side_effect_class.requires_cleanup_kernel()
        || action.action.id != obligation.source_action_id
        || action.action.plan_hash != obligation.source_action_plan_hash
    {
        return Err(CleanupError::ActionObligationMismatch);
    }
    if !evidence_is_valid(&action.evidence_ids) || !evidence_is_valid(&obligation.evidence_ids) {
        return Err(CleanupError::InvalidEvidence);
    }
    if !action.action.plan.is_object()
        || !obligation.affected_resource_snapshot.is_object()
        || !obligation.cleanup_strategy.is_object()
        || !is_hash(&action.action.plan_hash)
        || !is_hash(&obligation.resource_identity_hash)
        || obligation.proof_requirements.is_empty()
        || obligation.proof_requirements.len() > 64
        || obligation.proof_requirements.iter().any(|requirement| {
            requirement.kind.trim().is_empty()
                || requirement.kind.len() > 128
                || requirement.kind.chars().any(char::is_control)
        })
    {
        return Err(CleanupError::InvalidResourceSnapshot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use golish_post_exploit_domain::SideEffectClass;

    use super::*;

    fn pair(side_effect_class: SideEffectClass) -> (PendingSideEffectAction, NewCleanupObligation) {
        let action_id = ActionId(Uuid::new_v4());
        let plan_hash = "a".repeat(64);
        (
            PendingSideEffectAction {
                action: PostExploitAction {
                    id: action_id,
                    operation_id: Uuid::new_v4(),
                    project_scope_id: Uuid::new_v4(),
                    organization_id_at_time: Uuid::new_v4(),
                    capability_id: "post_exploit.remote_change".to_string(),
                    side_effect_class,
                    plan: serde_json::json!({"resource": "account"}),
                    plan_hash: plan_hash.clone(),
                },
                scope_snapshot_id: Uuid::new_v4(),
                evidence_ids: vec![1],
            },
            NewCleanupObligation {
                id: CleanupObligationId(Uuid::new_v4()),
                source_action_id: action_id,
                source_action_plan_hash: plan_hash,
                affected_resource_snapshot: serde_json::json!({"kind": "account"}),
                resource_identity_hash: "b".repeat(64),
                cleanup_strategy: serde_json::json!({"kind": "delete_account"}),
                proof_requirements: vec![AbsenceProofRequirement {
                    kind: "independent_lookup".to_string(),
                    independent_verifier_required: true,
                }],
                deadline: Utc::now(),
                evidence_ids: vec![2],
            },
        )
    }

    #[test]
    fn every_side_effect_requires_one_exact_obligation() {
        let (action, obligation) = pair(SideEffectClass::RemoteStateMutation);
        assert_eq!(
            validate_action_obligation_pair(&action, &obligation),
            Ok(())
        );
        let (read_only, obligation) = pair(SideEffectClass::None);
        assert_eq!(
            validate_action_obligation_pair(&read_only, &obligation),
            Err(CleanupError::ActionObligationMismatch)
        );
    }

    #[test]
    fn trusted_principal_has_no_serde_or_public_identity_field() {
        let source = include_str!("obligation.rs");
        let principal = source
            .split("pub struct TrustedOperatorPrincipal")
            .nth(1)
            .unwrap();
        let declaration = principal.split('}').next().unwrap();
        assert!(!declaration.contains("pub id"));
        assert!(!source.contains("Serialize, Deserialize)]\npub struct TrustedOperatorPrincipal"));
    }
}
