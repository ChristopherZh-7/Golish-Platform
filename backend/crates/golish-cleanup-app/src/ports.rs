use async_trait::async_trait;
use golish_cleanup_domain::{
    validate_action_obligation_pair, CleanupError, CleanupObligation, CleanupObligationId,
    CleanupObligationStatus, NewCleanupObligation, PendingSideEffectAction, ResidualRisk,
    TrustedOperatorPrincipal, WaiverRequest,
};
use golish_post_exploit_domain::{ActionId, SideEffectClass};
use std::collections::BTreeSet;

pub type CleanupObligationRecord = golish_db::repo::cleanup_obligations::CleanupObligationRow;
pub type CleanupAttemptRecord = golish_db::repo::cleanup_attempts::CleanupAttemptRow;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupToolAuthorityInput {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub worker_run_id: uuid::Uuid,
    pub lease_token: uuid::Uuid,
    pub attempt_epoch: i64,
    pub tool_call_record_id: uuid::Uuid,
}

#[async_trait]
pub trait CleanupObligationPort: Send + Sync {
    async fn record_action_and_obligation(
        &self,
        action: PendingSideEffectAction,
        obligation: NewCleanupObligation,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<(ActionId, CleanupObligationId), CleanupError>;

    async fn waive_obligation(
        &self,
        request: WaiverRequest,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<CleanupObligation, CleanupError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupCloseoutCounts {
    pub operation_id: uuid::Uuid,
    pub organization_id_at_time: uuid::Uuid,
    pub missing_obligation_count: i64,
    pub nonterminal_obligation_count: i64,
    pub undisclosed_residual_count: i64,
    pub invalid_terminal_truth_count: i64,
    pub residual_obligation_ids: BTreeSet<uuid::Uuid>,
}

impl CleanupCloseoutCounts {
    pub const fn allows_closeout(&self) -> bool {
        self.missing_obligation_count == 0
            && self.nonterminal_obligation_count == 0
            && self.undisclosed_residual_count == 0
            && self.invalid_terminal_truth_count == 0
    }
}

#[async_trait]
pub trait CleanupCloseoutPort: Send + Sync {
    async fn closeout_counts(
        &self,
        operation_id: uuid::Uuid,
        organization_id_at_time: uuid::Uuid,
    ) -> Result<CleanupCloseoutCounts, CleanupError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrganizationDeletionRequestResult {
    pub job_id: uuid::Uuid,
    pub root_organization_id_at_time: uuid::Uuid,
}

#[async_trait]
pub trait OrganizationDeletionPort: Send + Sync {
    /// Resolve the active local C0 principal inside the adapter; callers never
    /// provide an actor id.
    async fn request_organization_deletion(
        &self,
        root_organization_id: uuid::Uuid,
        expected_project_path: &str,
    ) -> Result<OrganizationDeletionRequestResult, CleanupError>;
}

#[derive(Clone, Debug)]
pub struct PgCleanupRepository {
    pool: golish_db::PgPool,
}

impl PgCleanupRepository {
    pub fn new(pool: golish_db::PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &golish_db::PgPool {
        &self.pool
    }

    pub async fn authorize_cleanup_tool(
        &self,
        input: CleanupToolAuthorityInput,
    ) -> Result<(), CleanupError> {
        golish_db::repo::post_exploit_actions::authorize_tool_context(
            &self.pool,
            golish_db::repo::post_exploit_actions::AuthorizePostExploitToolContext {
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                organization_id: input.organization_id,
                worker_run_id: input.worker_run_id,
                lease_token: input.lease_token,
                attempt_epoch: input.attempt_epoch,
                tool_call_record_id: input.tool_call_record_id,
                expected_stage_kind: "cleanup",
            },
        )
        .await
        .map(|_| ())
        .map_err(repository_error)
    }

    pub async fn load_exact_obligation(
        &self,
        operation_id: uuid::Uuid,
        project_scope_id: uuid::Uuid,
        scope_snapshot_id: uuid::Uuid,
        organization_id_at_time: uuid::Uuid,
        obligation_id: uuid::Uuid,
    ) -> Result<CleanupObligationRecord, CleanupError> {
        let row = golish_db::repo::cleanup_obligations::get(&self.pool, obligation_id)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| CleanupError::Repository("cleanup_obligation_not_found".to_string()))?;
        if row.operation_id != operation_id
            || row.project_scope_id != project_scope_id
            || row.scope_snapshot_id != scope_snapshot_id
            || row.organization_id_at_time != organization_id_at_time
        {
            return Err(CleanupError::ScopeNotAuthorized);
        }
        Ok(row)
    }

    /// Loads an obligation for a cleanup stage tool whose live worker lease has
    /// already been authorized for the exact operation and organization.
    ///
    /// Waiver commits must use [`Self::load_exact_obligation`] instead: their
    /// local-operator boundary carries and checks the complete frozen identity.
    pub async fn load_authorized_tool_obligation(
        &self,
        operation_id: uuid::Uuid,
        organization_id_at_time: uuid::Uuid,
        obligation_id: uuid::Uuid,
    ) -> Result<CleanupObligationRecord, CleanupError> {
        let row = golish_db::repo::cleanup_obligations::get(&self.pool, obligation_id)
            .await
            .map_err(repository_error)?
            .ok_or_else(|| CleanupError::Repository("cleanup_obligation_not_found".to_string()))?;
        if row.operation_id != operation_id
            || row.organization_id_at_time != organization_id_at_time
        {
            return Err(CleanupError::ScopeNotAuthorized);
        }
        Ok(row)
    }

    pub async fn list_obligations_for_operation(
        &self,
        operation_id: uuid::Uuid,
    ) -> Result<Vec<CleanupObligationRecord>, CleanupError> {
        golish_db::repo::cleanup_obligations::list_for_operation(&self.pool, operation_id)
            .await
            .map_err(repository_error)
    }

    pub async fn list_attempts(
        &self,
        obligation_id: uuid::Uuid,
    ) -> Result<Vec<CleanupAttemptRecord>, CleanupError> {
        golish_db::repo::cleanup_attempts::list_for_obligation(&self.pool, obligation_id)
            .await
            .map_err(repository_error)
    }

    pub async fn record_executor_unavailable(
        &self,
        obligation_id: uuid::Uuid,
        worker_run_id: uuid::Uuid,
        lease_token: uuid::Uuid,
    ) -> Result<CleanupAttemptRecord, CleanupError> {
        let claimed = golish_db::repo::cleanup_attempts::claim(
            &self.pool,
            &golish_db::repo::cleanup_attempts::ClaimCleanupAttempt {
                obligation_id,
                lease_token,
                lease_expires_at: chrono::Utc::now() + chrono::Duration::seconds(60),
                worker_run_id: Some(worker_run_id),
            },
        )
        .await
        .map_err(repository_error)?;
        let executing = golish_db::repo::cleanup_attempts::transition(
            &self.pool,
            &golish_db::repo::cleanup_attempts::TransitionCleanupAttempt {
                attempt_id: claimed.id,
                lease_token: claimed.lease_token,
                expected_row_version: claimed.row_version,
                expected_status: "claimed".to_string(),
                next_status: "executing".to_string(),
                result: None,
                evidence: Vec::new(),
                terminal_note: None,
            },
        )
        .await
        .map_err(repository_error)?;
        golish_db::repo::cleanup_attempts::transition(
            &self.pool,
            &golish_db::repo::cleanup_attempts::TransitionCleanupAttempt {
                attempt_id: executing.id,
                lease_token: executing.lease_token,
                expected_row_version: executing.row_version,
                expected_status: "executing".to_string(),
                next_status: "execution_failed".to_string(),
                result: Some(serde_json::json!({"code": "cleanup_executor_unavailable"})),
                evidence: Vec::new(),
                terminal_note: Some("cleanup_executor_unavailable".to_string()),
            },
        )
        .await
        .map_err(repository_error)
    }
}

fn repository_error(error: golish_db::DbError) -> CleanupError {
    CleanupError::Repository(error.to_string())
}

#[async_trait]
impl CleanupObligationPort for PgCleanupRepository {
    async fn record_action_and_obligation(
        &self,
        action: PendingSideEffectAction,
        obligation: NewCleanupObligation,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<(ActionId, CleanupObligationId), CleanupError> {
        validate_action_obligation_pair(&action, &obligation)?;
        let side_effect_class = match action.action.side_effect_class {
            SideEffectClass::None => return Err(CleanupError::ActionObligationMismatch),
            SideEffectClass::RemoteStateMutation => "remote_state_mutation",
            SideEffectClass::LocalArtifactMutation => "local_artifact_mutation",
        };
        let proof_requirements = serde_json::to_value(&obligation.proof_requirements)
            .map_err(|error| CleanupError::Repository(error.to_string()))?;
        let action_id = action.action.id;
        let obligation_id = obligation.id;
        golish_db::repo::cleanup_obligations::record_action_and_obligation(
            &self.pool,
            &golish_db::repo::cleanup_obligations::RecordActionAndObligation {
                action_id: action_id.0,
                obligation_id: obligation_id.0,
                operation_id: action.action.operation_id,
                project_scope_id: action.action.project_scope_id,
                scope_snapshot_id: action.scope_snapshot_id,
                organization_id_at_time: action.action.organization_id_at_time,
                principal_id: actor.id(),
                capability_id: action.action.capability_id,
                side_effect_class: side_effect_class.to_string(),
                action_plan: action.action.plan,
                action_plan_hash: action.action.plan_hash,
                action_evidence: action
                    .evidence_ids
                    .into_iter()
                    .map(|id| (id, "plan".to_string()))
                    .collect(),
                affected_resource_snapshot: obligation.affected_resource_snapshot,
                resource_identity_hash: obligation.resource_identity_hash,
                cleanup_strategy: obligation.cleanup_strategy,
                proof_requirements,
                deadline: obligation.deadline,
                obligation_evidence: obligation
                    .evidence_ids
                    .into_iter()
                    .map(|id| (id, "source".to_string()))
                    .collect(),
            },
        )
        .await
        .map_err(repository_error)?;
        Ok((action_id, obligation_id))
    }

    async fn waive_obligation(
        &self,
        request: WaiverRequest,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<CleanupObligation, CleanupError> {
        if !request.residual_risk.validate() {
            return Err(CleanupError::InvalidResourceSnapshot);
        }
        let result = golish_db::repo::cleanup_waivers::waive(
            &self.pool,
            &golish_db::repo::cleanup_waivers::WaiveCleanupObligation {
                id: request.id,
                obligation_id: request.obligation_id.0,
                operation_id: request.operation_id,
                project_scope_id: request.project_scope_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id_at_time: request.organization_id_at_time,
                expected_obligation_row_version: request.expected_row_version,
                principal_id: actor.id(),
                reason: request.reason,
                residual_risk: serde_json::to_value(request.residual_risk)
                    .map_err(|error| CleanupError::Repository(error.to_string()))?,
                evidence: request
                    .evidence_ids
                    .into_iter()
                    .map(|id| (id, "decision".to_string()))
                    .collect(),
            },
        )
        .await
        .map_err(repository_error)?;
        row_to_domain(result.obligation)
    }
}

#[async_trait]
impl CleanupCloseoutPort for PgCleanupRepository {
    async fn closeout_counts(
        &self,
        operation_id: uuid::Uuid,
        organization_id_at_time: uuid::Uuid,
    ) -> Result<CleanupCloseoutCounts, CleanupError> {
        let row = golish_db::repo::organization_deletion_jobs::cleanup_closeout_gate(
            &self.pool,
            operation_id,
            organization_id_at_time,
        )
        .await
        .map_err(repository_error)?;
        Ok(CleanupCloseoutCounts {
            operation_id: row.operation_id,
            organization_id_at_time: row.organization_id_at_time,
            missing_obligation_count: row.missing_obligation_count,
            nonterminal_obligation_count: row.nonterminal_obligation_count,
            undisclosed_residual_count: row.undisclosed_residual_count,
            invalid_terminal_truth_count: row.invalid_terminal_truth_count,
            residual_obligation_ids: row.residual_obligation_ids,
        })
    }
}

#[async_trait]
impl OrganizationDeletionPort for PgCleanupRepository {
    async fn request_organization_deletion(
        &self,
        root_organization_id: uuid::Uuid,
        expected_project_path: &str,
    ) -> Result<OrganizationDeletionRequestResult, CleanupError> {
        let principal = golish_db::repo::operator_principals::current_local(&self.pool)
            .await
            .map_err(repository_error)?;
        let row = golish_db::repo::organization_deletion_jobs::request(
            &self.pool,
            &golish_db::repo::organization_deletion_jobs::RequestOrganizationDeletion {
                job_id: uuid::Uuid::new_v4(),
                root_organization_id,
                principal_id: principal.id,
                expected_project_path: expected_project_path.to_string(),
            },
        )
        .await
        .map_err(repository_error)?;
        Ok(OrganizationDeletionRequestResult {
            job_id: row.id,
            root_organization_id_at_time: row.root_organization_id_at_time,
        })
    }
}

pub fn row_to_domain(
    row: golish_db::repo::cleanup_obligations::CleanupObligationRow,
) -> Result<CleanupObligation, CleanupError> {
    let status = match row.status.as_str() {
        "open" => CleanupObligationStatus::Open,
        "in_progress" => CleanupObligationStatus::InProgress,
        "verified_absent" => CleanupObligationStatus::VerifiedAbsent,
        "blocked" => CleanupObligationStatus::Blocked,
        "waived_by_user" => CleanupObligationStatus::WaivedByUser,
        _ => {
            return Err(CleanupError::Repository(
                "unknown cleanup status".to_string(),
            ))
        }
    };
    let proof_requirements = serde_json::from_value(row.proof_requirements)
        .map_err(|error| CleanupError::Repository(error.to_string()))?;
    let residual_risk: Option<ResidualRisk> = row
        .residual_risk
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| CleanupError::Repository(error.to_string()))?;
    Ok(CleanupObligation {
        id: CleanupObligationId(row.id),
        operation_id: row.operation_id,
        project_scope_id: row.project_scope_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id_at_time: row.organization_id_at_time,
        source_action_id: ActionId(row.source_action_id),
        source_action_plan_hash: row.source_action_plan_hash,
        affected_resource_snapshot: row.affected_resource_snapshot,
        resource_identity_hash: row.resource_identity_hash,
        cleanup_strategy: row.cleanup_strategy,
        proof_requirements,
        deadline: row.deadline,
        status,
        residual_risk,
        row_version: row.row_version,
    })
}
