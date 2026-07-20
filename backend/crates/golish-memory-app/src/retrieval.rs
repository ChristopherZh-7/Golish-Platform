use std::sync::Arc;

use async_trait::async_trait;
use golish_memory_domain::{
    validate_embedding_dimension, ContextAuthority, ContextItem, ContextRequest, ContextSubject,
    KnowledgeClass,
};

use crate::context_pack::{
    ContextError, ContextOmissionSummary, ContextPack, ContextPackProvider, EffectiveContextQuery,
    TrustedAuthorizationContextLoader,
};
use crate::ports::{
    AuthorizationSnapshotReader, KnowledgeContextSource, OperationDataPolicyReader,
    QueryEmbeddingProvider,
};
use crate::ranking::{estimated_tokens, stable_rank};

pub struct KnowledgeRetriever {
    loader: TrustedAuthorizationContextLoader,
    source: Arc<dyn KnowledgeContextSource>,
    query_embedding: Option<Arc<dyn QueryEmbeddingProvider>>,
}

impl KnowledgeRetriever {
    pub fn new(
        authorization: Arc<dyn AuthorizationSnapshotReader>,
        policy: Arc<dyn OperationDataPolicyReader>,
        source: Arc<dyn KnowledgeContextSource>,
        query_embedding: Option<Arc<dyn QueryEmbeddingProvider>>,
    ) -> Result<Self, ContextError> {
        if let Some(provider) = query_embedding.as_ref() {
            validate_embedding_dimension(provider.dimension()).map_err(|_| {
                ContextError::InvalidRequest("knowledge_query_embedding_dimension_mismatch")
            })?;
        }
        Ok(Self {
            loader: TrustedAuthorizationContextLoader::new(authorization, policy),
            source,
            query_embedding,
        })
    }

