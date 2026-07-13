use crate::ports::{InvalidateProjectionChainAndEmit, KnowledgeUnitOfWork, MemoryError};
use golish_memory_domain::event_catalog::KnowledgeEventNameV1;

pub struct InvalidationService<'a, U> {
    unit_of_work: &'a U,
}

impl<'a, U> InvalidationService<'a, U>
where
    U: KnowledgeUnitOfWork,
{
    pub const fn new(unit_of_work: &'a U) -> Self {
        Self { unit_of_work }
    }

    pub async fn invalidate(
        &self,
        command: InvalidateProjectionChainAndEmit,
    ) -> Result<(), MemoryError> {
        command
            .source
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        if command.reason_code.trim().is_empty() {
            return Err(MemoryError::Policy(
                "memory_invalidation_reason_empty".to_string(),
            ));
        }
        command
            .event
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        if command.event.event_name != KnowledgeEventNameV1::SourceScopeInvalidated
            || command.event.payload.source != command.source
        {
            return Err(MemoryError::Policy(
                "memory_invalidation_event_kind_mismatch".to_string(),
            ));
        }
        self.unit_of_work
            .invalidate_projection_chain_and_emit(command)
            .await
    }
}
