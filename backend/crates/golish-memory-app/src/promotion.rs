use uuid::Uuid;

use crate::ports::{KnowledgeUnitOfWork, MemoryError, PromoteAssertionAndEmit};
use golish_memory_domain::event_catalog::KnowledgeEventNameV1;
use golish_memory_domain::source_ref::CanonicalSourceKind;

/// Deterministic application service. It accepts only an already typed and
/// domain-validated assertion/event pair; raw stdout and model prose have no
/// representation in this API.
pub struct PromotionService<'a, U> {
    unit_of_work: &'a U,
}

impl<'a, U> PromotionService<'a, U>
where
    U: KnowledgeUnitOfWork,
{
    pub const fn new(unit_of_work: &'a U) -> Self {
        Self { unit_of_work }
    }

    pub async fn promote(&self, command: PromoteAssertionAndEmit) -> Result<Uuid, MemoryError> {
        command
            .event
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        if command.assertion.source != command.event.payload.source {
            return Err(MemoryError::Policy(
                "memory_assertion_event_source_mismatch".to_string(),
            ));
        }
        let source_kind_matches_event = matches!(
            (
                command.event.event_name,
                command.assertion.source.source_kind
            ),
            (
                KnowledgeEventNameV1::CandidateAttemptTerminal,
                CanonicalSourceKind::CandidateAttempt
            ) | (
                KnowledgeEventNameV1::FactDeltaAccepted,
                CanonicalSourceKind::FactDelta
            ) | (
                KnowledgeEventNameV1::PostExploitActionPrepared,
                CanonicalSourceKind::PostExploitAction
            ) | (
                KnowledgeEventNameV1::PostExploitFactTerminal,
                CanonicalSourceKind::Foothold | CanonicalSourceKind::ObjectiveOutcome
            ) | (
                KnowledgeEventNameV1::CleanupObligationTerminal,
                CanonicalSourceKind::CleanupObligation | CanonicalSourceKind::ResidualRisk
            )
        );
        if !source_kind_matches_event {
            return Err(MemoryError::Policy(
                "memory_promotion_event_kind_mismatch".to_string(),
            ));
        }
        self.unit_of_work.promote_assertion_and_emit(command).await
    }
}
