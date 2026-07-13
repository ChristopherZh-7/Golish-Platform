use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scope::ProjectScopeId;
use crate::source_ref::SourceRef;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectorId {
    AssertionPromoterV1,
    DocumentProjectorV1,
    EmbeddingProjectorV1,
    GraphProjectorV1,
    ReportArtifactIndexerV1,
}

impl ProjectorId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AssertionPromoterV1 => "assertion-promoter@1",
            Self::DocumentProjectorV1 => "document-projector@1",
            Self::EmbeddingProjectorV1 => "embedding-projector@1",
            Self::GraphProjectorV1 => "graph-projector@1",
            Self::ReportArtifactIndexerV1 => "report-artifact-indexer@1",
        }
    }

    pub const fn key(self) -> &'static str {
        self.as_str()
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::AssertionPromoterV1 => "assertion-promoter",
            Self::DocumentProjectorV1 => "document-projector",
            Self::EmbeddingProjectorV1 => "embedding-projector",
            Self::GraphProjectorV1 => "graph-projector",
            Self::ReportArtifactIndexerV1 => "report-artifact-indexer",
        }
    }

    pub const fn schema_version(self) -> i32 {
        1
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectorRoute {
    pub projector: ProjectorId,
    pub depends_on: Option<ProjectorId>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEventNameV1 {
    StageEpisodeClosed,
    CandidateAttemptTerminal,
    FactDeltaAccepted,
    PostExploitActionPrepared,
    PostExploitFactTerminal,
    CleanupObligationTerminal,
    SourceScopeInvalidated,
    ReportRevisionFinalized,
}

impl KnowledgeEventNameV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StageEpisodeClosed => "StageEpisodeClosed.v1",
            Self::CandidateAttemptTerminal => "CandidateAttemptTerminal.v1",
            Self::FactDeltaAccepted => "FactDeltaAccepted.v1",
            Self::PostExploitActionPrepared => "PostExploitActionPrepared.v1",
            Self::PostExploitFactTerminal => "PostExploitFactTerminal.v1",
            Self::CleanupObligationTerminal => "CleanupObligationTerminal.v1",
            Self::SourceScopeInvalidated => "SourceScopeInvalidated.v1",
            Self::ReportRevisionFinalized => "ReportRevisionFinalized.v1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeEventPayloadV1 {
    pub source: SourceRef,
    pub source_stream_key: String,
    pub source_version: i64,
    pub structured_payload: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeEventEnvelopeV1 {
    pub event_id: Uuid,
    pub project_scope_id: Option<ProjectScopeId>,
    pub organization_id_at_time: Option<Uuid>,
    pub source_operation_id: Uuid,
    pub event_name: KnowledgeEventNameV1,
    pub schema_version: i32,
    pub payload: KnowledgeEventPayloadV1,
    pub occurred_at: DateTime<Utc>,
}

impl KnowledgeEventEnvelopeV1 {
    pub fn validate(&self) -> Result<(), EventCatalogError> {
        if self.schema_version != 1 {
            return Err(EventCatalogError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        self.payload
            .source
            .validate()
            .map_err(|_| EventCatalogError::InvalidSource)?;
        if self.payload.source_stream_key != self.payload.source.source_stream_key
            || self.payload.source_version != self.payload.source.version
        {
            return Err(EventCatalogError::SourceEnvelopeMismatch);
        }
        Ok(())
    }

    pub fn dedupe_key(&self) -> Result<String, EventCatalogError> {
        self.validate()?;
        let stored =
            crate::source_ref::StoredCanonicalRowId::from_domain(&self.payload.source.row_id)
                .map_err(|_| EventCatalogError::InvalidSource)?;
        Ok(format!(
            "{}:{}:{}:{}:{}:{}",
            self.event_name.as_str(),
            self.project_scope_id
                .map(|id| id.0.hyphenated().to_string())
                .unwrap_or_else(|| "global_sanitized".to_string()),
            self.payload.source_stream_key,
            stored.kind,
            stored.value,
            self.payload.source_version
        ))
    }
}

pub fn routes_for(event: KnowledgeEventNameV1) -> Vec<ProjectorRoute> {
    if event == KnowledgeEventNameV1::ReportRevisionFinalized {
        // Reporting artifacts are immutable canonical output, not retrieval or
        // Gate input. Until a real artifact indexer is composed, recording the
        // event must not manufacture a permanently pending placeholder
        // delivery and must never feed Assertion/Document/Embedding/Graph.
        return Vec::new();
    }
    vec![
        ProjectorRoute {
            projector: ProjectorId::AssertionPromoterV1,
            depends_on: None,
        },
        ProjectorRoute {
            projector: ProjectorId::DocumentProjectorV1,
            depends_on: Some(ProjectorId::AssertionPromoterV1),
        },
        ProjectorRoute {
            projector: ProjectorId::EmbeddingProjectorV1,
            depends_on: Some(ProjectorId::DocumentProjectorV1),
        },
        ProjectorRoute {
            projector: ProjectorId::GraphProjectorV1,
            depends_on: Some(ProjectorId::AssertionPromoterV1),
        },
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EventCatalogError {
    #[error("unsupported memory event schema version: {0}")]
    UnsupportedSchemaVersion(i32),
    #[error("memory event has an invalid canonical source")]
    InvalidSource,
    #[error("memory event source fields disagree with the typed source reference")]
    SourceEnvelopeMismatch,
}

impl EventCatalogError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedSchemaVersion(_) => "memory_event_schema_unsupported",
            Self::InvalidSource => "memory_event_source_invalid",
            Self::SourceEnvelopeMismatch => "memory_event_source_mismatch",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_retrievable_event_routes_graph_independently_after_assertions() {
        let routes = routes_for(KnowledgeEventNameV1::StageEpisodeClosed);
        assert_eq!(routes.len(), 4);
        assert_eq!(routes[0].depends_on, None);
        assert_eq!(routes[1].depends_on, Some(routes[0].projector));
        assert_eq!(routes[2].depends_on, Some(routes[1].projector));
        assert_eq!(routes[3].projector, ProjectorId::GraphProjectorV1);
        assert_eq!(routes[3].depends_on, Some(routes[0].projector));
    }

    #[test]
    fn post_exploit_terminal_event_uses_the_mandatory_projection_dag() {
        assert_eq!(
            KnowledgeEventNameV1::PostExploitFactTerminal.as_str(),
            "PostExploitFactTerminal.v1"
        );
        let routes = routes_for(KnowledgeEventNameV1::PostExploitFactTerminal);
        assert_eq!(
            routes,
            vec![
                ProjectorRoute {
                    projector: ProjectorId::AssertionPromoterV1,
                    depends_on: None,
                },
                ProjectorRoute {
                    projector: ProjectorId::DocumentProjectorV1,
                    depends_on: Some(ProjectorId::AssertionPromoterV1),
                },
                ProjectorRoute {
                    projector: ProjectorId::EmbeddingProjectorV1,
                    depends_on: Some(ProjectorId::DocumentProjectorV1),
                },
                ProjectorRoute {
                    projector: ProjectorId::GraphProjectorV1,
                    depends_on: Some(ProjectorId::AssertionPromoterV1),
                },
            ]
        );
    }

    #[test]
    fn prepared_side_effect_action_uses_the_mandatory_projection_dag() {
        assert_eq!(
            KnowledgeEventNameV1::PostExploitActionPrepared.as_str(),
            "PostExploitActionPrepared.v1"
        );
        assert_eq!(
            routes_for(KnowledgeEventNameV1::PostExploitActionPrepared),
            routes_for(KnowledgeEventNameV1::StageEpisodeClosed)
        );
    }

    #[test]
    fn finalized_report_has_no_rag_or_graph_gate_route() {
        assert!(routes_for(KnowledgeEventNameV1::ReportRevisionFinalized).is_empty());
    }
}
