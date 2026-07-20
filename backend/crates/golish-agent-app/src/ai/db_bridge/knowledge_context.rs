use std::sync::Arc;

use async_trait::async_trait;
use golish_db::repo::{knowledge_assertions, knowledge_context};
use golish_graphiti::{ScopedGraphQuery, TemporalGraphClient};
use golish_memory_app::{
    AuthorizationSnapshot, AuthorizationSnapshotReader, ContextError, ContextPack,
    ContextPackProvider, EffectiveContextQuery, KnowledgeContextSource, KnowledgeRetriever,
    QueryEmbeddingProvider,
};
use golish_memory_domain::{
    ContextAuthority, ContextItem, ContextRequest, ContextSubject, KnowledgeClass,
    KnowledgeClassification, KnowledgeValue, ProjectScopeId, VaultCredentialRef,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::ai::knowledge_policy_adapter::KnowledgePolicyAdapter;

pub struct PgKnowledgeContextAdapter {
    retriever: KnowledgeRetriever,
}

impl PgKnowledgeContextAdapter {
    pub fn new(pool: Arc<PgPool>) -> Result<Self, ContextError> {
        Self::with_query_embedding(pool, None)
    }

    pub fn with_query_embedding(
        pool: Arc<PgPool>,
        query_embedding: Option<Arc<dyn QueryEmbeddingProvider>>,
    ) -> Result<Self, ContextError> {
        let authorization = Arc::new(PgKnowledgeAuthorizationReader { pool: pool.clone() });
        let policy = Arc::new(KnowledgePolicyAdapter::new(pool.clone()));
        let source = Arc::new(PgKnowledgeContextSource {
            graph: TemporalGraphClient::new(pool.as_ref().clone()),
            pool,
        });
        Ok(Self {
            retriever: KnowledgeRetriever::new(authorization, policy, source, query_embedding)?,
        })
    }
}

#[async_trait]
impl ContextPackProvider for PgKnowledgeContextAdapter {
    async fn retrieve(
        &self,
        subject: ContextSubject,
        request: ContextRequest,
    ) -> Result<ContextPack, ContextError> {
        self.retriever.retrieve(subject, request).await
    }
}

struct PgKnowledgeAuthorizationReader {
    pool: Arc<PgPool>,
}

#[async_trait]
impl AuthorizationSnapshotReader for PgKnowledgeAuthorizationReader {
    async fn load(&self, subject: &ContextSubject) -> Result<AuthorizationSnapshot, ContextError> {
        let row = knowledge_context::load_authorization_snapshot(
            &self.pool,
            subject.operation_id(),
            subject.stage_execution_id(),
            subject.stage_run_unit_id(),
            subject.worker_run_id(),
            subject.organization_id(),
            subject.stage_kind(),
        )
        .await
        .map_err(database_error)?
        .ok_or(ContextError::AuthorizationSnapshotMismatch)?;
        Ok(AuthorizationSnapshot {
            project_scope_id: ProjectScopeId(row.project_scope_id),
            operation_id: row.operation_id,
            scope_snapshot_id: row.scope_snapshot_id,
            scope_snapshot_hash: row.scope_snapshot_hash,
            organization_id: row.organization_id,
            frozen_organization_ids: row.frozen_organization_ids.into_iter().collect(),
            server_now: row.server_now,
        })
    }
}

struct PgKnowledgeContextSource {
    pool: Arc<PgPool>,
    graph: TemporalGraphClient,
}

#[async_trait]
impl KnowledgeContextSource for PgKnowledgeContextSource {
    async fn canonical(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::canonical_current(
                &self.pool,
                trusted.operation_id(),
                trusted.scope_snapshot_id(),
                trusted.organization_id(),
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::CanonicalFact,
            ContextAuthority::CanonicalDb,
        )
    }

    async fn runtime(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::runtime_current(
                &self.pool,
                trusted.operation_id(),
                trusted.scope_snapshot_id(),
                trusted.organization_id(),
                trusted.stage_execution_id(),
                trusted.stage_run_unit_id(),
                trusted.worker_run_id(),
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::RuntimeState,
            ContextAuthority::Runtime,
        )
    }

    async fn handoffs(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::current_handoffs(
                &self.pool,
                trusted.operation_id(),
                trusted.scope_snapshot_id(),
                trusted.organization_id(),
                trusted.scope_snapshot_hash(),
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::PassedHandoff,
            ContextAuthority::Handoff,
        )
    }

    async fn episodes(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::current_episodes(
                &self.pool,
                trusted.operation_id(),
                trusted.scope_snapshot_id(),
                trusted.organization_id(),
                trusted.scope_snapshot_hash(),
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::StageEpisode,
            ContextAuthority::Episode,
        )
    }

    async fn assertions(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::active_assertions(
                &self.pool,
                trusted.project_scope_id().0,
                trusted.organization_id(),
                trusted.server_now(),
                trusted.classification_ceiling().as_str(),
                &query.request().query_text,
                50,
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::AssertionPrior,
            ContextAuthority::Assertion,
        )
    }

    async fn documents(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::active_documents(
                &self.pool,
                trusted.project_scope_id().0,
                trusted.organization_id(),
                trusted.server_now(),
                trusted.classification_ceiling().as_str(),
                &query.request().query_text,
                30,
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::DocumentPrior,
            ContextAuthority::Document,
        )
    }

    async fn temporal_graph(
        &self,
        query: &EffectiveContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let trusted = query.trusted();
        let result = self
            .graph
            .query(
                ScopedGraphQuery::for_organization(
                    trusted.project_scope_id(),
                    trusted.organization_id(),
                    &query.request().query_text,
                    trusted.server_now(),
                )
                .with_classification_ceiling(trusted.classification_ceiling()),
            )
            .await
            .map_err(|_| ContextError::Source("temporal_graph_query_failed".to_string()))?;
        let mut items = Vec::with_capacity(result.entities.len() + result.relations.len());
        for entity in result.entities {
            let Some(lineage) = entity.lineages.first() else {
                return Err(ContextError::InvalidItem);
            };
            let assertion = knowledge_assertions::get(&self.pool, lineage.assertion_id)
                .await
                .map_err(|_| ContextError::Source("graph_lineage_assertion_missing".to_string()))?;
            let value = serde_json::json!({
                "canonical_ref": entity.canonical_ref,
                "entity_type": entity.entity_type.as_str(),
                "display_name": entity.display_name,
                "properties": entity.properties,
                "assertion_ids": entity.lineages.iter().map(|lineage| lineage.assertion_id).collect::<Vec<_>>(),
            });
            items.push(graph_item(
                format!("graph_entity:{}", entity.entity_id),
                value,
                &assertion,
                trusted,
            )?);
        }
        for relation in result.relations {
            let Some(lineage) = relation.lineages.first() else {
                return Err(ContextError::InvalidItem);
            };
            let assertion = knowledge_assertions::get(&self.pool, lineage.assertion_id)
                .await
                .map_err(|_| ContextError::Source("graph_lineage_assertion_missing".to_string()))?;
            let value = serde_json::json!({
                "from": relation.from_canonical_ref,
                "relation": relation.relation_type.as_str(),
                "to": relation.to_canonical_ref,
                "properties": relation.properties,
                "assertion_ids": relation.lineages.iter().map(|lineage| lineage.assertion_id).collect::<Vec<_>>(),
            });
            items.push(graph_item(
                format!("graph_relation:{}", relation.relation_id),
                value,
                &assertion,
                trusted,
            )?);
        }
        Ok(items)
    }

    async fn vector(
        &self,
        query: &EffectiveContextQuery,
        query_embedding: Option<&[f32]>,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let Some(query_embedding) = query_embedding else {
            return Ok(Vec::new());
        };
        let trusted = query.trusted();
        rows_to_items(
            knowledge_context::vector_documents(
                &self.pool,
                trusted.project_scope_id().0,
                trusted.organization_id(),
                trusted.server_now(),
                trusted.classification_ceiling().as_str(),
                query_embedding,
                20,
            )
            .await
            .map_err(database_error)?,
            KnowledgeClass::VectorPrior,
            ContextAuthority::Vector,
        )
    }
}

fn rows_to_items(
    rows: Vec<knowledge_context::KnowledgeContextRow>,
    class: KnowledgeClass,
    authority: ContextAuthority,
) -> Result<Vec<ContextItem>, ContextError> {
    rows.into_iter()
        .map(|row| row_to_item(row, class, authority))
        .collect()
}

fn row_to_item(
    row: knowledge_context::KnowledgeContextRow,
    class: KnowledgeClass,
    authority: ContextAuthority,
) -> Result<ContextItem, ContextError> {
    let value = match row.value_kind.as_str() {
        "text" => KnowledgeValue::Text(row.text_value.ok_or(ContextError::InvalidItem)?),
        "json" => KnowledgeValue::Json(row.json_value.ok_or(ContextError::InvalidItem)?),
        "vault_ref" => KnowledgeValue::VaultRef(VaultCredentialRef(
            row.vault_ref.ok_or(ContextError::InvalidItem)?,
        )),
        _ => return Err(ContextError::InvalidItem),
    };
    let classification = parse_classification(&row.classification)?;
    let content_hash = match row.content_hash {
        Some(content_hash) => content_hash,
        None => hash_value(&value)?,
    };
    let item = ContextItem {
        item_id: row.item_id,
        class,
        authority,
        value,
        source_label: row.source_label,
        source_ref: None,
        project_scope_id: ProjectScopeId(row.project_scope_id),
        source_operation_id: row.source_operation_id,
        scope_snapshot_id: row.scope_snapshot_id,
        scope_snapshot_hash: row.scope_snapshot_hash,
        organization_id_at_time: row.organization_id_at_time,
        classification,
        evidence_ids: row.evidence_refs,
        valid_from: row.valid_from,
        valid_to: row.valid_to,
        content_hash,
        score_micros: row.score_micros,
        must_revalidate: row.must_revalidate,
    };
    item.validate().map_err(|_| ContextError::InvalidItem)?;
    Ok(item)
}

fn graph_item(
    item_id: String,
    value: serde_json::Value,
    assertion: &golish_memory_domain::KnowledgeAssertion,
    trusted: &golish_memory_app::TrustedAuthorizationContext,
) -> Result<ContextItem, ContextError> {
    let value = KnowledgeValue::Json(value);
    let item = ContextItem {
        item_id,
        class: KnowledgeClass::TemporalGraphPrior,
        authority: ContextAuthority::TemporalGraph,
        value: value.clone(),
        source_label: "scoped_temporal_graph".to_string(),
        source_ref: Some(assertion.source.clone()),
        project_scope_id: trusted.project_scope_id(),
        source_operation_id: assertion.source_operation_id,
        scope_snapshot_id: None,
        scope_snapshot_hash: assertion.source_scope_snapshot_hash.clone(),
        organization_id_at_time: trusted.organization_id(),
        classification: assertion.classification,
        evidence_ids: assertion.evidence_ids.clone(),
        valid_from: assertion.valid_from,
        valid_to: assertion.valid_to,
        content_hash: hash_value(&value)?,
        score_micros: 600_000,
        must_revalidate: true,
    };
    item.validate().map_err(|_| ContextError::InvalidItem)?;
    Ok(item)
}

fn hash_value(value: &KnowledgeValue) -> Result<String, ContextError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| ContextError::Source("context_value_serialization_failed".to_string()))?;
    Ok(Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn parse_classification(value: &str) -> Result<KnowledgeClassification, ContextError> {
    match value {
        "public" => Ok(KnowledgeClassification::Public),
        "internal" => Ok(KnowledgeClassification::Internal),
        "customer_confidential" => Ok(KnowledgeClassification::CustomerConfidential),
        "restricted" => Ok(KnowledgeClassification::Restricted),
        _ => Err(ContextError::InvalidItem),
    }
}

fn database_error(error: sqlx::Error) -> ContextError {
    ContextError::Source(format!("knowledge_context_database_error:{error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_adapter_uses_only_scoped_v2_sources() {
        let source = include_str!("knowledge_context.rs");
        assert!(source.contains("load_authorization_snapshot"));
        assert!(source.contains("ScopedGraphQuery::for_organization"));
        assert!(source.contains("vector_documents"));
        let legacy_wiki_search = ["wiki_search", "_fts"].concat();
        let legacy_memory_briefing = ["fetch_memories", "_for_briefing"].concat();
        assert!(!source.contains(&legacy_wiki_search));
        assert!(!source.contains(&legacy_memory_briefing));
    }
}
