use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{KnowledgeClassification, ProjectScopeId, SourceRef};

pub const DEFAULT_CONTEXT_TOKEN_CAP: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeClass {
    CanonicalFact,
    RuntimeState,
    PassedHandoff,
    StageEpisode,
    AssertionPrior,
    DocumentPrior,
    TemporalGraphPrior,
    VectorPrior,
}

impl KnowledgeClass {
    pub const ALL: [Self; 8] = [
        Self::CanonicalFact,
        Self::RuntimeState,
        Self::PassedHandoff,
        Self::StageEpisode,
        Self::AssertionPrior,
        Self::DocumentPrior,
        Self::TemporalGraphPrior,
        Self::VectorPrior,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalFact => "canonical_fact",
            Self::RuntimeState => "runtime_state",
            Self::PassedHandoff => "passed_handoff",
            Self::StageEpisode => "stage_episode",
            Self::AssertionPrior => "assertion_prior",
            Self::DocumentPrior => "document_prior",
            Self::TemporalGraphPrior => "temporal_graph_prior",
            Self::VectorPrior => "vector_prior",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextAuthority {
    CanonicalDb,
    Runtime,
    Handoff,
    Episode,
    Assertion,
    Document,
    TemporalGraph,
    Vector,
}

impl ContextAuthority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalDb => "db_fact",
            Self::Runtime => "runtime",
            Self::Handoff => "handoff",
            Self::Episode => "episode",
            Self::Assertion => "assertion",
            Self::Document => "document",
            Self::TemporalGraph => "temporal_graph",
            Self::Vector => "vector",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultCredentialRef(pub Uuid);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "value_kind", content = "value", rename_all = "snake_case")]
pub enum KnowledgeValue {
    Text(String),
    Json(serde_json::Value),
    VaultRef(VaultCredentialRef),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub query_text: String,
    pub target_id: Option<Uuid>,
    pub candidate_id: Option<Uuid>,
    pub requested_classes: BTreeSet<KnowledgeClass>,
    pub requested_token_budget: usize,
}

impl ContextRequest {
    pub fn for_harness(query_text: impl Into<String>, token_budget: usize) -> Self {
        Self {
            query_text: query_text.into(),
            target_id: None,
            candidate_id: None,
            requested_classes: KnowledgeClass::ALL.into_iter().collect(),
            requested_token_budget: token_budget,
        }
    }

    pub fn validate(&self) -> Result<(), ContextContractError> {
        let query = self.query_text.trim();
        if query.is_empty() || query.chars().count() > 4_096 {
            return Err(ContextContractError::InvalidQuery);
        }
        if self.requested_token_budget == 0 {
            return Err(ContextContractError::InvalidTokenBudget);
        }
        Ok(())
    }
}

/// Server runtime identity hint. It is intentionally not `Deserialize`; the
/// provider re-resolves every field against frozen DB ownership before it can
/// create a trusted authorization context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSubject {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    worker_run_id: Option<Uuid>,
    organization_id: Uuid,
    stage_kind: String,
    wave: Option<i32>,
}

impl ContextSubject {
    #[allow(clippy::too_many_arguments)]
    pub fn from_server_runtime(
        operation_id: Uuid,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
        worker_run_id: Option<Uuid>,
        organization_id: Uuid,
        stage_kind: impl Into<String>,
        wave: Option<i32>,
    ) -> Result<Self, ContextContractError> {
        let stage_kind = stage_kind.into().trim().to_string();
        if stage_kind.is_empty() || wave.is_some_and(|value| value < 0) {
            return Err(ContextContractError::InvalidSubject);
        }
        Ok(Self {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            worker_run_id,
            organization_id,
            stage_kind,
            wave,
        })
    }

    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub const fn stage_execution_id(&self) -> Uuid {
        self.stage_execution_id
    }

    pub const fn stage_run_unit_id(&self) -> Uuid {
        self.stage_run_unit_id
    }

    pub const fn worker_run_id(&self) -> Option<Uuid> {
        self.worker_run_id
    }

    pub const fn organization_id(&self) -> Uuid {
        self.organization_id
    }

    pub fn stage_kind(&self) -> &str {
        &self.stage_kind
    }

    pub const fn wave(&self) -> Option<i32> {
        self.wave
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub item_id: String,
    pub class: KnowledgeClass,
    pub authority: ContextAuthority,
    pub value: KnowledgeValue,
    pub source_label: String,
    pub source_ref: Option<SourceRef>,
    pub project_scope_id: ProjectScopeId,
    pub source_operation_id: Uuid,
    pub scope_snapshot_id: Option<Uuid>,
    pub scope_snapshot_hash: String,
    pub organization_id_at_time: Uuid,
    pub classification: KnowledgeClassification,
    pub evidence_ids: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub content_hash: String,
    /// Integer micros keep ordering deterministic across platforms and avoid
    /// exposing provider-specific floating point behavior to prompt assembly.
    pub score_micros: i64,
    pub must_revalidate: bool,
}

impl ContextItem {
    pub fn validate(&self) -> Result<(), ContextContractError> {
        if self.item_id.trim().is_empty()
            || self.source_label.trim().is_empty()
            || self.scope_snapshot_hash.trim().is_empty()
            || self.content_hash.len() != 64
            || !self
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.evidence_ids.iter().any(|id| *id <= 0)
            || self.valid_to.is_some_and(|until| until < self.valid_from)
        {
            return Err(ContextContractError::InvalidItem);
        }
        if self
            .source_ref
            .as_ref()
            .is_some_and(|source| source.validate().is_err())
        {
            return Err(ContextContractError::InvalidItem);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ContextContractError {
    #[error("context query is empty or too large")]
    InvalidQuery,
    #[error("context token budget must be positive")]
    InvalidTokenBudget,
    #[error("runtime context subject is invalid")]
    InvalidSubject,
    #[error("context item is invalid")]
    InvalidItem,
}

impl ContextContractError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidQuery => "knowledge_context_query_invalid",
            Self::InvalidTokenBudget => "knowledge_context_token_budget_invalid",
            Self::InvalidSubject => "knowledge_context_subject_invalid",
            Self::InvalidItem => "knowledge_context_item_invalid",
        }
    }
}
