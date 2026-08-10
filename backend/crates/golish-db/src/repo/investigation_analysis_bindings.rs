//! Exact authority bridge between unified Investigation analysis work and the
//! Candidate Registry snapshot/ordinal-zero attempt it owns.
//!
//! This repository does not schedule Controller/Analyst/Critic roles.  It only
//! persists and replays the immutable binding needed by a future unified
//! Investigation analysis host.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::unified_investigation_runtime::InvestigationUnitIdentity;

#[derive(Debug, thiserror::Error)]
pub enum InvestigationAnalysisBindingStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid Investigation analysis binding input: {0}")]
    InvalidInput(&'static str),
    #[error("Investigation analysis binding identity conflict: {0}")]
    IdentityConflict(&'static str),
}

pub type InvestigationAnalysisBindingStoreResult<T> =
    Result<T, InvestigationAnalysisBindingStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindInvestigationAnalysisAttemptInput {
    pub binding_id: Uuid,
    pub stable_request_id: Uuid,
    pub identity: InvestigationUnitIdentity,
    pub work_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationAnalysisAttemptBindingRow {
    pub binding_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub work_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub attempt_ordinal: i32,
    pub contract_version: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindInvestigationAnalysisAttemptOutcome {
    pub binding: InvestigationAnalysisAttemptBindingRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CandidateOrdinalZeroAnalysisAttemptRow {
    pub analysis_attempt_id: Uuid,
    pub snapshot_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub attempt_input_hash: String,
}

#[derive(Clone)]
pub struct PgInvestigationAnalysisBindingRepository {
    pool: Arc<PgPool>,
}

impl PgInvestigationAnalysisBindingRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn bind(
        &self,
        input: &BindInvestigationAnalysisAttemptInput,
    ) -> InvestigationAnalysisBindingStoreResult<BindInvestigationAnalysisAttemptOutcome> {
        validate_input(input)?;
        if let Some(binding) = self
            .load_by_stable_request_unscoped(input.stable_request_id)
            .await?
        {
            validate_exact_replay(&binding, input)?;
            return Ok(BindInvestigationAnalysisAttemptOutcome {
                binding,
                replayed: true,
            });
        }
        let inserted = sqlx::query_as::<_, InvestigationAnalysisAttemptBindingRow>(&format!(
            r#"INSERT INTO investigation_analysis_attempt_bindings(
                   binding_id,stable_request_id,authority_id,operation_id,
                   stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,work_id,candidate_snapshot_id,
                   analysis_attempt_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
               ON CONFLICT(stable_request_id) DO NOTHING
               RETURNING {BINDING_COLUMNS}"#
        ))
        .bind(input.binding_id)
        .bind(input.stable_request_id)
        .bind(input.identity.stage.authority_id)
        .bind(input.identity.stage.operation_id)
        .bind(input.identity.stage.stage_execution_id)
        .bind(&input.identity.stage.owning_stage_run_request_id)
        .bind(input.identity.stage_run_unit_id)
        .bind(input.identity.stage.scope_snapshot_id)
        .bind(input.identity.organization_id)
        .bind(input.work_id)
        .bind(input.candidate_snapshot_id)
        .bind(input.analysis_attempt_id)
        .fetch_optional(&*self.pool)
        .await?;

        if let Some(binding) = inserted {
            return Ok(BindInvestigationAnalysisAttemptOutcome {
                binding,
                replayed: false,
            });
        }

        let binding = self
            .load_by_stable_request_unscoped(input.stable_request_id)
            .await?
            .ok_or(InvestigationAnalysisBindingStoreError::IdentityConflict(
                "stable_request_replay_missing",
            ))?;
        validate_exact_replay(&binding, input)?;
        Ok(BindInvestigationAnalysisAttemptOutcome {
            binding,
            replayed: true,
        })
    }

    pub async fn load_ordinal_zero_attempt(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        candidate_snapshot_id: Uuid,
    ) -> InvestigationAnalysisBindingStoreResult<Option<CandidateOrdinalZeroAnalysisAttemptRow>>
    {
        for (id, field) in [
            (operation_id, "operation_id"),
            (organization_id, "organization_id"),
            (candidate_snapshot_id, "candidate_snapshot_id"),
        ] {
            validate_id(id, field)?;
        }
        Ok(sqlx::query_as::<_, CandidateOrdinalZeroAnalysisAttemptRow>(
            r#"SELECT analysis_attempt_id,snapshot_id,operation_id,organization_id,
                      attempt_input_hash
                 FROM candidate_analysis_attempts
                WHERE snapshot_id=$1 AND operation_id=$2 AND organization_id=$3
                  AND attempt_ordinal=0"#,
        )
        .bind(candidate_snapshot_id)
        .bind(operation_id)
        .bind(organization_id)
        .fetch_optional(&*self.pool)
        .await?)
    }

    pub async fn load(
        &self,
        identity: &InvestigationUnitIdentity,
        work_id: Uuid,
    ) -> InvestigationAnalysisBindingStoreResult<Option<InvestigationAnalysisAttemptBindingRow>>
    {
        validate_identity(identity)?;
        validate_id(work_id, "work_id")?;
        Ok(
            sqlx::query_as::<_, InvestigationAnalysisAttemptBindingRow>(&format!(
                r#"SELECT {BINDING_COLUMNS}
                 FROM investigation_analysis_attempt_bindings
                WHERE authority_id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND owning_stage_run_request_id=$4 AND stage_run_unit_id=$5
                  AND scope_snapshot_id=$6 AND organization_id=$7 AND work_id=$8"#
            ))
            .bind(identity.stage.authority_id)
            .bind(identity.stage.operation_id)
            .bind(identity.stage.stage_execution_id)
            .bind(&identity.stage.owning_stage_run_request_id)
            .bind(identity.stage_run_unit_id)
            .bind(identity.stage.scope_snapshot_id)
            .bind(identity.organization_id)
            .bind(work_id)
            .fetch_optional(&*self.pool)
            .await?,
        )
    }

    pub async fn load_by_stable_request(
        &self,
        identity: &InvestigationUnitIdentity,
        stable_request_id: Uuid,
    ) -> InvestigationAnalysisBindingStoreResult<Option<InvestigationAnalysisAttemptBindingRow>>
    {
        validate_identity(identity)?;
        validate_id(stable_request_id, "stable_request_id")?;
        Ok(
            sqlx::query_as::<_, InvestigationAnalysisAttemptBindingRow>(&format!(
                r#"SELECT {BINDING_COLUMNS}
                 FROM investigation_analysis_attempt_bindings
                WHERE stable_request_id=$1 AND authority_id=$2 AND operation_id=$3
                  AND stage_execution_id=$4 AND owning_stage_run_request_id=$5
                  AND stage_run_unit_id=$6 AND scope_snapshot_id=$7
                  AND organization_id=$8"#
            ))
            .bind(stable_request_id)
            .bind(identity.stage.authority_id)
            .bind(identity.stage.operation_id)
            .bind(identity.stage.stage_execution_id)
            .bind(&identity.stage.owning_stage_run_request_id)
            .bind(identity.stage_run_unit_id)
            .bind(identity.stage.scope_snapshot_id)
            .bind(identity.organization_id)
            .fetch_optional(&*self.pool)
            .await?,
        )
    }

    async fn load_by_stable_request_unscoped(
        &self,
        stable_request_id: Uuid,
    ) -> InvestigationAnalysisBindingStoreResult<Option<InvestigationAnalysisAttemptBindingRow>>
    {
        Ok(
            sqlx::query_as::<_, InvestigationAnalysisAttemptBindingRow>(&format!(
                r#"SELECT {BINDING_COLUMNS}
                 FROM investigation_analysis_attempt_bindings
                WHERE stable_request_id=$1"#
            ))
            .bind(stable_request_id)
            .fetch_optional(&*self.pool)
            .await?,
        )
    }
}

const BINDING_COLUMNS: &str = r#"binding_id,stable_request_id,authority_id,operation_id,
    stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
    organization_id,stage_kind,work_id,candidate_snapshot_id,analysis_attempt_id,
    attempt_ordinal,contract_version,created_at"#;

fn validate_input(
    input: &BindInvestigationAnalysisAttemptInput,
) -> InvestigationAnalysisBindingStoreResult<()> {
    validate_identity(&input.identity)?;
    for (id, field) in [
        (input.binding_id, "binding_id"),
        (input.stable_request_id, "stable_request_id"),
        (input.work_id, "work_id"),
        (input.candidate_snapshot_id, "candidate_snapshot_id"),
        (input.analysis_attempt_id, "analysis_attempt_id"),
    ] {
        validate_id(id, field)?;
    }
    Ok(())
}

fn validate_identity(
    identity: &InvestigationUnitIdentity,
) -> InvestigationAnalysisBindingStoreResult<()> {
    for (id, field) in [
        (identity.stage.authority_id, "authority_id"),
        (identity.stage.operation_id, "operation_id"),
        (identity.stage.stage_execution_id, "stage_execution_id"),
        (identity.stage.scope_snapshot_id, "scope_snapshot_id"),
        (identity.stage_run_unit_id, "stage_run_unit_id"),
        (identity.organization_id, "organization_id"),
    ] {
        validate_id(id, field)?;
    }
    let request = identity.stage.owning_stage_run_request_id.trim();
    if request.is_empty() || request.len() > 512 {
        return Err(InvestigationAnalysisBindingStoreError::InvalidInput(
            "owning_stage_run_request_id",
        ));
    }
    Ok(())
}

fn validate_id(id: Uuid, field: &'static str) -> InvestigationAnalysisBindingStoreResult<()> {
    if id.is_nil() {
        return Err(InvestigationAnalysisBindingStoreError::InvalidInput(field));
    }
    Ok(())
}

fn validate_exact_replay(
    row: &InvestigationAnalysisAttemptBindingRow,
    input: &BindInvestigationAnalysisAttemptInput,
) -> InvestigationAnalysisBindingStoreResult<()> {
    if row.binding_id != input.binding_id
        || row.authority_id != input.identity.stage.authority_id
        || row.operation_id != input.identity.stage.operation_id
        || row.stage_execution_id != input.identity.stage.stage_execution_id
        || row.owning_stage_run_request_id != input.identity.stage.owning_stage_run_request_id
        || row.stage_run_unit_id != input.identity.stage_run_unit_id
        || row.scope_snapshot_id != input.identity.stage.scope_snapshot_id
        || row.organization_id != input.identity.organization_id
        || row.stage_kind != "investigation"
        || row.work_id != input.work_id
        || row.candidate_snapshot_id != input.candidate_snapshot_id
        || row.analysis_attempt_id != input.analysis_attempt_id
        || row.attempt_ordinal != 0
        || row.contract_version != "unified_investigation_analysis_binding.v1"
    {
        return Err(InvestigationAnalysisBindingStoreError::IdentityConflict(
            "stable_request_replay_mismatch",
        ));
    }
    Ok(())
}
