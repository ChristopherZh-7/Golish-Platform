//! Trusted, DB-authoritative Reporting read model and explicit publication IPC.

use std::collections::BTreeMap;
use std::path::Path;

use golish_app_core::domain::operator::{
    OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider,
};
use golish_reporting_app::{ExplicitFinalizeRequest, ReportFinalizer, ReportFormat};
use golish_reporting_domain::{PublicationStatus, ReportReadModel, ValidationStatus};
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::ai::db_bridge::reporting::{
    load_report_bundle, PgReportPublicationPort, ReportingProjectAuthority,
    StoredReportArtifactView, StoredReportBundle,
};
use crate::ai::db_bridge::reporting_gate::build_or_reuse_validated_report_with_project_authority;
use crate::state::AgentState;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportingCommandError {
    pub code: String,
    pub message: String,
}

impl ReportingCommandError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

const REPORT_FORBIDDEN: &str = "REPORT_FORBIDDEN";
const REPORT_FORBIDDEN_MESSAGE: &str = "reporting scope is not authorized";

fn report_forbidden() -> ReportingCommandError {
    ReportingCommandError::new(REPORT_FORBIDDEN, REPORT_FORBIDDEN_MESSAGE)
}

/// Server-derived authority for one Reporting IPC request.
///
/// This value is intentionally not serializable or caller-constructible. The
/// public seam exists so the command-boundary contract can be exercised without
/// manufacturing a Tauri `State`; production commands remain its consumers.
#[derive(Debug)]
pub struct AuthorizedReportingScope {
    operation_id: Uuid,
    project_scope_id: Uuid,
    scope_snapshot_id: Uuid,
    scope_snapshot_hash: String,
    canonical_project_path: String,
    project_path_sha256: String,
    project_row_version: i64,
    principal: TrustedOperatorPrincipal,
}

impl AuthorizedReportingScope {
    fn project_authority(&self) -> ReportingProjectAuthority {
        ReportingProjectAuthority::new(
            self.project_scope_id,
            self.scope_snapshot_id,
            self.scope_snapshot_hash.clone(),
            self.canonical_project_path.clone(),
            self.project_path_sha256.clone(),
            self.project_row_version,
        )
    }
}

