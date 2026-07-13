//! Trusted local IPC for the authoritative structured temporal graph.
//!
//! These commands are intentionally separate from the legacy `kg_*` surface.
//! The request never accepts an actor id or project path as authority: the
//! server resolves the opaque active local principal, then verifies the stable
//! project-scope id and exact organization binding in canonical DB state.

use chrono::{DateTime, Utc};
use golish_app_core::domain::operator::OperatorChannel;
use golish_db::repo::{knowledge_assertions, knowledge_graph};
use golish_graphiti::{
    GraphError, ScopedGraphQuery, TemporalGraphClient, TemporalGraphFact, TemporalGraphQueryResult,
    TemporalGraphRelationFact,
};
use golish_memory_app::{rebuild_graph_scope_from_assertions, GraphRebuildScope, MemoryError};
use golish_memory_domain::classification::AssertionVisibility;
use golish_memory_domain::scope::ProjectScopeId;
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;
use uuid::Uuid;

use crate::error::GolishError;
use crate::state::AgentState;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "scopeKind")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum KnowledgeGraphScopeRequest {
    #[serde(rename = "organization")]
    Organization {
        #[serde(rename = "projectScopeId")]
        project_scope_id: String,
        #[serde(rename = "organizationIdAtTime")]
        organization_id_at_time: String,
    },
    #[serde(rename = "global_sanitized")]
    GlobalSanitized,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphQueryRequest {
    pub scope: KnowledgeGraphScopeRequest,
    pub query: String,
    pub valid_at: Option<String>,
    #[ts(type = "number | null")]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphRebuildRequest {
    pub scope: KnowledgeGraphScopeRequest,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphLineageView {
    pub assertion_id: String,
    pub source_stream_key: String,
    #[ts(type = "number")]
    pub source_version: i64,
    #[ts(type = "Array<number>")]
    pub evidence_refs: Vec<i64>,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub fresh_until: Option<String>,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphEntityView {
    pub generation_id: String,
    pub entity_id: String,
    pub scope_key: String,
    pub project_scope_id: Option<String>,
    pub organization_id_at_time: Option<String>,
    pub canonical_ref: String,
    pub entity_type: String,
    pub display_name: String,
    pub properties: serde_json::Value,
    pub lineages: Vec<KnowledgeGraphLineageView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphRelationView {
    pub generation_id: String,
    pub relation_id: String,
    pub scope_key: String,
    pub project_scope_id: Option<String>,
    pub organization_id_at_time: Option<String>,
    pub from_entity_id: String,
    pub from_canonical_ref: String,
    pub from_entity_type: String,
    pub from_display_name: String,
    pub to_entity_id: String,
    pub to_canonical_ref: String,
    pub to_entity_type: String,
    pub to_display_name: String,
    pub relation_type: String,
    pub properties: serde_json::Value,
    pub lineages: Vec<KnowledgeGraphLineageView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphQueryResultView {
    pub entities: Vec<KnowledgeGraphEntityView>,
    pub relations: Vec<KnowledgeGraphRelationView>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct KnowledgeGraphGenerationView {
    pub generation_id: String,
    pub scope_key: String,
    #[ts(type = "number")]
    pub projection_schema_version: i32,
    pub status: String,
    pub build_hash: Option<String>,
    #[ts(type = "number | null")]
    pub entity_count: Option<i64>,
    #[ts(type = "number | null")]
    pub relation_count: Option<i64>,
}

#[derive(Clone, Debug)]
enum AuthorizedGraphScope {
    Organization {
        project_scope_id: ProjectScopeId,
        organization_id_at_time: Uuid,
    },
    GlobalSanitized,
}

impl AuthorizedGraphScope {
    fn rebuild_scope(&self) -> GraphRebuildScope {
        match self {
            Self::Organization {
                project_scope_id,
                organization_id_at_time,
            } => GraphRebuildScope::Organization {
                project_scope_id: *project_scope_id,
                organization_id_at_time: *organization_id_at_time,
            },
            Self::GlobalSanitized => GraphRebuildScope::GlobalSanitized,
        }
    }

    fn assertion_visibility(&self) -> AssertionVisibility {
        match self {
            Self::Organization {
                project_scope_id,
                organization_id_at_time,
            } => AssertionVisibility::OrganizationLongTerm {
                project_scope_id: *project_scope_id,
                organization_id_at_time: *organization_id_at_time,
            },
            Self::GlobalSanitized => AssertionVisibility::GlobalSanitized,
        }
    }
}

#[tauri::command]
pub async fn knowledge_graph_query_scoped(
    request: KnowledgeGraphQueryRequest,
    state: State<'_, AgentState>,
) -> Result<KnowledgeGraphQueryResultView, GolishError> {
    validate_query_text(&request.query)?;
    let scope = authorize_scope(&request.scope, &state).await?;
    let valid_at = parse_valid_at(request.valid_at.as_deref())?;
    let mut query = match scope {
        AuthorizedGraphScope::Organization {
            project_scope_id,
            organization_id_at_time,
        } => ScopedGraphQuery::for_organization(
            project_scope_id,
            organization_id_at_time,
            request.query,
            valid_at,
        ),
        AuthorizedGraphScope::GlobalSanitized => {
            ScopedGraphQuery::global_sanitized(request.query, valid_at)
        }
    };
    query.limit = request.limit.unwrap_or(100).clamp(1, 200);
    let result = TemporalGraphClient::new((*state.db_pool).clone())
        .query(query)
        .await
        .map_err(map_graph_error)?;
    Ok(result.into())
}

#[tauri::command]
pub async fn knowledge_graph_rebuild_scope(
    request: KnowledgeGraphRebuildRequest,
    state: State<'_, AgentState>,
) -> Result<KnowledgeGraphGenerationView, GolishError> {
    let scope = authorize_scope(&request.scope, &state).await?;
    let assertions = knowledge_assertions::list_active_for_visibility(
        &state.db_pool,
        &scope.assertion_visibility(),
    )
    .await
    .map_err(|error| GolishError::Internal(format!("{}: {error}", error.code())))?;
    let client = TemporalGraphClient::new((*state.db_pool).clone());
    let generation =
        rebuild_graph_scope_from_assertions(&client, &scope.rebuild_scope(), assertions)
            .await
            .map_err(map_memory_error)?;
    Ok(KnowledgeGraphGenerationView {
        generation_id: generation.generation_id.to_string(),
        scope_key: generation.scope_key.as_str().to_string(),
        projection_schema_version: generation.projection_schema_version,
        status: generation.status,
        build_hash: generation.build_hash,
        entity_count: generation.entity_count,
        relation_count: generation.relation_count,
    })
}

async fn authorize_scope(
    request: &KnowledgeGraphScopeRequest,
    state: &AgentState,
) -> Result<AuthorizedGraphScope, GolishError> {
    let principal = state
        .operator_principal_provider
        .current(OperatorChannel::LocalDesktop)
        .await?;
    if principal.channel() != OperatorChannel::LocalDesktop {
        return Err(GolishError::Validation(
            "knowledge_graph_local_operator_required".to_string(),
        ));
    }
    match request {
        KnowledgeGraphScopeRequest::Organization {
            project_scope_id,
            organization_id_at_time,
        } => {
            let project_scope_id = parse_uuid(project_scope_id, "projectScopeId")?;
            let organization_id_at_time =
                parse_uuid(organization_id_at_time, "organizationIdAtTime")?;
            let authorized = knowledge_graph::organization_scope_is_registered_and_bound(
                &state.db_pool,
                project_scope_id,
                organization_id_at_time,
            )
            .await
            .map_err(map_repository_error)?;
            if !authorized {
                return Err(GolishError::Validation(
                    "knowledge_graph_scope_not_authorized".to_string(),
                ));
            }
            Ok(AuthorizedGraphScope::Organization {
                project_scope_id: ProjectScopeId(project_scope_id),
                organization_id_at_time,
            })
        }
        KnowledgeGraphScopeRequest::GlobalSanitized => {
            let _server_owned_operator_id = principal.id();
            Ok(AuthorizedGraphScope::GlobalSanitized)
        }
    }
}

fn validate_query_text(query: &str) -> Result<(), GolishError> {
    if query.trim().is_empty() || query.len() > 512 || query.chars().any(char::is_control) {
        return Err(GolishError::Validation(
            "knowledge_graph_query_invalid".to_string(),
        ));
    }
    Ok(())
}

fn parse_valid_at(value: Option<&str>) -> Result<DateTime<Utc>, GolishError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| {
                    GolishError::Validation("knowledge_graph_valid_at_invalid".to_string())
                })
        })
        .transpose()
        .map(|value| value.unwrap_or_else(Utc::now))
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, GolishError> {
    Uuid::parse_str(value)
        .map_err(|_| GolishError::Validation(format!("knowledge_graph_{field}_invalid")))
}

fn map_graph_error(error: GraphError) -> GolishError {
    match error {
        GraphError::Database(error) => GolishError::Database(error),
        GraphError::TemporalRepository(knowledge_graph::KnowledgeGraphError::Sqlx(error)) => {
            GolishError::Database(error)
        }
        GraphError::InvalidArgument(message) => GolishError::Validation(message),
        other => GolishError::Internal(format!("{}: {other}", other.code())),
    }
}

fn map_repository_error(error: knowledge_graph::KnowledgeGraphError) -> GolishError {
    match error {
        knowledge_graph::KnowledgeGraphError::Sqlx(error) => GolishError::Database(error),
        other => GolishError::Internal(format!("{}: {other}", other.code())),
    }
}

fn map_memory_error(error: MemoryError) -> GolishError {
    match error {
        MemoryError::Policy(message) | MemoryError::GraphProjection(message) => {
            GolishError::Validation(message)
        }
        other => GolishError::Internal(format!("{}: {other}", other.code())),
    }
}

impl From<TemporalGraphQueryResult> for KnowledgeGraphQueryResultView {
    fn from(value: TemporalGraphQueryResult) -> Self {
        Self {
            entities: value.entities.into_iter().map(Into::into).collect(),
            relations: value.relations.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<TemporalGraphFact> for KnowledgeGraphEntityView {
    fn from(value: TemporalGraphFact) -> Self {
        Self {
            generation_id: value.generation_id.to_string(),
            entity_id: value.entity_id.to_string(),
            scope_key: value.scope_key.as_str().to_string(),
            project_scope_id: value.project_scope_id.map(|id| id.0.to_string()),
            organization_id_at_time: value.organization_id_at_time.map(|id| id.to_string()),
            canonical_ref: value.canonical_ref,
            entity_type: value.entity_type.as_str().to_string(),
            display_name: value.display_name,
            properties: value.properties,
            lineages: value.lineages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<TemporalGraphRelationFact> for KnowledgeGraphRelationView {
    fn from(value: TemporalGraphRelationFact) -> Self {
        Self {
            generation_id: value.generation_id.to_string(),
            relation_id: value.relation_id.to_string(),
            scope_key: value.scope_key.as_str().to_string(),
            project_scope_id: value.project_scope_id.map(|id| id.0.to_string()),
            organization_id_at_time: value.organization_id_at_time.map(|id| id.to_string()),
            from_entity_id: value.from_entity_id.to_string(),
            from_canonical_ref: value.from_canonical_ref,
            from_entity_type: value.from_entity_type.as_str().to_string(),
            from_display_name: value.from_display_name,
            to_entity_id: value.to_entity_id.to_string(),
            to_canonical_ref: value.to_canonical_ref,
            to_entity_type: value.to_entity_type.as_str().to_string(),
            to_display_name: value.to_display_name,
            relation_type: value.relation_type.as_str().to_string(),
            properties: value.properties,
            lineages: value.lineages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<golish_graphiti::TemporalLineageFact> for KnowledgeGraphLineageView {
    fn from(value: golish_graphiti::TemporalLineageFact) -> Self {
        Self {
            assertion_id: value.assertion_id.to_string(),
            source_stream_key: value.source_stream_key,
            source_version: value.source_version,
            evidence_refs: value.evidence_refs,
            valid_from: value.valid_from.to_rfc3339(),
            valid_to: value.valid_to.map(|value| value.to_rfc3339()),
            fresh_until: value.fresh_until.map(|value| value.to_rfc3339()),
            classification: value.classification.as_str().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_wire_has_no_actor_or_project_path_authority() {
        let serialized = serde_json::to_value(KnowledgeGraphScopeRequest::Organization {
            project_scope_id: Uuid::from_u128(1).to_string(),
            organization_id_at_time: Uuid::from_u128(2).to_string(),
        })
        .expect("serialize scope request");
        let object = serialized.as_object().expect("object scope request");
        assert!(!object.contains_key("actorId"));
        assert!(!object.contains_key("operatorId"));
        assert!(!object.contains_key("projectPath"));
    }

    #[test]
    fn partial_organization_scope_fails_deserialization() {
        let error = serde_json::from_value::<KnowledgeGraphScopeRequest>(serde_json::json!({
            "scopeKind": "organization",
            "projectScopeId": Uuid::from_u128(1).to_string()
        }))
        .expect_err("organization id is mandatory");
        assert!(error.to_string().contains("organizationIdAtTime"));
    }

    #[test]
    fn query_validation_is_bounded_and_control_free() {
        assert!(validate_query_text("host:10.0.0.5").is_ok());
        assert!(validate_query_text("").is_err());
        assert!(validate_query_text("host\nforged").is_err());
        assert!(validate_query_text(&"x".repeat(513)).is_err());
    }
}
