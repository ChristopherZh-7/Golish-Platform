use async_trait::async_trait;
use chrono::{DateTime, Utc};
use golish_memory_domain::{
    event_catalog::KnowledgeEventEnvelopeV1, KnowledgeAssertion, SourceRef, StageEpisode,
};
use uuid::Uuid;

use crate::context_pack::{
    AuthorizationSnapshot, ContextError, EffectiveContextQuery, ServerDataPolicy,
};
use crate::embedding_projector::{
    DocumentDeliverySnapshot, EmbeddingProjectionError, ProjectedEmbedding,
};
use crate::projectors::document::ProjectedDocument;
use golish_memory_domain::{ContextItem, ContextSubject};

#[derive(Clone, Debug)]
pub struct CloseEpisodeAndEmit {
    pub episode: StageEpisode,
    pub event: KnowledgeEventEnvelopeV1,
}

#[derive(Clone, Debug)]
pub struct PromoteAssertionAndEmit {
    pub assertion: KnowledgeAssertion,
    pub event: KnowledgeEventEnvelopeV1,
}

#[derive(Clone, Debug)]
pub struct InvalidateProjectionChainAndEmit {
    pub source: SourceRef,
    pub invalidated_at: DateTime<Utc>,
    pub reason_code: String,
    pub event: KnowledgeEventEnvelopeV1,
}

/// Each method is a single database transaction boundary. Implementations may
/// not split the canonical terminal row from its immutable outbox deliveries.
#[async_trait]
pub trait KnowledgeUnitOfWork: Send + Sync {
    async fn close_episode_and_emit(
        &self,
        command: CloseEpisodeAndEmit,
    ) -> Result<Uuid, MemoryError>;

    async fn promote_assertion_and_emit(
        &self,
        command: PromoteAssertionAndEmit,
    ) -> Result<Uuid, MemoryError>;

    async fn invalidate_projection_chain_and_emit(
        &self,
        command: InvalidateProjectionChainAndEmit,
    ) -> Result<(), MemoryError>;
}

#[async_trait]
pub trait DocumentProjectionPort: Send + Sync {
    async fn load_promoted_assertions(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError>;

    async fn upsert_document(&self, document: ProjectedDocument) -> Result<Uuid, MemoryError>;
}

#[async_trait]
pub trait AuthorizationSnapshotReader: Send + Sync {
    async fn load(&self, subject: &ContextSubject) -> Result<AuthorizationSnapshot, ContextError>;
}

#[async_trait]
pub trait OperationDataPolicyReader: Send + Sync {
    async fn resolve(
        &self,
        subject: &ContextSubject,
        snapshot: &AuthorizationSnapshot,
    ) -> Result<ServerDataPolicy, ContextError>;
}

#[async_trait]
pub trait KnowledgeContextSource: Send + Sync {
    async fn canonical(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn runtime(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn handoffs(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn episodes(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn assertions(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn documents(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn temporal_graph(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;
    async fn vector(
        &self,
        query: &EffectiveContextQuery,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<ContextItem>, ContextError>;
}

#[async_trait]
pub trait QueryEmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, ContextError>;
}

#[async_trait]
pub trait EmbeddingProjectionPort: Send + Sync {
    async fn load_document_delivery(
        &self,
        event_id: Uuid,
    ) -> Result<DocumentDeliverySnapshot, EmbeddingProjectionError>;

    async fn store_embedding(
        &self,
        embedding: ProjectedEmbedding,
    ) -> Result<Uuid, EmbeddingProjectionError>;
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MemoryError {
    #[error("memory port failed: {0}")]
    Port(String),
    #[error("event has no promoted assertions")]
    NoPromotedAssertions,
    #[error("assertions for one document do not share scope/source/version")]
    MixedDocumentSources,
    #[error("structured memory document serialization failed")]
    Serialization,
    #[error("memory policy rejected the command: {0}")]
    Policy(String),
    #[error("graph projection rejected the assertion: {0}")]
    GraphProjection(String),
}

impl MemoryError {
    pub fn code(&self) -> &str {
        match self {
            Self::Port(_) => "memory_port_failure",
            Self::NoPromotedAssertions => "memory_document_assertions_missing",
            Self::MixedDocumentSources => "memory_document_source_mismatch",
            Self::Serialization => "memory_document_serialization_failed",
            Self::Policy(_) => "memory_policy_rejected",
            Self::GraphProjection(code) => code,
        }
    }
}