/// Authorize one operation before any Reporting read, existence branch, build,
/// artifact lookup, or publication attempt.
///
/// Every rejected state deliberately collapses to `REPORT_FORBIDDEN`: caller
/// trust failure, unknown/foreign operation, missing project binding, retired
/// project, unsealed scope, and any operation/project/snapshot mismatch must not
/// become an existence oracle at the IPC boundary.
pub async fn authorize_reporting_scope(
    pool: &sqlx::PgPool,
    principal_provider: &dyn TrustedOperatorPrincipalProvider,
    operation_id: Uuid,
) -> Result<AuthorizedReportingScope, ReportingCommandError> {
    let principal = principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await
        .map_err(|_| report_forbidden())?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(report_forbidden());
    }

    let operation = golish_db::repo::operation_state::get(pool, operation_id)
        .await
        .map_err(|_| report_forbidden())?
        .ok_or_else(report_forbidden)?;
    let project_scope_id = operation.project_scope_id.ok_or_else(report_forbidden)?;
    let project = golish_db::repo::project_scopes::get_active_for_share(pool, project_scope_id)
        .await
        .map_err(|_| report_forbidden())?
        .ok_or_else(report_forbidden)?;
    let scope = golish_db::repo::operation_org_scope::load_for_operation(pool, operation_id)
        .await
        .map_err(|_| report_forbidden())?
        .ok_or_else(report_forbidden)?;
    let snapshot = &scope.snapshot;
    let has_exact_root = scope.units.iter().any(|unit| {
        unit.snapshot_id == snapshot.id
            && unit.organization_id == snapshot.root_organization_id
            && unit.role == "root"
    });
    if snapshot.operation_id != operation_id
        || snapshot.project_scope_id != project_scope_id
        || snapshot.sealed_at.is_none()
        || !has_exact_root
    {
        return Err(report_forbidden());
    }

    Ok(AuthorizedReportingScope {
        operation_id,
        project_scope_id,
        scope_snapshot_id: snapshot.id,
        scope_snapshot_hash: snapshot.scope_hash.clone(),
        canonical_project_path: project.canonical_project_path,
        project_path_sha256: project.path_sha256,
        project_row_version: project.row_version,
        principal,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportingScopeRequest {
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportingFinalizeRequest {
    pub operation_id: String,
    pub revision_id: String,
    pub expected_source_hash: String,
    #[ts(type = "number")]
    pub expected_revision_version: i64,
    pub confirm_final_publish: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportRevisionView {
    pub revision_id: String,
    #[ts(type = "number")]
    pub revision_number: i32,
    #[ts(type = "number")]
    pub row_version: i64,
    pub source_set_hash: String,
    pub validation_status: String,
    pub publication_status: String,
    pub supersedes_revision_id: Option<String>,
    pub validated_at: Option<String>,
    pub finalized_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportCitationView {
    pub citation_id: String,
    pub source_kind: String,
    pub source_id_kind: String,
    pub source_id_value: String,
    #[ts(type = "number")]
    pub source_row_version: i64,
    pub source_hash: String,
    #[ts(type = "number")]
    pub evidence_audit_id: i64,
    pub organization_id_at_time: String,
    pub display_label: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportClaimView {
    pub claim_id: String,
    pub claim_kind: String,
    pub subject_ref: String,
    pub predicate: String,
    #[ts(type = "unknown")]
    pub value: serde_json::Value,
    #[ts(type = "number")]
    pub ordinal: i32,
    pub citations: Vec<ReportCitationView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportSectionView {
    pub section_id: String,
    pub organization_id_at_time: Option<String>,
    pub organization_name_at_snapshot: Option<String>,
    pub section_kind: String,
    #[ts(type = "number")]
    pub ordinal: i32,
    pub rendered_content: Option<String>,
    pub claims: Vec<ReportClaimView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportArtifactView {
    pub revision_id: String,
    pub artifact_kind: String,
    pub content_key: String,
    pub sha256: String,
    #[ts(type = "number")]
    pub byte_len: i64,
    #[ts(type = "number")]
    pub redaction_version: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct ReportReadModelView {
    pub report_id: String,
    pub operation_id: String,
    pub project_scope_id: String,
    pub scope_snapshot_id: String,
    pub scope_snapshot_hash: String,
    pub current: Option<ReportRevisionView>,
    pub revisions: Vec<ReportRevisionView>,
    pub sections: Vec<ReportSectionView>,
    pub artifacts: Vec<ReportArtifactView>,
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, ReportingCommandError> {
    Uuid::parse_str(value).map_err(|_| {
        ReportingCommandError::new("REPORT_REQUEST_INVALID", format!("invalid {field}"))
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn revision_view(row: &golish_db::repo::report_revisions::ReportRevisionRow) -> ReportRevisionView {
    ReportRevisionView {
        revision_id: row.revision_id.to_string(),
        revision_number: row.revision_number,
        row_version: row.row_version,
        source_set_hash: row.source_set_hash.clone(),
        validation_status: row.validation_status.clone(),
        publication_status: row.publication_status.clone(),
        supersedes_revision_id: row.supersedes_revision_id.map(|id| id.to_string()),
        validated_at: row.validated_at.map(|value| value.to_rfc3339()),
        finalized_at: row.finalized_at.map(|value| value.to_rfc3339()),
    }
}

fn artifact_view(row: &StoredReportArtifactView) -> ReportArtifactView {
    ReportArtifactView {
        revision_id: row.revision_id.to_string(),
        artifact_kind: row.artifact_kind.clone(),
        content_key: row.content_key.clone(),
        sha256: row.sha256.clone(),
        byte_len: row.byte_len,
        redaction_version: row.redaction_version,
    }
}

fn bundle_view(bundle: &StoredReportBundle) -> ReportReadModelView {
    let mut citations_by_claim = BTreeMap::<Uuid, Vec<ReportCitationView>>::new();
    for citation in &bundle.citations {
        citations_by_claim
            .entry(citation.claim_id)
            .or_default()
            .push(ReportCitationView {
                citation_id: citation.citation_id.to_string(),
                source_kind: citation.source_kind.clone(),
                source_id_kind: citation.source_id_kind.clone(),
                source_id_value: citation.source_id_value.clone(),
                source_row_version: citation.source_row_version,
                source_hash: hex(&citation.source_hash),
                evidence_audit_id: citation.evidence_audit_id,
                organization_id_at_time: citation.organization_id_at_time.to_string(),
                display_label: citation.display_label.clone(),
            });
    }
    let mut claims_by_section = BTreeMap::<Uuid, Vec<ReportClaimView>>::new();
    for claim in &bundle.claims {
        claims_by_section
            .entry(claim.section_id)
            .or_default()
            .push(ReportClaimView {
                claim_id: claim.claim_id.to_string(),
                claim_kind: claim.claim_kind.clone(),
                subject_ref: claim.subject_ref.clone(),
                predicate: claim.predicate.clone(),
                value: claim.object_value.clone(),
                ordinal: claim.ordinal,
                citations: citations_by_claim
                    .remove(&claim.claim_id)
                    .unwrap_or_default(),
            });
    }
    let sections = bundle
        .sections
        .iter()
        .map(|section| ReportSectionView {
            section_id: section.section_id.to_string(),
            organization_id_at_time: section.organization_id_at_time.map(|id| id.to_string()),
            organization_name_at_snapshot: section.organization_name_at_snapshot.clone(),
            section_kind: section.section_kind.clone(),
            ordinal: section.ordinal,
            rendered_content: section.rendered_content.clone(),
            claims: claims_by_section
                .remove(&section.section_id)
                .unwrap_or_default(),
        })
        .collect();
    ReportReadModelView {
        report_id: bundle.report.report_id.to_string(),
        operation_id: bundle.report.operation_id.to_string(),
        project_scope_id: bundle.report.project_scope_id.to_string(),
        scope_snapshot_id: bundle.report.scope_snapshot_id.to_string(),
        scope_snapshot_hash: bundle.report.scope_snapshot_hash.clone(),
        current: bundle.current_revision.as_ref().map(revision_view),
        revisions: bundle.revisions.iter().map(revision_view).collect(),
        sections,
        artifacts: bundle.artifacts.iter().map(artifact_view).collect(),
    }
}

async fn load_bundle(
    state: &AgentState,
    operation_id: Uuid,
) -> Result<Option<StoredReportBundle>, ReportingCommandError> {
    load_report_bundle(&state.db_pool, operation_id)
        .await
        .map_err(|error| ReportingCommandError::new("REPORT_DATABASE", error.to_string()))
}

async fn load_authorized_bundle(
    state: &AgentState,
    authority: &AuthorizedReportingScope,
) -> Result<Option<StoredReportBundle>, ReportingCommandError> {
    let bundle = load_bundle(state, authority.operation_id).await?;
    if bundle.as_ref().is_some_and(|bundle| {
        bundle.report.operation_id != authority.operation_id
            || bundle.report.project_scope_id != authority.project_scope_id
            || bundle.report.scope_snapshot_id != authority.scope_snapshot_id
            || bundle.report.scope_snapshot_hash != authority.scope_snapshot_hash
    }) {
        return Err(report_forbidden());
    }
    Ok(bundle)
}

#[tauri::command]
pub async fn reporting_get_read_model(
    request: ReportingScopeRequest,
    state: State<'_, AgentState>,
) -> Result<Option<ReportReadModelView>, ReportingCommandError> {
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let authority = authorize_reporting_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        operation_id,
    )
    .await?;
    Ok(load_authorized_bundle(&state, &authority)
        .await?
        .as_ref()
        .map(bundle_view))
}

#[tauri::command]
pub async fn reporting_build_read_model(
    request: ReportingScopeRequest,
    state: State<'_, AgentState>,
) -> Result<ReportReadModelView, ReportingCommandError> {
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let authority = authorize_reporting_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        operation_id,
    )
    .await?;
    build_or_reuse_validated_report_with_project_authority(
        state.db_pool.clone(),
        operation_id,
        authority.project_authority(),
    )
    .await
    .map_err(|error| ReportingCommandError::new("REPORT_BUILD_FAILED", error.to_string()))?;
    load_authorized_bundle(&state, &authority)
        .await?
        .as_ref()
        .map(bundle_view)
        .ok_or_else(|| ReportingCommandError::new("REPORT_NOT_FOUND", "report disappeared"))
}

#[tauri::command]
pub async fn reporting_list_revisions(
    request: ReportingScopeRequest,
    state: State<'_, AgentState>,
) -> Result<Vec<ReportRevisionView>, ReportingCommandError> {
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let authority = authorize_reporting_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        operation_id,
    )
    .await?;
    Ok(load_authorized_bundle(&state, &authority)
        .await?
        .map(|bundle| bundle.revisions.iter().map(revision_view).collect())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn reporting_get_artifacts(
    request: ReportingScopeRequest,
    state: State<'_, AgentState>,
) -> Result<Vec<ReportArtifactView>, ReportingCommandError> {
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let authority = authorize_reporting_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        operation_id,
    )
    .await?;
    Ok(load_authorized_bundle(&state, &authority)
        .await?
        .map(|bundle| bundle.artifacts.iter().map(artifact_view).collect())
        .unwrap_or_default())
}

fn deterministic_markdown(view: &ReportReadModelView) -> Vec<u8> {
    let mut output = format!("# Golish Report\n\nOperation: `{}`\n\n", view.operation_id);
    for section in &view.sections {
        output.push_str(&format!("## {}\n\n", section.section_kind));
        for claim in &section.claims {
            output.push_str(&format!(
                "- **{}** {} `{}`\n",
                claim.subject_ref, claim.predicate, claim.value
            ));
            for citation in &claim.citations {
                output.push_str(&format!(
                    "  - Evidence {} · {}:{}@v{}\n",
                    citation.evidence_audit_id,
                    citation.source_kind,
                    citation.source_id_value,
                    citation.source_row_version
                ));
            }
        }
        output.push('\n');
    }
    output.into_bytes()
}

#[tauri::command]
pub async fn reporting_finalize_revision(
    request: ReportingFinalizeRequest,
    state: State<'_, AgentState>,
) -> Result<ReportReadModelView, ReportingCommandError> {
    let operation_id = parse_uuid(&request.operation_id, "operationId")?;
    let authority = authorize_reporting_scope(
        state.db_pool.as_ref(),
        state.operator_principal_provider.as_ref(),
        operation_id,
    )
    .await?;
    if !request.confirm_final_publish {
        return Err(ReportingCommandError::new(
            "REPORT_FINALIZE_CONFIRMATION_REQUIRED",
            "explicit final publication confirmation is required",
        ));
    }
    let revision_id = parse_uuid(&request.revision_id, "revisionId")?;
    let bundle = load_authorized_bundle(&state, &authority)
        .await?
        .ok_or_else(|| ReportingCommandError::new("REPORT_NOT_FOUND", "report not found"))?;
    let revision = bundle.current_revision.as_ref().ok_or_else(|| {
        ReportingCommandError::new("REPORT_REVISION_NOT_CURRENT", "current revision missing")
    })?;
    if revision.revision_id != revision_id {
        return Err(ReportingCommandError::new(
            "REPORT_REVISION_NOT_CURRENT",
            "revision is not current",
        ));
    }
    if revision.row_version != request.expected_revision_version
        || revision.source_set_hash != request.expected_source_hash
    {
        return Err(ReportingCommandError::new(
            "REPORT_SOURCE_CHANGED",
            "revision version or source hash changed",
        ));
    }
    if revision.validation_status != "validated" || revision.publication_status != "unpublished" {
        return Err(ReportingCommandError::new(
            "REPORT_REVISION_NOT_VALIDATED",
            "only the current validated unpublished revision can be finalized",
        ));
    }
    let snapshot = bundle.source_snapshot.clone().ok_or_else(|| {
        ReportingCommandError::new("REPORT_SOURCE_CHANGED", "source manifest missing")
    })?;
    if hex(&snapshot.source_set_hash) != request.expected_source_hash {
        return Err(ReportingCommandError::new(
            "REPORT_SOURCE_CHANGED",
            "manifest hash does not match revision",
        ));
    }
    let view = bundle_view(&bundle);
    let json = serde_json::to_vec_pretty(&view)
        .map_err(|error| ReportingCommandError::new("REPORT_RENDER_FAILED", error.to_string()))?;
    let markdown = deterministic_markdown(&view);
    let model = ReportReadModel {
        report_id: bundle.report.report_id,
        revision_id,
        operation_id,
        project_scope_id: bundle.report.project_scope_id,
        scope_snapshot_id: bundle.report.scope_snapshot_id,
        scope_snapshot_hash: bundle.report.scope_snapshot_hash.clone(),
        source_snapshot: snapshot,
        organization_sections: Vec::new(),
        findings: Vec::new(),
        cleanup_residuals: Vec::new(),
        citations: Vec::new(),
    };
    let store = state.reporting_artifact_store_factory.for_project(
        authority.project_scope_id,
        Path::new(&authority.canonical_project_path),
    );
    let finalizer = ReportFinalizer::new(
        store,
        PgReportPublicationPort::with_project_authority(
            state.db_pool.clone(),
            authority.project_authority(),
        ),
    );
    finalizer
        .finalize(
            &model,
            ExplicitFinalizeRequest {
                principal_id: authority.principal.id().as_uuid(),
                confirm_final_publish: true,
                expected_row_version: request.expected_revision_version,
                validation_status: ValidationStatus::Validated,
                publication_status: PublicationStatus::Unpublished,
            },
            vec![
                (ReportFormat::Markdown, markdown),
                (ReportFormat::Json, json),
            ],
        )
        .await
        .map_err(|error| ReportingCommandError::new(error.code(), error.to_string()))?;
    load_authorized_bundle(&state, &authority)
        .await?
        .as_ref()
        .map(bundle_view)
        .ok_or_else(|| ReportingCommandError::new("REPORT_NOT_FOUND", "report disappeared"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_request_has_no_actor_project_path_or_storage_authority() {
        let request = ReportingFinalizeRequest {
            operation_id: Uuid::new_v4().to_string(),
            revision_id: Uuid::new_v4().to_string(),
            expected_source_hash: "a".repeat(64),
            expected_revision_version: 3,
            confirm_final_publish: true,
        };
        let value = serde_json::to_value(request).expect("serialize request");
        for forbidden in [
            "actorId",
            "principalId",
            "projectScopeId",
            "projectPath",
            "storagePath",
            "contentKey",
        ] {
            assert!(
                value.get(forbidden).is_none(),
                "forbidden field {forbidden}"
            );
        }
    }
}
