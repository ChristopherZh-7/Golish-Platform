//! Port-driven Memory Fabric application services.
//!
//! C1 defines the pure orchestration seams and deterministic projectors. The
//! live canonical writers and process-global supervisor are connected in C2.

pub mod context_pack;
pub mod embedding_projector;
pub mod graph_projection;
pub mod invalidation;
pub mod outbox;
pub mod ports;
pub mod projectors;
pub mod promotion;
pub mod ranking;
pub mod redaction;
pub mod retrieval;
pub mod supervisor;

pub use context_pack::{
    AuthorizationSnapshot, ContextError, ContextOmissionSummary, ContextPack, ContextPackProvider,
    EffectiveContextQuery, ServerDataPolicy, TrustedAuthorizationContext,
};
pub use embedding_projector::{
    DocumentDeliverySnapshot, DocumentDeliveryStatus, EmbeddingDocument, EmbeddingProjectionError,
    EmbeddingProjectionOutcome, EmbeddingProjector, EmbeddingProvider, ProjectedEmbedding,
};
pub use graph_projection::{project_assertion, project_invalidation, ProjectionError};
pub use invalidation::InvalidationService;
pub use ports::{
    AuthorizationSnapshotReader, DocumentProjectionPort, EmbeddingProjectionPort,
    KnowledgeContextSource, KnowledgeUnitOfWork, MemoryError, OperationDataPolicyReader,
    QueryEmbeddingProvider,
};
pub use projectors::document::{DocumentProjector, ProjectedDocument};
pub use projectors::graph::{
    rebuild_graph_scope_from_assertions, GraphAssertionReader, GraphDeliveryOutcome,
    GraphProjectionDelivery, GraphProjectionDeliveryPort, GraphProjector, GraphProjectorTick,
    GraphRebuildScope, TemporalGraphProjectionPort,
};
pub use promotion::PromotionService;
pub use redaction::{escape_prompt_markup, render_safe_value, RedactionError};
pub use retrieval::KnowledgeRetriever;
pub use supervisor::{
    KnowledgeProjectorSupervisor, KnowledgeProjectorSupervisorPort, KnowledgeProjectorWorker,
    ProjectorRunState, SupervisorStartOutcome,
};
