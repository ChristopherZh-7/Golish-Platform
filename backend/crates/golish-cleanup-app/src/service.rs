use std::sync::Arc;

use golish_cleanup_domain::{
    CleanupError, CleanupObligationId, NewCleanupObligation, PendingSideEffectAction,
    TrustedOperatorPrincipal, WaiverRequest,
};
use golish_post_exploit_domain::ActionId;

use crate::ports::CleanupObligationPort;

#[derive(Clone)]
pub struct CleanupKernel<P> {
    repository: Arc<P>,
}

impl<P> CleanupKernel<P>
where
    P: CleanupObligationPort,
{
    pub fn new(repository: Arc<P>) -> Self {
        Self { repository }
    }

    pub async fn prepare_side_effect(
        &self,
        action: PendingSideEffectAction,
        obligation: NewCleanupObligation,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<(ActionId, CleanupObligationId), CleanupError> {
        self.repository
            .record_action_and_obligation(action, obligation, actor)
            .await
    }

    pub async fn waive_obligation(
        &self,
        request: WaiverRequest,
        actor: &TrustedOperatorPrincipal,
    ) -> Result<golish_cleanup_domain::CleanupObligation, CleanupError> {
        self.repository.waive_obligation(request, actor).await
    }
}
