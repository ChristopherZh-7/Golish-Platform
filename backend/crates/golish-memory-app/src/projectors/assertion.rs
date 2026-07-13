use async_trait::async_trait;
use golish_memory_domain::event_catalog::KnowledgeEventEnvelopeV1;
use uuid::Uuid;

use crate::ports::MemoryError;

/// Port reserved for `assertion-promoter@1`. C2 connects canonical source
/// readers and the atomic writer; C1 intentionally has no live producer.
#[async_trait]
pub trait AssertionProjectionPort: Send + Sync {
    async fn promote_typed_event(
        &self,
        event: KnowledgeEventEnvelopeV1,
    ) -> Result<Vec<Uuid>, MemoryError>;
}
