use std::sync::Arc;

use async_trait::async_trait;
use golish_memory_app::{
    AuthorizationSnapshot, ContextError, OperationDataPolicyReader, ServerDataPolicy,
};
use golish_memory_domain::ContextSubject;
use sqlx::PgPool;

/// Server-owned policy resolver. V1 deliberately defaults to customer-local
/// retrieval and never accepts actor, classification, or provider approval
/// from ContextRequest/model fields.
pub struct KnowledgePolicyAdapter {
    pool: Arc<PgPool>,
}

impl KnowledgePolicyAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OperationDataPolicyReader for KnowledgePolicyAdapter {
    async fn resolve(
        &self,
        subject: &ContextSubject,
        snapshot: &AuthorizationSnapshot,
    ) -> Result<ServerDataPolicy, ContextError> {
        if subject.operation_id() != snapshot.operation_id
            || subject.organization_id() != snapshot.organization_id
        {
            return Err(ContextError::AuthorizationSnapshotMismatch);
        }
        let principal = golish_db::repo::operator_principals::current_local(&self.pool)
            .await
            .map_err(|_| ContextError::Source("operator_principal_unavailable".to_string()))?;
        if !principal.active || principal.principal_kind != "local_operator" {
            return Err(ContextError::AuthorizationSnapshotMismatch);
        }
        Ok(ServerDataPolicy::customer_local_only(principal.id))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn policy_adapter_has_no_request_selected_actor_or_external_approval() {
        let source = include_str!("knowledge_policy_adapter.rs");
        assert!(source.contains("operator_principals::current_local"));
        assert!(source.contains("customer_local_only"));
        let request_selected_actor = ["requested", "_actor"].concat();
        let external_policy_approval = ["approved_policy", "_id"].concat();
        assert!(!source.contains(&request_selected_actor));
        assert!(!source.contains(&external_policy_approval));
    }
}
