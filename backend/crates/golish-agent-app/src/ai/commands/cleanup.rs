//! Trusted Cleanup P7b command surface.
//!
//! Model-visible tools may suggest a waiver, but only this local IPC obtains
//! the opaque C0 principal and can commit one with exact row-version CAS.

use std::sync::Arc;

use golish_app_core::domain::operator::OperatorChannel;
use golish_cleanup_app::{
    CleanupCloseoutPort, CleanupKernel, CleanupObligationRecord, PgCleanupRepository,
};
use golish_cleanup_domain::{
    CleanupObligationId, ResidualRisk, TrustedOperatorPrincipal, WaiverRequest,
};
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::state::AgentState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCommandError {
    pub code: String,
    pub message: String,
}

impl CleanupCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct CleanupObligationListRequest {
    pub operation_id: String,
    pub organization_id_at_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct CleanupWaiverSubmitRequest {
    pub waiver_id: String,
    pub obligation_id: String,
    pub operation_id: String,
    pub project_scope_id: String,
    pub scope_snapshot_id: String,
    pub organization_id_at_time: String,
    #[ts(type = "number")]
    pub expected_row_version: i64,
    pub reason: String,
    pub residual_summary: String,
    pub residual_severity: String,
    #[ts(type = "Array<number>")]
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct CleanupObligationView {
    pub obligation_id: String,
    pub operation_id: String,
    pub project_scope_id: String,
    pub scope_snapshot_id: String,
    pub organization_id_at_time: String,
    pub source_action_id: String,
    pub status: String,
    pub deadline: String,
    #[ts(type = "unknown")]
    pub affected_resource_snapshot: serde_json::Value,
    #[ts(type = "unknown")]
    pub cleanup_strategy: serde_json::Value,
    #[ts(type = "unknown | null")]
    pub residual_risk: Option<serde_json::Value>,
    #[ts(type = "number")]
    pub row_version: i64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct CleanupCloseoutGateView {
    pub operation_id: String,
    pub organization_id_at_time: String,
    #[ts(type = "number")]
    pub missing_obligation_count: i64,
    #[ts(type = "number")]
    pub nonterminal_obligation_count: i64,
    #[ts(type = "number")]
    pub undisclosed_residual_count: i64,
    #[ts(type = "number")]
    pub invalid_terminal_truth_count: i64,
    pub allows_closeout: bool,
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, CleanupCommandError> {
    Uuid::parse_str(value).map_err(|_| {
        CleanupCommandError::new("cleanup_request_invalid", format!("invalid {field}"))
    })
}

async fn local_principal(
    state: &AgentState,
) -> Result<TrustedOperatorPrincipal, CleanupCommandError> {
    let principal = state
        .operator_principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(CleanupCommandError::new(
            "cleanup_operator_untrusted",
            "cleanup changes require the local desktop operator",
        ));
    }
    Ok(TrustedOperatorPrincipal::from_server_record(
        principal.id().as_uuid(),
    ))
}

fn view(row: CleanupObligationRecord) -> CleanupObligationView {
    CleanupObligationView {
        obligation_id: row.id.to_string(),
        operation_id: row.operation_id.to_string(),
        project_scope_id: row.project_scope_id.to_string(),
        scope_snapshot_id: row.scope_snapshot_id.to_string(),
        organization_id_at_time: row.organization_id_at_time.to_string(),
        source_action_id: row.source_action_id.to_string(),
        status: row.status,
        deadline: row.deadline.to_rfc3339(),
        affected_resource_snapshot: row.affected_resource_snapshot,
        cleanup_strategy: row.cleanup_strategy,
        residual_risk: row.residual_risk,
        row_version: row.row_version,
    }
}

#[tauri::command]
pub async fn cleanup_list_obligations(
    request: CleanupObligationListRequest,
    state: State<'_, AgentState>,
) -> Result<Vec<CleanupObligationView>, CleanupCommandError> {
    let _principal = local_principal(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operation_id")?;
    let organization_id = parse_uuid(&request.organization_id_at_time, "organization_id_at_time")?;
    let rows = PgCleanupRepository::new(state.db_pool.as_ref().clone())
        .list_obligations_for_operation(operation_id)
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    Ok(rows
        .into_iter()
        .filter(|row| row.organization_id_at_time == organization_id)
        .map(view)
        .collect())
}

#[tauri::command]
pub async fn cleanup_get_closeout_gate(
    request: CleanupObligationListRequest,
    state: State<'_, AgentState>,
) -> Result<CleanupCloseoutGateView, CleanupCommandError> {
    let _principal = local_principal(&state).await?;
    let operation_id = parse_uuid(&request.operation_id, "operation_id")?;
    let organization_id = parse_uuid(&request.organization_id_at_time, "organization_id_at_time")?;
    let gate = PgCleanupRepository::new(state.db_pool.as_ref().clone())
        .closeout_counts(operation_id, organization_id)
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    Ok(CleanupCloseoutGateView {
        operation_id: gate.operation_id.to_string(),
        organization_id_at_time: gate.organization_id_at_time.to_string(),
        missing_obligation_count: gate.missing_obligation_count,
        nonterminal_obligation_count: gate.nonterminal_obligation_count,
        undisclosed_residual_count: gate.undisclosed_residual_count,
        invalid_terminal_truth_count: gate.invalid_terminal_truth_count,
        allows_closeout: gate.allows_closeout(),
    })
}

#[tauri::command]
pub async fn cleanup_waive_obligation(
    request: CleanupWaiverSubmitRequest,
    state: State<'_, AgentState>,
) -> Result<CleanupObligationView, CleanupCommandError> {
    let principal = local_principal(&state).await?;
    let waiver_id = parse_uuid(&request.waiver_id, "waiver_id")?;
    let obligation_id = parse_uuid(&request.obligation_id, "obligation_id")?;
    let operation_id = parse_uuid(&request.operation_id, "operation_id")?;
    let project_scope_id = parse_uuid(&request.project_scope_id, "project_scope_id")?;
    let scope_snapshot_id = parse_uuid(&request.scope_snapshot_id, "scope_snapshot_id")?;
    let organization_id = parse_uuid(&request.organization_id_at_time, "organization_id_at_time")?;
    let repository = Arc::new(PgCleanupRepository::new(state.db_pool.as_ref().clone()));
    repository
        .load_exact_obligation(
            operation_id,
            project_scope_id,
            scope_snapshot_id,
            organization_id,
            obligation_id,
        )
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    let kernel = CleanupKernel::new(repository.clone());
    let obligation = kernel
        .waive_obligation(
            WaiverRequest {
                id: waiver_id,
                obligation_id: CleanupObligationId(obligation_id),
                operation_id,
                project_scope_id,
                scope_snapshot_id,
                organization_id_at_time: organization_id,
                expected_row_version: request.expected_row_version,
                reason: request.reason,
                residual_risk: ResidualRisk {
                    summary: request.residual_summary,
                    severity: request.residual_severity,
                },
                evidence_ids: request.evidence_ids,
            },
            &principal,
        )
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    let row = repository
        .load_exact_obligation(
            operation_id,
            project_scope_id,
            scope_snapshot_id,
            organization_id,
            obligation.id.0,
        )
        .await
        .map_err(|error| CleanupCommandError::new(error.code(), error.to_string()))?;
    Ok(view(row))
}

#[cfg(test)]
mod tests {
    #[test]
    fn waiver_request_has_no_actor_identity() {
        let source = include_str!("cleanup.rs");
        let request = source
            .split("pub struct CleanupWaiverSubmitRequest")
            .nth(1)
            .expect("waiver request")
            .split('}')
            .next()
            .expect("waiver fields");
        assert!(!request.contains("actor_id"));
        assert!(!request.contains("principal_id"));
        assert!(!request.contains("decided_by"));
        for exact_scope_field in [
            "operation_id",
            "project_scope_id",
            "scope_snapshot_id",
            "organization_id_at_time",
            "expected_row_version",
        ] {
            assert!(request.contains(exact_scope_field));
        }
        assert!(source.contains("OperatorChannel::LocalDesktop"));
    }
}