    async fn retrieve_inner(
        &self,
        subject: ContextSubject,
        request: ContextRequest,
    ) -> Result<ContextPack, ContextError> {
        // This ordering is security-significant: the DB-owned scope snapshot
        // and server policy are resolved before any customer knowledge query.
        let trusted = self.loader.load(&subject).await?;
        let query = EffectiveContextQuery::intersect(trusted, request, subject.stage_kind())?;

        let canonical_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::CanonicalFact)
        {
            self.read_layer(
                &query,
                KnowledgeClass::CanonicalFact,
                ContextAuthority::CanonicalDb,
                true,
                self.source.canonical(&query).await?,
            )?
        } else {
            Vec::new()
        };
        let runtime_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::RuntimeState)
        {
            self.read_layer(
                &query,
                KnowledgeClass::RuntimeState,
                ContextAuthority::Runtime,
                true,
                self.source.runtime(&query).await?,
            )?
        } else {
            Vec::new()
        };
        let handoff_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::PassedHandoff)
        {
            self.read_layer(
                &query,
                KnowledgeClass::PassedHandoff,
                ContextAuthority::Handoff,
                true,
                self.source.handoffs(&query).await?,
            )?
        } else {
            Vec::new()
        };
        let episode_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::StageEpisode)
        {
            self.read_layer(
                &query,
                KnowledgeClass::StageEpisode,
                ContextAuthority::Episode,
                true,
                self.source.episodes(&query).await?,
            )?
        } else {
            Vec::new()
        };
        let assertion_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::AssertionPrior)
        {
            self.read_layer(
                &query,
                KnowledgeClass::AssertionPrior,
                ContextAuthority::Assertion,
                false,
                self.source.assertions(&query).await?,
            )?
        } else {
            Vec::new()
        };
        let document_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::DocumentPrior)
        {
            self.read_layer(
                &query,
                KnowledgeClass::DocumentPrior,
                ContextAuthority::Document,
                false,
                self.source.documents(&query).await?,
            )?
        } else {
            Vec::new()
        };

        let mut omitted = ContextOmissionSummary::default();
        let graph_items = if query
            .allowed_classes()
            .contains(&KnowledgeClass::TemporalGraphPrior)
        {
            match self.source.temporal_graph(&query).await {
                Ok(items) => self.read_layer(
                    &query,
                    KnowledgeClass::TemporalGraphPrior,
                    ContextAuthority::TemporalGraph,
                    false,
                    items,
                )?,
                Err(error) => {
                    omitted
                        .reasons
                        .push(format!("temporal_graph_degraded:{}", error.code()));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Vector retrieval is always last. Local-only providers are safe under
        // the default customer-local policy; providers that require data egress
        // are never called without policy-derived authorization.
        let vector_allowed = query
            .allowed_classes()
            .contains(&KnowledgeClass::VectorPrior);
        let query_embedding = match self.query_embedding.as_ref() {
            Some(_) if !vector_allowed => None,
            Some(provider)
                if !provider.requires_external_data_egress()
                    || query.trusted().allows_external_embedding() =>
            {
                match provider.embed_query(&query.request().query_text).await {
                    Ok(embedding) if validate_embedding_dimension(embedding.len()).is_ok() => {
                        Some(embedding)
                    }
                    Ok(_) => {
                        omitted
                            .reasons
                            .push("vector_query_dimension_mismatch".to_string());
                        None
                    }
                    Err(error) => {
                        omitted
                            .reasons
                            .push(format!("vector_query_degraded:{}", error.code()));
                        None
                    }
                }
            }
            Some(_) => {
                omitted
                    .reasons
                    .push("external_embedding_policy_denied".to_string());
                None
            }
            None => None,
        };
        let vector_items = if vector_allowed {
            match self.source.vector(&query, query_embedding.as_deref()).await {
                Ok(items) => self.read_layer(
                    &query,
                    KnowledgeClass::VectorPrior,
                    ContextAuthority::Vector,
                    false,
                    items,
                )?,
                Err(error) => {
                    omitted
                        .reasons
                        .push(format!("vector_degraded:{}", error.code()));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        apply_budget(
            &query,
            ContextPack {
                canonical_items,
                runtime_items,
                handoff_items,
                episode_items,
                assertion_items,
                document_items,
                graph_items,
                vector_items,
                omitted,
            },
        )
    }

    fn read_layer(
        &self,
        query: &EffectiveContextQuery,
        class: KnowledgeClass,
        authority: ContextAuthority,
        exact_operation: bool,
        mut items: Vec<ContextItem>,
    ) -> Result<Vec<ContextItem>, ContextError> {
        if !query.allowed_classes().contains(&class) {
            return Ok(Vec::new());
        }
        let trusted = query.trusted();
        for item in &items {
            item.validate().map_err(|_| ContextError::InvalidItem)?;
            if item.class != class
                || item.authority != authority
                || item.project_scope_id != trusted.project_scope_id()
                || item.organization_id_at_time != trusted.organization_id()
                || !trusted.classification_ceiling().allows(item.classification)
                || item.valid_from > trusted.server_now()
                || item
                    .valid_to
                    .is_some_and(|valid_to| valid_to <= trusted.server_now())
            {
                return Err(ContextError::InvalidItem);
            }
            if exact_operation
                && (item.source_operation_id != trusted.operation_id()
                    || item.scope_snapshot_id != Some(trusted.scope_snapshot_id())
                    || item.scope_snapshot_hash != trusted.scope_snapshot_hash())
            {
                return Err(ContextError::InvalidItem);
            }
            if !exact_operation && !item.must_revalidate {
                return Err(ContextError::InvalidItem);
            }
        }
        stable_rank(&mut items);
        Ok(items)
    }
}

#[async_trait]
impl ContextPackProvider for KnowledgeRetriever {
    async fn retrieve(
        &self,
        subject: ContextSubject,
        request: ContextRequest,
    ) -> Result<ContextPack, ContextError> {
        self.retrieve_inner(subject, request).await
    }
}

fn apply_budget(
    query: &EffectiveContextQuery,
    mut pack: ContextPack,
) -> Result<ContextPack, ContextError> {
    let mandatory_tokens = pack
        .canonical_items
        .iter()
        .chain(&pack.runtime_items)
        .map(estimated_tokens)
        .sum::<usize>();
    if mandatory_tokens > query.token_budget() {
        return Err(ContextError::MandatoryContextTooLarge {
            required_tokens: mandatory_tokens,
            server_cap: query.token_budget(),
        });
    }
    let mut remaining = query.token_budget() - mandatory_tokens;
    for layer in [
        &mut pack.handoff_items,
        &mut pack.episode_items,
        &mut pack.assertion_items,
        &mut pack.document_items,
        &mut pack.graph_items,
        &mut pack.vector_items,
    ] {
        let mut kept = Vec::with_capacity(layer.len());
        for item in std::mem::take(layer) {
            let tokens = estimated_tokens(&item);
            if tokens <= remaining {
                remaining -= tokens;
                kept.push(item);
            } else {
                pack.omitted.omitted_count += 1;
                pack.omitted.item_ids.push(item.item_id);
            }
        }
        *layer = kept;
    }
    if pack.omitted.omitted_count > 0 {
        pack.omitted.reasons.push("token_budget".to_string());
    }
    Ok(pack)
}
