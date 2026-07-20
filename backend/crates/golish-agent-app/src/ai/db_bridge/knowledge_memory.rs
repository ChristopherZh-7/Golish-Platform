use std::sync::Arc;

use async_trait::async_trait;
use golish_db::embeddings::Embedder;
use golish_db::repo::{
    cleanup_obligations, knowledge_assertions, knowledge_documents, knowledge_embeddings,
    knowledge_outbox, post_exploit_actions, stage_episodes,
};
use golish_graphiti::TemporalGraphClient;
use golish_memory_app::projectors::document::DOCUMENT_PROJECTION_SCHEMA_V1;
use golish_memory_app::{
    ContextError, DocumentProjectionPort, DocumentProjector, GraphAssertionReader,
    GraphDeliveryOutcome, GraphProjectionDelivery, GraphProjectionDeliveryPort, GraphProjector,
    GraphProjectorTick, GraphRebuildScope, KnowledgeProjectorSupervisor, KnowledgeProjectorWorker,
    KnowledgeUnitOfWork, MemoryError, ProjectedDocument, ProjectorRunState, QueryEmbeddingProvider,
    SupervisorStartOutcome,
};
use golish_memory_domain::assertion::{
    AssertionIdentity, AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertionDraft,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, ProjectorId,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind};
use golish_memory_domain::{KnowledgeAssertion, StageEpisode, EMBEDDING_DIMENSION_V1};
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use golish_memory_app::ports::{
    CloseEpisodeAndEmit, InvalidateProjectionChainAndEmit, PromoteAssertionAndEmit,
};

const REDACTION_POLICY_VERSION_V1: i32 = 1;
const MAX_DELIVERY_ATTEMPTS: i32 = 5;
const CANDIDATE_REASON_ONLY_BLOCKED_SUPPRESSION: &str =
    "memory_candidate_reason_only_blocked_no_audit_evidence";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssertionPromotionPolicy {
    DeriveFromCanonicalEvent,
    DeriveCandidateOrSuppressReasonOnlyBlocked,
    RequiresProducerAssertion,
    ProducerPreclosedInvalidation,
    NoProjectionRoute,
}

const fn assertion_promotion_policy(event_name: KnowledgeEventNameV1) -> AssertionPromotionPolicy {
    match event_name {
        KnowledgeEventNameV1::StageEpisodeClosed
        | KnowledgeEventNameV1::PostExploitActionPrepared
        | KnowledgeEventNameV1::PostExploitFactTerminal
        | KnowledgeEventNameV1::CleanupObligationTerminal => {
            AssertionPromotionPolicy::DeriveFromCanonicalEvent
        }
        KnowledgeEventNameV1::CandidateAttemptTerminal => {
            AssertionPromotionPolicy::DeriveCandidateOrSuppressReasonOnlyBlocked
        }
        KnowledgeEventNameV1::FactDeltaAccepted => {
            AssertionPromotionPolicy::RequiresProducerAssertion
        }
        KnowledgeEventNameV1::SourceScopeInvalidated => {
            AssertionPromotionPolicy::ProducerPreclosedInvalidation
        }
        KnowledgeEventNameV1::ReportRevisionFinalized => {
            AssertionPromotionPolicy::NoProjectionRoute
        }
    }
}

#[derive(Clone)]
pub struct KnowledgeEmbeddingProvider {
    provider_name: String,
    embedder: Arc<dyn Embedder>,
}

impl KnowledgeEmbeddingProvider {
    pub fn new(
        provider_name: impl Into<String>,
        embedder: Arc<dyn Embedder>,
    ) -> Result<Self, MemoryError> {
        let provider_name = provider_name.into().trim().to_string();
        if provider_name.is_empty() || embedder.model_name().trim().is_empty() {
            return Err(MemoryError::Policy(
                "memory_embedding_provider_identity_invalid".to_string(),
            ));
        }
        if embedder.dimension() != EMBEDDING_DIMENSION_V1 {
            return Err(MemoryError::Policy(
                "memory_embedding_provider_dimension_mismatch".to_string(),
            ));
        }
        Ok(Self {
            provider_name,
            embedder,
        })
    }
}

#[async_trait]
impl QueryEmbeddingProvider for KnowledgeEmbeddingProvider {
    fn dimension(&self) -> usize {
        self.embedder.dimension()
    }

    fn requires_external_data_egress(&self) -> bool {
        false
    }

    async fn embed_query(&self, query: &str) -> Result<Vec<f32>, ContextError> {
        let embedding =
            self.embedder.embed(query).await.map_err(|_| {
                ContextError::Source("knowledge_query_embedding_failed".to_string())
            })?;
        if embedding.len() != EMBEDDING_DIMENSION_V1
            || embedding.iter().any(|value| !value.is_finite())
        {
            return Err(ContextError::Source(
                "knowledge_query_embedding_invalid".to_string(),
            ));
        }
        Ok(embedding)
    }
}

/// Production Memory Fabric adapter. It owns no background task; the process
/// composition root passes the same Arc both to bridges and to the one shared
/// [`KnowledgeMemoryRuntime`].
pub struct PgKnowledgeMemory {
    pool: Arc<PgPool>,
    graph: TemporalGraphClient,
    embedding: Option<KnowledgeEmbeddingProvider>,
}

struct ScopedEventAuthority {
    project_scope_id: ProjectScopeId,
    organization_id_at_time: Uuid,
    source_scope_snapshot_hash: String,
}

impl PgKnowledgeMemory {
    pub fn new(pool: Arc<PgPool>, embedding: Option<KnowledgeEmbeddingProvider>) -> Self {
        Self {
            graph: TemporalGraphClient::new(pool.as_ref().clone()),
            pool,
            embedding,
        }
    }

    pub fn pool(&self) -> &PgPool {
        self.pool.as_ref()
    }

    async fn claim_event(
        &self,
        projector: ProjectorId,
        worker_id: &str,
    ) -> Result<Option<KnowledgeEventEnvelopeV1>, MemoryError> {
        let Some(delivery) =
            knowledge_outbox::claim_delivery_batch(self.pool(), projector, worker_id, 1)
                .await
                .map_err(outbox_error)?
                .into_iter()
                .next()
        else {
            return Ok(None);
        };
        match knowledge_outbox::get_event(self.pool(), delivery.event_id).await {
            Ok(event) => Ok(Some(event)),
            Err(error) => {
                let code = error.code();
                knowledge_outbox::fail_delivery(
                    self.pool(),
                    delivery.event_id,
                    projector,
                    worker_id,
                    code,
                    MAX_DELIVERY_ATTEMPTS,
                )
                .await
                .map_err(outbox_error)?;
                Err(outbox_error(error))
            }
        }
    }

    async fn complete_claimed(
        &self,
        event_id: Uuid,
        projector: ProjectorId,
        worker_id: &str,
        result: Result<Option<String>, MemoryError>,
    ) -> Result<ProjectorRunState, MemoryError> {
        match result {
            Ok(reason) => {
                let (status, reason) = match reason {
                    Some(reason) => (
                        knowledge_outbox::DeliveryStatus::SucceededSuppressed,
                        Some(reason),
                    ),
                    None => (knowledge_outbox::DeliveryStatus::Succeeded, None),
                };
                knowledge_outbox::complete_delivery(
                    self.pool(),
                    event_id,
                    projector,
                    worker_id,
                    status,
                    reason.as_deref(),
                )
                .await
                .map_err(outbox_error)?;
                Ok(ProjectorRunState::Processed)
            }
            Err(error) => {
                knowledge_outbox::fail_delivery(
                    self.pool(),
                    event_id,
                    projector,
                    worker_id,
                    error.code(),
                    MAX_DELIVERY_ATTEMPTS,
                )
                .await
                .map_err(outbox_error)?;
                Err(error)
            }
        }
    }

    async fn run_assertion_once(&self, worker_id: &str) -> Result<ProjectorRunState, MemoryError> {
        let Some(event) = self
            .claim_event(ProjectorId::AssertionPromoterV1, worker_id)
            .await?
        else {
            return Ok(ProjectorRunState::Idle);
        };
        let event_id = event.event_id;
        let result = async {
            let derived_assertion = match event.event_name {
                KnowledgeEventNameV1::StageEpisodeClosed => {
                    let episode_id = match &event.payload.source.row_id {
                        CanonicalRowId::Uuid(episode_id)
                            if event.payload.source.source_kind
                                == CanonicalSourceKind::StageEpisode =>
                        {
                            *episode_id
                        }
                        _ => {
                            return Err(MemoryError::Policy(
                                "memory_episode_event_source_mismatch".to_string(),
                            ));
                        }
                    };
                    let episode = stage_episodes::get(self.pool(), episode_id)
                        .await
                        .map_err(sqlx_error)?
                        .into_domain()
                        .map_err(|error| MemoryError::Port(error.code().to_string()))?;
                    Some(assertion_from_stage_episode(&event, &episode)?)
                }
                KnowledgeEventNameV1::CandidateAttemptTerminal => {
                    match self.assertion_from_candidate_terminal(&event).await? {
                        Some(assertion) => Some(assertion),
                        None => {
                            return Ok(Some(CANDIDATE_REASON_ONLY_BLOCKED_SUPPRESSION.to_string()));
                        }
                    }
                }
                KnowledgeEventNameV1::PostExploitActionPrepared => {
                    Some(self.assertion_from_prepared_action(&event).await?)
                }
                KnowledgeEventNameV1::PostExploitFactTerminal => {
                    Some(self.assertion_from_post_exploit_fact(&event).await?)
                }
                KnowledgeEventNameV1::CleanupObligationTerminal => {
                    Some(self.assertion_from_cleanup_terminal(&event).await?)
                }
                KnowledgeEventNameV1::FactDeltaAccepted
                | KnowledgeEventNameV1::SourceScopeInvalidated => None,
                KnowledgeEventNameV1::ReportRevisionFinalized => {
                    return Err(MemoryError::Policy(
                        "memory_event_has_no_projection_route".to_string(),
                    ));
                }
            };
            let assertions = if let Some(assertion) = derived_assertion {
                let mut tx = self.pool.begin().await.map_err(sqlx_error)?;
                let stored = knowledge_assertions::insert_with_connection(&mut tx, &assertion)
                    .await
                    .map_err(assertion_error)?;
                tx.commit().await.map_err(sqlx_error)?;
                vec![stored]
            } else {
                knowledge_assertions::list_for_event_source(self.pool(), &event)
                    .await
                    .map_err(assertion_error)?
            };
            if assertions.is_empty() {
                let reason = match assertion_promotion_policy(event.event_name) {
                    AssertionPromotionPolicy::RequiresProducerAssertion => {
                        "memory_fact_delta_producer_assertion_missing"
                    }
                    AssertionPromotionPolicy::ProducerPreclosedInvalidation => {
                        "memory_invalidation_source_assertion_missing"
                    }
                    AssertionPromotionPolicy::DeriveFromCanonicalEvent => {
                        "memory_canonical_event_assertion_derivation_missing"
                    }
                    AssertionPromotionPolicy::DeriveCandidateOrSuppressReasonOnlyBlocked => {
                        "memory_candidate_terminal_assertion_derivation_missing"
                    }
                    AssertionPromotionPolicy::NoProjectionRoute => {
                        "memory_event_has_no_projection_route"
                    }
                };
                return Err(MemoryError::Policy(reason.to_string()));
            }
            for assertion in &assertions {
                validate_assertion_event_lineage(&event, assertion)?;
            }
            Ok(None)
        }
        .await;
        self.complete_claimed(
            event_id,
            ProjectorId::AssertionPromoterV1,
            worker_id,
            result,
        )
        .await
    }

    async fn assertion_from_candidate_terminal(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<Option<KnowledgeAssertion>, MemoryError> {
        let payload: CandidateTerminalPayload = parse_structured_payload(
            event,
            KnowledgeEventNameV1::CandidateAttemptTerminal,
            "memory_candidate_terminal_event_payload_invalid",
        )?;
        let source_id = exact_source_uuid(
            event,
            CanonicalSourceKind::CandidateAttempt,
            "memory_candidate_terminal_event_source_mismatch",
        )?;
        let authority = self.scoped_event_authority(event).await?;
        let (kind, evidence_ids) = match payload.projection_decision(source_id)? {
            CandidateProjectionDecision::Project { kind, evidence_ids } => (kind, evidence_ids),
            CandidateProjectionDecision::SuppressReasonOnlyBlocked => return Ok(None),
        };
        self.assertion_from_scoped_event_with_authority(
            event,
            authority,
            format!("candidate_attempt:{source_id}"),
            "candidate_attempt_terminal",
            kind,
            evidence_ids,
        )
        .map(Some)
    }

    async fn assertion_from_post_exploit_fact(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<KnowledgeAssertion, MemoryError> {
        let payload: PostExploitFactPayload = parse_structured_payload(
            event,
            KnowledgeEventNameV1::PostExploitFactTerminal,
            "memory_post_exploit_fact_event_payload_invalid",
        )?;
        let (source_kind, source_id, subject_key, kind, evidence_ids) = match &payload {
            PostExploitFactPayload::Foothold {
                foothold_id,
                candidate_source,
                target_type_at_time,
                target_value_at_time,
                target_identity_hash,
                evidence_ids,
            } => {
                validate_bounded_text(candidate_source, 128)?;
                validate_bounded_text(target_type_at_time, 128)?;
                validate_bounded_text(target_value_at_time, 4096)?;
                validate_bounded_text(target_identity_hash, 256)?;
                (
                    CanonicalSourceKind::Foothold,
                    *foothold_id,
                    format!("foothold:{foothold_id}"),
                    AssertionKind::VerifiedOutcome,
                    exact_evidence_ids(evidence_ids)?,
                )
            }
            PostExploitFactPayload::ObjectiveOutcome {
                objective_attempt_id,
                attack_path_id,
                objective_kind,
                outcome,
                simulation_plan_hash,
                evidence_ids,
            } => {
                if attack_path_id.is_some_and(|id| id.is_nil()) {
                    return Err(MemoryError::Policy(
                        "memory_post_exploit_fact_event_payload_invalid".to_string(),
                    ));
                }
                validate_bounded_text(objective_kind, 128)?;
                validate_bounded_text(simulation_plan_hash, 256)?;
                let kind = match outcome.as_str() {
                    "simulated_achievable" => AssertionKind::VerifiedOutcome,
                    "simulated_blocked" | "insufficient_evidence" => AssertionKind::RefutedOutcome,
                    _ => {
                        return Err(MemoryError::Policy(
                            "memory_post_exploit_fact_outcome_invalid".to_string(),
                        ));
                    }
                };
                (
                    CanonicalSourceKind::ObjectiveOutcome,
                    *objective_attempt_id,
                    format!("objective_outcome:{objective_attempt_id}"),
                    kind,
                    exact_evidence_ids(evidence_ids)?,
                )
            }
        };
        let exact_source_id = exact_source_uuid(
            event,
            source_kind,
            "memory_post_exploit_fact_event_source_mismatch",
        )?;
        if source_id.is_nil() || source_id != exact_source_id {
            return Err(MemoryError::Policy(
                "memory_post_exploit_fact_event_source_mismatch".to_string(),
            ));
        }
        self.assertion_from_scoped_event(
            event,
            subject_key,
            "post_exploit_fact_terminal",
            kind,
            evidence_ids,
        )
        .await
    }

    async fn assertion_from_cleanup_terminal(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<KnowledgeAssertion, MemoryError> {
        let payload: CleanupTerminalPayload = parse_structured_payload(
            event,
            KnowledgeEventNameV1::CleanupObligationTerminal,
            "memory_cleanup_terminal_event_payload_invalid",
        )?;
        let source_id = exact_source_uuid(
            event,
            CanonicalSourceKind::CleanupObligation,
            "memory_cleanup_terminal_event_source_mismatch",
        )?;
        let (kind, evidence_ids) = payload.projection_decision(source_id)?;
        self.assertion_from_scoped_event(
            event,
            format!("cleanup_obligation:{source_id}"),
            "cleanup_obligation_terminal",
            kind,
            evidence_ids,
        )
        .await
    }

    async fn assertion_from_scoped_event(
        &self,
        event: &KnowledgeEventEnvelopeV1,
        source_subject: String,
        payload_key: &str,
        kind: AssertionKind,
        evidence_ids: Vec<i64>,
    ) -> Result<KnowledgeAssertion, MemoryError> {
        let authority = self.scoped_event_authority(event).await?;
        self.assertion_from_scoped_event_with_authority(
            event,
            authority,
            source_subject,
            payload_key,
            kind,
            evidence_ids,
        )
    }

    async fn scoped_event_authority(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<ScopedEventAuthority, MemoryError> {
        event
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        let project_scope_id = event.project_scope_id.ok_or_else(|| {
            MemoryError::Policy("memory_canonical_event_project_scope_missing".to_string())
        })?;
        let organization_id_at_time = event.organization_id_at_time.ok_or_else(|| {
            MemoryError::Policy("memory_canonical_event_organization_missing".to_string())
        })?;
        let source_scope_snapshot_hash = sqlx::query_scalar::<_, String>(
            r#"SELECT snapshot.scope_hash
                 FROM operation_org_scope_snapshots AS snapshot
                 JOIN operation_org_scope_units AS unit
                   ON unit.snapshot_id=snapshot.id
                  AND unit.organization_id=$3
                WHERE snapshot.operation_id=$1
                  AND snapshot.project_scope_id=$2
                  AND snapshot.sealed_at IS NOT NULL"#,
        )
        .bind(event.source_operation_id)
        .bind(project_scope_id.0)
        .bind(organization_id_at_time)
        .fetch_optional(self.pool())
        .await
        .map_err(sqlx_error)?
        .ok_or_else(|| {
            MemoryError::Policy("memory_canonical_event_frozen_scope_mismatch".to_string())
        })?;

        Ok(ScopedEventAuthority {
            project_scope_id,
            organization_id_at_time,
            source_scope_snapshot_hash,
        })
    }

    fn assertion_from_scoped_event_with_authority(
        &self,
        event: &KnowledgeEventEnvelopeV1,
        authority: ScopedEventAuthority,
        source_subject: String,
        payload_key: &str,
        kind: AssertionKind,
        evidence_ids: Vec<i64>,
    ) -> Result<KnowledgeAssertion, MemoryError> {
        let ScopedEventAuthority {
            project_scope_id,
            organization_id_at_time,
            source_scope_snapshot_hash,
        } = authority;

        let mut object = serde_json::Map::new();
        object.insert(
            "canonical_ref".to_string(),
            serde_json::Value::String(format!("organization:{organization_id_at_time}")),
        );
        object.insert(
            "display_name".to_string(),
            serde_json::Value::String(organization_id_at_time.to_string()),
        );
        object.insert(
            "properties".to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
        object.insert(
            payload_key.to_string(),
            event.payload.structured_payload.clone(),
        );
        let object = AssertionObject::Json(serde_json::Value::Object(object));
        let identity = AssertionIdentity::derive(
            format!("organization:{organization_id_at_time}:{source_subject}"),
            "graph.entity.organization",
            &object,
        )
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        KnowledgeAssertionDraft {
            assertion_id: Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!(
                    "canonical_event_assertion_v1:{}:{}",
                    event.event_id, identity.identity_hash
                )
                .as_bytes(),
            ),
            visibility: AssertionVisibility::OrganizationLongTerm {
                project_scope_id,
                organization_id_at_time,
            },
            source_operation_id: event.source_operation_id,
            source_scope_snapshot_hash,
            source: event.payload.source.clone(),
            identity,
            kind,
            status: AssertionStatus::Active,
            object,
            classification: KnowledgeClassification::CustomerConfidential,
            evidence_ids,
            valid_from: event.occurred_at,
            valid_to: None,
            fresh_until: None,
        }
        .validate()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))
    }

    async fn assertion_from_prepared_action(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<KnowledgeAssertion, MemoryError> {
        event
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        let action_id = match &event.payload.source.row_id {
            CanonicalRowId::Uuid(action_id)
                if event.event_name == KnowledgeEventNameV1::PostExploitActionPrepared
                    && event.payload.source.source_kind
                        == CanonicalSourceKind::PostExploitAction =>
            {
                *action_id
            }
            _ => {
                return Err(MemoryError::Policy(
                    "memory_post_exploit_action_event_source_mismatch".to_string(),
                ));
            }
        };
        let payload: PreparedActionPayload =
            serde_json::from_value(event.payload.structured_payload.clone()).map_err(|_| {
                MemoryError::Policy("memory_post_exploit_action_event_payload_invalid".to_string())
            })?;
        let action = post_exploit_actions::get(self.pool(), action_id)
            .await
            .map_err(|_| {
                MemoryError::Port("memory_post_exploit_action_database_error".to_string())
            })?
            .ok_or_else(|| MemoryError::Policy("memory_post_exploit_action_missing".to_string()))?;
        let obligation = cleanup_obligations::get(self.pool(), payload.obligation_id)
            .await
            .map_err(|_| MemoryError::Port("memory_cleanup_database_error".to_string()))?
            .ok_or_else(|| MemoryError::Policy("memory_cleanup_obligation_missing".to_string()))?;
        let relation_is_exact: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1
                     FROM post_exploit_actions AS action
                     JOIN cleanup_obligations AS obligation
                       ON obligation.id=action.cleanup_obligation_id
                      AND obligation.source_action_id=action.id
                    WHERE action.id=$1 AND obligation.id=$2
               )"#,
        )
        .bind(action.id)
        .bind(obligation.id)
        .fetch_one(self.pool())
        .await
        .map_err(sqlx_error)?;
        if !relation_is_exact
            || payload.action_id != action.id
            || payload.capability != action.capability_id
            || payload.side_effect_class != action.side_effect_class
            || payload.plan_hash != action.plan_hash
            || payload.resource_identity_hash != obligation.resource_identity_hash
            || obligation.source_action_plan_hash != action.plan_hash
            || obligation.operation_id != action.operation_id
            || obligation.project_scope_id != action.project_scope_id
            || obligation.scope_snapshot_id != action.scope_snapshot_id
            || obligation.organization_id_at_time != action.organization_id_at_time
            || event.project_scope_id
                != Some(golish_memory_domain::ProjectScopeId(
                    action.project_scope_id,
                ))
            || event.organization_id_at_time != Some(action.organization_id_at_time)
            || event.source_operation_id != action.operation_id
        {
            return Err(MemoryError::Policy(
                "memory_post_exploit_action_event_source_mismatch".to_string(),
            ));
        }

        let mut evidence_ids = sqlx::query_scalar::<_, i64>(
            r#"SELECT evidence_id FROM post_exploit_action_evidence WHERE action_id=$1
               UNION
               SELECT evidence_id FROM cleanup_obligation_evidence WHERE obligation_id=$2
               ORDER BY 1"#,
        )
        .bind(action.id)
        .bind(obligation.id)
        .fetch_all(self.pool())
        .await
        .map_err(sqlx_error)?;
        evidence_ids.sort_unstable();
        let mut payload_evidence = payload.evidence_ids.clone();
        payload_evidence.sort_unstable();
        payload_evidence.dedup();
        if evidence_ids.is_empty()
            || evidence_ids != payload_evidence
            || payload.evidence_ids != payload_evidence
        {
            return Err(MemoryError::Policy(
                "memory_post_exploit_action_event_evidence_mismatch".to_string(),
            ));
        }
        let scope_snapshot_hash = sqlx::query_scalar::<_, String>(
            r#"SELECT snapshot.scope_hash
                 FROM operation_org_scope_snapshots AS snapshot
                 JOIN operation_org_scope_units AS unit
                   ON unit.snapshot_id=snapshot.id
                  AND unit.organization_id=$4
                WHERE snapshot.id=$1 AND snapshot.operation_id=$2
                  AND snapshot.project_scope_id=$3 AND snapshot.sealed_at IS NOT NULL"#,
        )
        .bind(action.scope_snapshot_id)
        .bind(action.operation_id)
        .bind(action.project_scope_id)
        .bind(action.organization_id_at_time)
        .fetch_one(self.pool())
        .await
        .map_err(sqlx_error)?;
        let object = AssertionObject::Json(serde_json::json!({
            "canonical_ref": format!("organization:{}", action.organization_id_at_time),
            "display_name": action.organization_id_at_time.to_string(),
            "properties": {},
            "prepared_action": event.payload.structured_payload.clone(),
        }));
        let identity = AssertionIdentity::derive(
            format!("post_exploit_action:{}", action.id),
            "graph.entity.organization",
            &object,
        )
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        KnowledgeAssertionDraft {
            assertion_id: Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!(
                    "post_exploit_action_assertion_v1:{}:{}",
                    event.event_id, identity.identity_hash
                )
                .as_bytes(),
            ),
            visibility: AssertionVisibility::OrganizationLongTerm {
                project_scope_id: golish_memory_domain::ProjectScopeId(action.project_scope_id),
                organization_id_at_time: action.organization_id_at_time,
            },
            source_operation_id: action.operation_id,
            source_scope_snapshot_hash: scope_snapshot_hash,
            source: event.payload.source.clone(),
            identity,
            kind: AssertionKind::Observation,
            status: AssertionStatus::Active,
            object,
            classification: KnowledgeClassification::CustomerConfidential,
            evidence_ids,
            valid_from: action.created_at,
            valid_to: None,
            fresh_until: None,
        }
        .validate()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))
    }

    async fn run_document_once(&self, worker_id: &str) -> Result<ProjectorRunState, MemoryError> {
        let Some(event) = self
            .claim_event(ProjectorId::DocumentProjectorV1, worker_id)
            .await?
        else {
            return Ok(ProjectorRunState::Idle);
        };
        let event_id = event.event_id;
        let result = if event.event_name == KnowledgeEventNameV1::SourceScopeInvalidated {
            Ok(Some("memory_source_invalidated".to_string()))
        } else {
            match DocumentProjector::new(REDACTION_POLICY_VERSION_V1)
                .project(self, event_id)
                .await
            {
                Ok(_) => Ok(None),
                Err(MemoryError::NoPromotedAssertions) => {
                    Ok(Some("memory_document_assertions_missing".to_string()))
                }
                Err(error) => Err(error),
            }
        };
        self.complete_claimed(
            event_id,
            ProjectorId::DocumentProjectorV1,
            worker_id,
            result,
        )
        .await
    }

    async fn run_embedding_once(&self, worker_id: &str) -> Result<ProjectorRunState, MemoryError> {
        let Some(event) = self
            .claim_event(ProjectorId::EmbeddingProjectorV1, worker_id)
            .await?
        else {
            return Ok(ProjectorRunState::Idle);
        };
        let event_id = event.event_id;
        let result = self.project_embeddings(&event).await;
        self.complete_claimed(
            event_id,
            ProjectorId::EmbeddingProjectorV1,
            worker_id,
            result,
        )
        .await
    }

    async fn project_embeddings(
        &self,
        event: &KnowledgeEventEnvelopeV1,
    ) -> Result<Option<String>, MemoryError> {
        if event.event_name == KnowledgeEventNameV1::SourceScopeInvalidated {
            return Ok(Some("memory_source_invalidated".to_string()));
        }
        match knowledge_outbox::get_delivery_status(
            self.pool(),
            event.event_id,
            ProjectorId::DocumentProjectorV1,
        )
        .await
        .map_err(outbox_error)?
        {
            Some(knowledge_outbox::DeliveryStatus::Succeeded) => {}
            Some(_) => {
                return Ok(Some(
                    "memory_embedding_document_delivery_not_succeeded".to_string(),
                ));
            }
            None => {
                return Err(MemoryError::Policy(
                    "memory_embedding_document_delivery_missing".to_string(),
                ));
            }
        }
        let Some(provider) = self.embedding.as_ref() else {
            return Ok(Some("memory_embedding_provider_unconfigured".to_string()));
        };
        if provider.embedder.dimension() != EMBEDDING_DIMENSION_V1 {
            return Err(MemoryError::Policy(
                "memory_embedding_provider_dimension_mismatch".to_string(),
            ));
        }
        let documents = knowledge_documents::list_active_for_source(
            self.pool(),
            event.project_scope_id.map(|id| id.0),
            &event.payload.source_stream_key,
            event.payload.source_version,
        )
        .await
        .map_err(sqlx_error)?;
        if documents.is_empty() {
            return Ok(Some("memory_embedding_document_missing".to_string()));
        }
        if documents
            .iter()
            .any(|document| document.classification == "restricted")
        {
            return Ok(Some(
                "memory_embedding_restricted_classification".to_string(),
            ));
        }
        for document in documents {
            let embedding = provider
                .embedder
                .embed(&document.redacted_content)
                .await
                .map_err(|_| MemoryError::Port("memory_embedding_provider_failed".to_string()))?;
            if embedding.len() != EMBEDDING_DIMENSION_V1
                || embedding.iter().any(|value| !value.is_finite())
            {
                return Err(MemoryError::Policy(
                    "memory_embedding_provider_result_invalid".to_string(),
                ));
            }
            let identity = format!(
                "{}\0{}\0{}\0{}",
                document.document_id,
                provider.provider_name,
                provider.embedder.model_name(),
                document.content_hash
            );
            let input = knowledge_embeddings::InsertKnowledgeEmbedding {
                embedding_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes()),
                document_id: document.document_id,
                source_stream_key: document.source_stream_key,
                source_version: document.source_version,
                provider: provider.provider_name.clone(),
                model: provider.embedder.model_name().to_string(),
                embedding,
                content_hash: document.content_hash,
                valid_from: document.valid_from,
                valid_to: document.valid_to,
            };
            knowledge_embeddings::insert(self.pool(), &input)
                .await
                .map_err(|error| MemoryError::Port(error.code().to_string()))?;
        }
        Ok(None)
    }

    async fn run_graph_once(&self, worker_id: &str) -> Result<ProjectorRunState, MemoryError> {
        let projector = GraphProjector::new(self, self, &self.graph, worker_id)?;
        match projector.run_once().await? {
            GraphProjectorTick::NoWork => Ok(ProjectorRunState::Idle),
            GraphProjectorTick::Succeeded { .. }
            | GraphProjectorTick::SucceededSuppressed { .. } => Ok(ProjectorRunState::Processed),
        }
    }
}

/// Process-level runtime shared by desktop and every CLI session. Construction
/// is side-effect free; only the composition root calls `start` after DB-ready.
#[derive(Clone)]
pub struct KnowledgeMemoryRuntime {
    adapter: Arc<PgKnowledgeMemory>,
    supervisor: KnowledgeProjectorSupervisor,
    query_embedding: Option<Arc<dyn QueryEmbeddingProvider>>,
}

impl KnowledgeMemoryRuntime {
    pub fn new(pool: Arc<PgPool>, embedding: Option<KnowledgeEmbeddingProvider>) -> Self {
        let query_embedding = embedding
            .clone()
            .map(|provider| Arc::new(provider) as Arc<dyn QueryEmbeddingProvider>);
        let adapter = Arc::new(PgKnowledgeMemory::new(pool, embedding));
        let process_id = Uuid::new_v4();
        let workers = [
            ProjectorId::AssertionPromoterV1,
            ProjectorId::DocumentProjectorV1,
            ProjectorId::EmbeddingProjectorV1,
            ProjectorId::GraphProjectorV1,
        ]
        .into_iter()
        .map(|projector| {
            Arc::new(PgKnowledgeProjectorWorker {
                adapter: adapter.clone(),
                projector,
                worker_id: format!(
                    "memory-{}-{}-{}",
                    std::process::id(),
                    process_id.simple(),
                    projector.name()
                ),
            }) as Arc<dyn KnowledgeProjectorWorker>
        })
        .collect();
        let supervisor = KnowledgeProjectorSupervisor::new(workers)
            .expect("fixed Memory Fabric projector ids are unique");
        Self {
            adapter,
            supervisor,
            query_embedding,
        }
    }

    pub fn from_settings(pool: Arc<PgPool>, settings: &golish_settings::GolishSettings) -> Self {
        let embedding = settings
            .ai
            .ollama
            .embedding_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .and_then(|model| {
                match golish_db::embeddings::HttpEmbedder::local_openai_compatible(
                    &settings.ai.ollama.base_url,
                    model,
                    EMBEDDING_DIMENSION_V1,
                ) {
                    Ok(embedder) => KnowledgeEmbeddingProvider::new(
                        "ollama-loopback",
                        Arc::new(embedder) as Arc<dyn Embedder>,
                    )
                    .ok(),
                    Err(error) => {
                        tracing::warn!(
                            target: "harness::knowledge_memory",
                            %error,
                            "local embedding configuration rejected; vector projection remains disabled"
                        );
                        None
                    }
                }
            });
        Self::new(pool, embedding)
    }

    pub fn adapter(&self) -> Arc<PgKnowledgeMemory> {
        self.adapter.clone()
    }

    pub fn unit_of_work(&self) -> Arc<dyn KnowledgeUnitOfWork> {
        self.adapter.clone()
    }

    pub fn query_embedding_provider(&self) -> Option<Arc<dyn QueryEmbeddingProvider>> {
        self.query_embedding.clone()
    }

    pub async fn start(&self) -> Result<SupervisorStartOutcome, MemoryError> {
        self.supervisor.start().await
    }

    pub async fn shutdown(&self) -> Result<(), MemoryError> {
        self.supervisor.shutdown().await
    }

    pub fn is_running(&self) -> bool {
        self.supervisor.is_running()
    }

    pub fn owner_count(&self) -> usize {
        self.supervisor.owner_count()
    }

    pub fn start_count(&self) -> u64 {
        self.supervisor.start_count()
    }
}

struct PgKnowledgeProjectorWorker {
    adapter: Arc<PgKnowledgeMemory>,
    projector: ProjectorId,
    worker_id: String,
}

#[async_trait]
impl KnowledgeProjectorWorker for PgKnowledgeProjectorWorker {
    fn projector_id(&self) -> ProjectorId {
        self.projector
    }

    async fn register(&self) -> Result<(), MemoryError> {
        knowledge_outbox::activate_paused_projector(self.adapter.pool(), self.projector)
            .await
            .map_err(outbox_error)?;
        if self.projector == ProjectorId::EmbeddingProjectorV1 && self.adapter.embedding.is_some() {
            let requeued =
                knowledge_outbox::requeue_provider_unconfigured_embeddings(self.adapter.pool())
                    .await
                    .map_err(outbox_error)?;
            if requeued > 0 {
                tracing::info!(
                    target: "harness::knowledge_memory",
                    requeued,
                    "requeued embeddings suppressed before the local provider was configured"
                );
            }
        }
        Ok(())
    }

    async fn run_once(&self) -> Result<ProjectorRunState, MemoryError> {
        match self.projector {
            ProjectorId::AssertionPromoterV1 => {
                self.adapter.run_assertion_once(&self.worker_id).await
            }
            ProjectorId::DocumentProjectorV1 => {
                self.adapter.run_document_once(&self.worker_id).await
            }
            ProjectorId::EmbeddingProjectorV1 => {
                self.adapter.run_embedding_once(&self.worker_id).await
            }
            ProjectorId::GraphProjectorV1 => self.adapter.run_graph_once(&self.worker_id).await,
            ProjectorId::ReportArtifactIndexerV1 => Err(MemoryError::Policy(
                "memory_report_projector_not_owned".to_string(),
            )),
        }
    }
}

#[async_trait]
impl KnowledgeUnitOfWork for PgKnowledgeMemory {
    async fn close_episode_and_emit(
        &self,
        command: CloseEpisodeAndEmit,
    ) -> Result<Uuid, MemoryError> {
        let mut tx = self.pool.begin().await.map_err(sqlx_error)?;
        let event_id = close_episode_and_emit_in_transaction(&mut tx, &command).await?;
        tx.commit().await.map_err(sqlx_error)?;
        Ok(event_id)
    }

    async fn promote_assertion_and_emit(
        &self,
        command: PromoteAssertionAndEmit,
    ) -> Result<Uuid, MemoryError> {
        let mut tx = self.pool.begin().await.map_err(sqlx_error)?;
        let event_id = promote_assertion_and_emit_in_transaction(&mut tx, &command).await?;
        tx.commit().await.map_err(sqlx_error)?;
        Ok(event_id)
    }

    async fn invalidate_projection_chain_and_emit(
        &self,
        command: InvalidateProjectionChainAndEmit,
    ) -> Result<(), MemoryError> {
        if command.reason_code.trim().is_empty() {
            return Err(MemoryError::Policy(
                "memory_invalidation_reason_empty".to_string(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(sqlx_error)?;
        invalidate_projection_chain_and_emit_in_transaction(&mut tx, &command).await?;
        tx.commit().await.map_err(sqlx_error)
    }
}

/// Inner transaction seam for P1 Task 9. The caller owns commit/rollback and
/// must invoke this inside the final Unit/Handoff compound transaction.
pub async fn close_episode_and_emit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &CloseEpisodeAndEmit,
) -> Result<Uuid, MemoryError> {
    stage_episodes::close_episode_with_event(tx, &command.episode, &command.event)
        .await
        .map_err(|error| MemoryError::Port(error.code().to_string()))
}

/// Inner transaction seam for P2 Task 8 and later canonical producers. It must
/// be called before the producer's final compound transaction commits.
pub async fn promote_assertion_and_emit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &PromoteAssertionAndEmit,
) -> Result<Uuid, MemoryError> {
    knowledge_assertions::promote_assertion_with_event_with_connection(
        tx,
        &command.assertion,
        &command.event,
    )
    .await
    .map_err(assertion_error)
}

pub async fn invalidate_projection_chain_and_emit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &InvalidateProjectionChainAndEmit,
) -> Result<Uuid, MemoryError> {
    knowledge_assertions::invalidate_projection_chain_with_event_with_connection(
        tx,
        &command.source,
        command.invalidated_at,
        &command.event,
    )
    .await
    .map_err(assertion_error)
}

#[async_trait]
impl DocumentProjectionPort for PgKnowledgeMemory {
    async fn load_promoted_assertions(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        let event = knowledge_outbox::get_event(self.pool(), event_id)
            .await
            .map_err(outbox_error)?;
        knowledge_assertions::list_for_event_source(self.pool(), &event)
            .await
            .map_err(assertion_error)
    }

    async fn upsert_document(&self, document: ProjectedDocument) -> Result<Uuid, MemoryError> {
        if document.projection_schema_version != DOCUMENT_PROJECTION_SCHEMA_V1 {
            return Err(MemoryError::Policy(
                "memory_document_projection_schema_invalid".to_string(),
            ));
        }
        let input = knowledge_documents::UpsertKnowledgeDocument {
            document_id: document.document_id,
            document_key: document.document_key,
            project_scope_id: document.project_scope_id.map(|id| id.0),
            source_stream_key: document.source_stream_key,
            source_version: document.source_version,
            projection_schema_version: document.projection_schema_version,
            redaction_policy_version: document.redaction_policy_version,
            assertion_ids: document.assertion_ids,
            document_type: "assertion_document_v1".to_string(),
            redacted_content: document.redacted_content,
            content_hash: document.content_hash,
            classification: document.classification.as_str().to_string(),
            valid_from: document.valid_from,
            valid_to: document.valid_to,
        };
        knowledge_documents::upsert(self.pool(), &input)
            .await
            .map(|row| row.document_id)
            .map_err(sqlx_error)
    }
}

#[async_trait]
impl GraphProjectionDeliveryPort for PgKnowledgeMemory {
    async fn claim_graph_delivery(
        &self,
        worker_id: &str,
    ) -> Result<Option<GraphProjectionDelivery>, MemoryError> {
        self.claim_event(ProjectorId::GraphProjectorV1, worker_id)
            .await
            .map(|event| event.map(|event| GraphProjectionDelivery { event }))
    }

    async fn complete_graph_delivery(
        &self,
        event_id: Uuid,
        worker_id: &str,
        outcome: GraphDeliveryOutcome,
    ) -> Result<(), MemoryError> {
        let (status, reason) = match outcome {
            GraphDeliveryOutcome::Succeeded => (knowledge_outbox::DeliveryStatus::Succeeded, None),
            GraphDeliveryOutcome::SucceededSuppressed { reason_code } => (
                knowledge_outbox::DeliveryStatus::SucceededSuppressed,
                Some(reason_code),
            ),
        };
        knowledge_outbox::complete_delivery(
            self.pool(),
            event_id,
            ProjectorId::GraphProjectorV1,
            worker_id,
            status,
            reason.as_deref(),
        )
        .await
        .map_err(outbox_error)
    }

    async fn retry_graph_delivery(
        &self,
        event_id: Uuid,
        worker_id: &str,
        error_code: &str,
    ) -> Result<(), MemoryError> {
        knowledge_outbox::fail_delivery(
            self.pool(),
            event_id,
            ProjectorId::GraphProjectorV1,
            worker_id,
            error_code,
            MAX_DELIVERY_ATTEMPTS,
        )
        .await
        .map(|_| ())
        .map_err(outbox_error)
    }
}

#[async_trait]
impl GraphAssertionReader for PgKnowledgeMemory {
    async fn load_promoted_assertions(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        DocumentProjectionPort::load_promoted_assertions(self, event_id).await
    }

    async fn list_active_assertions_for_rebuild(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        let visibility = match scope {
            GraphRebuildScope::Organization {
                project_scope_id,
                organization_id_at_time,
            } => AssertionVisibility::OrganizationLongTerm {
                project_scope_id: *project_scope_id,
                organization_id_at_time: *organization_id_at_time,
            },
            GraphRebuildScope::GlobalSanitized => AssertionVisibility::GlobalSanitized,
        };
        knowledge_assertions::list_active_for_visibility(self.pool(), &visibility)
            .await
            .map_err(assertion_error)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedActionPayload {
    action_id: Uuid,
    obligation_id: Uuid,
    capability: String,
    side_effect_class: String,
    plan_hash: String,
    resource_identity_hash: String,
    evidence_ids: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateTerminalPayload {
    attempt_id: Uuid,
    candidate_id: Uuid,
    approval_id: Uuid,
    disposition: String,
    candidate_plan_hash: String,
    result_hash: String,
    finding_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    target_identity_hash: String,
    evidence_ids: Vec<i64>,
    proof_evidence_ids: Vec<i64>,
    refutation_evidence_ids: Vec<i64>,
    blocker_evidence_ids: Vec<i64>,
    blocker_reason_code: Option<String>,
    fact_delta_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CandidateProjectionDecision {
    Project {
        kind: AssertionKind,
        evidence_ids: Vec<i64>,
    },
    SuppressReasonOnlyBlocked,
}

impl CandidateTerminalPayload {
    fn projection_decision(
        &self,
        source_attempt_id: Uuid,
    ) -> Result<CandidateProjectionDecision, MemoryError> {
        if self.attempt_id.is_nil()
            || self.attempt_id != source_attempt_id
            || self.candidate_id.is_nil()
            || self.approval_id.is_nil()
            || self.finding_id.is_some_and(|id| id.is_nil())
            || self.fact_delta_count > 4096
        {
            return Err(MemoryError::Policy(
                "memory_candidate_terminal_event_payload_invalid".to_string(),
            ));
        }
        validate_bounded_text(&self.candidate_plan_hash, 512)?;
        validate_bounded_text(&self.result_hash, 512)?;
        validate_bounded_text(&self.target_type_at_time, 128)?;
        validate_bounded_text(&self.target_value_at_time, 4096)?;
        validate_bounded_text(&self.target_identity_hash, 256)?;
        if let Some(reason_code) = &self.blocker_reason_code {
            validate_bounded_text(reason_code, 256)?;
        }
        if self.fact_delta_count > 0 {
            return Err(MemoryError::Policy(
                "memory_candidate_terminal_fact_delta_evidence_untyped".to_string(),
            ));
        }
        let all_evidence = exact_evidence_ids_allow_empty(&self.evidence_ids)?;
        let proof_evidence = exact_evidence_ids_allow_empty(&self.proof_evidence_ids)?;
        let refutation_evidence = exact_evidence_ids_allow_empty(&self.refutation_evidence_ids)?;
        let blocker_evidence = exact_evidence_ids_allow_empty(&self.blocker_evidence_ids)?;
        let (kind, selected) = match self.disposition.as_str() {
            "verified"
                if self.finding_id.is_some()
                    && !proof_evidence.is_empty()
                    && refutation_evidence.is_empty()
                    && blocker_evidence.is_empty()
                    && self.blocker_reason_code.is_none() =>
            {
                (AssertionKind::VerifiedOutcome, proof_evidence)
            }
            "refuted"
                if self.finding_id.is_none()
                    && proof_evidence.is_empty()
                    && !refutation_evidence.is_empty()
                    && blocker_evidence.is_empty()
                    && self.blocker_reason_code.is_none() =>
            {
                (AssertionKind::RefutedOutcome, refutation_evidence)
            }
            "blocked"
                if self.finding_id.is_none()
                    && proof_evidence.is_empty()
                    && refutation_evidence.is_empty()
                    && !blocker_evidence.is_empty() =>
            {
                (AssertionKind::Observation, blocker_evidence)
            }
            "blocked"
                if self.finding_id.is_none()
                    && proof_evidence.is_empty()
                    && refutation_evidence.is_empty()
                    && blocker_evidence.is_empty()
                    && self.blocker_reason_code.is_some()
                    && all_evidence.is_empty()
                    && self.fact_delta_count == 0 =>
            {
                return Ok(CandidateProjectionDecision::SuppressReasonOnlyBlocked);
            }
            _ => {
                return Err(MemoryError::Policy(
                    "memory_candidate_terminal_event_payload_invalid".to_string(),
                ));
            }
        };
        if all_evidence.is_empty() || selected != all_evidence {
            return Err(MemoryError::Policy(
                "memory_candidate_terminal_event_evidence_mismatch".to_string(),
            ));
        }
        Ok(CandidateProjectionDecision::Project {
            kind,
            evidence_ids: all_evidence,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "fact_kind", deny_unknown_fields)]
enum PostExploitFactPayload {
    #[serde(rename = "foothold")]
    Foothold {
        foothold_id: Uuid,
        candidate_source: String,
        target_type_at_time: String,
        target_value_at_time: String,
        target_identity_hash: String,
        evidence_ids: Vec<i64>,
    },
    #[serde(rename = "objective_outcome")]
    ObjectiveOutcome {
        objective_attempt_id: Uuid,
        attack_path_id: Option<Uuid>,
        objective_kind: String,
        outcome: String,
        simulation_plan_hash: String,
        evidence_ids: Vec<i64>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "terminal_kind", deny_unknown_fields)]
enum CleanupTerminalPayload {
    #[serde(rename = "independent_absence")]
    IndependentAbsence {
        obligation_id: Uuid,
        terminal_status: String,
        resource_identity_hash: String,
        cleanup_attempt_id: Uuid,
        absence_check_id: Uuid,
        evidence_ids: Vec<i64>,
    },
    #[serde(rename = "operator_waiver")]
    OperatorWaiver {
        obligation_id: Uuid,
        terminal_status: String,
        resource_identity_hash: String,
        waiver_id: Uuid,
        residual_risk: serde_json::Value,
        evidence_ids: Vec<i64>,
    },
}

impl CleanupTerminalPayload {
    fn projection_decision(
        &self,
        source_obligation_id: Uuid,
    ) -> Result<(AssertionKind, Vec<i64>), MemoryError> {
        let (obligation_id, kind, evidence_ids) = match self {
            Self::IndependentAbsence {
                obligation_id,
                terminal_status,
                resource_identity_hash,
                cleanup_attempt_id,
                absence_check_id,
                evidence_ids,
            } => {
                if terminal_status != "verified_absent"
                    || cleanup_attempt_id.is_nil()
                    || absence_check_id.is_nil()
                {
                    return Err(MemoryError::Policy(
                        "memory_cleanup_terminal_event_payload_invalid".to_string(),
                    ));
                }
                validate_bounded_text(resource_identity_hash, 256)?;
                (
                    *obligation_id,
                    AssertionKind::CleanupAttestation,
                    exact_evidence_ids(evidence_ids)?,
                )
            }
            Self::OperatorWaiver {
                obligation_id,
                terminal_status,
                resource_identity_hash,
                waiver_id,
                residual_risk,
                evidence_ids,
            } => {
                if terminal_status != "waived_by_user"
                    || waiver_id.is_nil()
                    || !residual_risk.is_object()
                {
                    return Err(MemoryError::Policy(
                        "memory_cleanup_terminal_event_payload_invalid".to_string(),
                    ));
                }
                validate_bounded_text(resource_identity_hash, 256)?;
                (
                    *obligation_id,
                    AssertionKind::ResidualRisk,
                    exact_evidence_ids(evidence_ids)?,
                )
            }
        };
        if obligation_id.is_nil() || obligation_id != source_obligation_id {
            return Err(MemoryError::Policy(
                "memory_cleanup_terminal_event_source_mismatch".to_string(),
            ));
        }
        Ok((kind, evidence_ids))
    }
}

fn parse_structured_payload<T>(
    event: &KnowledgeEventEnvelopeV1,
    expected_event: KnowledgeEventNameV1,
    error_code: &'static str,
) -> Result<T, MemoryError>
where
    T: for<'de> Deserialize<'de>,
{
    event
        .validate()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    if event.event_name != expected_event {
        return Err(MemoryError::Policy(error_code.to_string()));
    }
    serde_json::from_value(event.payload.structured_payload.clone())
        .map_err(|_| MemoryError::Policy(error_code.to_string()))
}

fn exact_source_uuid(
    event: &KnowledgeEventEnvelopeV1,
    expected_kind: CanonicalSourceKind,
    error_code: &'static str,
) -> Result<Uuid, MemoryError> {
    match &event.payload.source.row_id {
        CanonicalRowId::Uuid(source_id)
            if !source_id.is_nil() && event.payload.source.source_kind == expected_kind =>
        {
            Ok(*source_id)
        }
        _ => Err(MemoryError::Policy(error_code.to_string())),
    }
}

fn exact_evidence_ids(evidence_ids: &[i64]) -> Result<Vec<i64>, MemoryError> {
    let normalized = exact_evidence_ids_allow_empty(evidence_ids)?;
    if normalized.is_empty() {
        return Err(MemoryError::Policy(
            "memory_canonical_event_evidence_invalid".to_string(),
        ));
    }
    Ok(normalized)
}

fn exact_evidence_ids_allow_empty(evidence_ids: &[i64]) -> Result<Vec<i64>, MemoryError> {
    if evidence_ids.iter().any(|id| *id <= 0) {
        return Err(MemoryError::Policy(
            "memory_canonical_event_evidence_invalid".to_string(),
        ));
    }
    let mut normalized = evidence_ids.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.len() != evidence_ids.len() {
        return Err(MemoryError::Policy(
            "memory_canonical_event_evidence_invalid".to_string(),
        ));
    }
    Ok(normalized)
}

fn validate_bounded_text(value: &str, max_bytes: usize) -> Result<(), MemoryError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(MemoryError::Policy(
            "memory_canonical_event_payload_text_invalid".to_string(),
        ));
    }
    Ok(())
}

fn assertion_from_stage_episode(
    event: &KnowledgeEventEnvelopeV1,
    episode: &StageEpisode,
) -> Result<KnowledgeAssertion, MemoryError> {
    event
        .validate()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    if event.event_name != KnowledgeEventNameV1::StageEpisodeClosed
        || event.project_scope_id != Some(episode.scope.project_scope_id)
        || event.source_operation_id != episode.scope.source_operation_id
        || event.organization_id_at_time != Some(episode.scope.organization_id_at_time)
        || event.payload.source.source_kind != CanonicalSourceKind::StageEpisode
        || event.payload.source.row_id != CanonicalRowId::Uuid(episode.episode_id)
    {
        return Err(MemoryError::Policy(
            "memory_episode_event_source_mismatch".to_string(),
        ));
    }
    if episode.evidence_ids.is_empty() {
        return Err(MemoryError::Policy(
            "memory_assertion_evidence_missing".to_string(),
        ));
    }

    let object = AssertionObject::Json(serde_json::json!({
        "canonical_ref": format!("organization:{}", episode.scope.organization_id_at_time),
        "display_name": episode.scope.organization_id_at_time.to_string(),
        "properties": {},
        "stage_episode": {
            "stage_execution_id": episode.stage_execution_id,
            "stage_run_unit_id": episode.stage_run_unit_id,
            "worker_run_id": episode.worker_run_id,
            "candidate_attempt_id": episode.candidate_attempt_id,
            "stage_kind": episode.stage_kind,
            "wave": episode.wave,
            "verdict": episode.verdict.as_str(),
            "deliverable_submission_id": episode.deliverable_submission_id,
            "handoff_id": episode.handoff_id,
            "reason_codes": episode.reason_codes,
            "fact_refs": episode.fact_refs,
        }
    }));
    let subject_key = format!(
        "organization:{}:stage:{}",
        episode.scope.organization_id_at_time, episode.stage_kind
    );
    let identity = AssertionIdentity::derive(subject_key, "graph.entity.organization", &object)
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    let assertion_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!(
            "stage_episode_assertion_v1:{}:{}:{}",
            event.event_id, event.payload.source.source_stream_key, identity.identity_hash
        )
        .as_bytes(),
    );
    KnowledgeAssertionDraft {
        assertion_id,
        visibility: AssertionVisibility::OrganizationLongTerm {
            project_scope_id: episode.scope.project_scope_id,
            organization_id_at_time: episode.scope.organization_id_at_time,
        },
        source_operation_id: episode.scope.source_operation_id,
        source_scope_snapshot_hash: episode.scope.scope_snapshot_hash.clone(),
        source: event.payload.source.clone(),
        identity,
        kind: AssertionKind::Observation,
        status: AssertionStatus::Active,
        object,
        classification: KnowledgeClassification::CustomerConfidential,
        evidence_ids: episode.evidence_ids.clone(),
        valid_from: episode.ended_at,
        valid_to: None,
        fresh_until: None,
    }
    .validate()
    .map_err(|error| MemoryError::Policy(error.code().to_string()))
}

fn validate_assertion_event_lineage(
    event: &KnowledgeEventEnvelopeV1,
    assertion: &KnowledgeAssertion,
) -> Result<(), MemoryError> {
    assertion
        .validate_integrity()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    let invalidation = event.event_name == KnowledgeEventNameV1::SourceScopeInvalidated;
    if (invalidation && assertion.status == AssertionStatus::Active)
        || (!invalidation && assertion.status != AssertionStatus::Active)
        || assertion.source_operation_id != event.source_operation_id
        || assertion.source != event.payload.source
    {
        return Err(MemoryError::Policy(
            "memory_projector_assertion_event_mismatch".to_string(),
        ));
    }
    let scope_matches = match &assertion.visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } => {
            event.project_scope_id == Some(*project_scope_id)
                && event.organization_id_at_time == Some(*organization_id_at_time)
        }
        AssertionVisibility::GlobalSanitized => {
            event.project_scope_id.is_none() && event.organization_id_at_time.is_none()
        }
    };
    if !scope_matches {
        return Err(MemoryError::Policy(
            "memory_projector_assertion_scope_mismatch".to_string(),
        ));
    }
    Ok(())
}

fn outbox_error(error: knowledge_outbox::KnowledgeOutboxError) -> MemoryError {
    MemoryError::Port(error.code().to_string())
}

fn assertion_error(error: knowledge_assertions::AssertionRepoError) -> MemoryError {
    MemoryError::Port(error.code().to_string())
}

fn sqlx_error(_: sqlx::Error) -> MemoryError {
    MemoryError::Port("memory_database_error".to_string())
}

#[cfg(test)]
mod static_composition_tests {
    use std::sync::Arc;

    use chrono::{TimeZone, Utc};
    use golish_memory_domain::event_catalog::{routes_for, KnowledgeEventPayloadV1, ProjectorId};
    use golish_memory_domain::scope::{OperationScope, ProjectScopeId};
    use golish_memory_domain::source_ref::{CanonicalRowId, SourceRef};
    use golish_memory_domain::EpisodeVerdict;

    use super::*;

    fn stage_episode_fixture() -> (KnowledgeEventEnvelopeV1, StageEpisode) {
        let project_scope_id = ProjectScopeId(Uuid::from_u128(0x7101));
        let operation_id = Uuid::from_u128(0x7102);
        let organization_id = Uuid::from_u128(0x7103);
        let episode_id = Uuid::from_u128(0x7104);
        let source = golish_memory_domain::SourceRef {
            source_kind: CanonicalSourceKind::StageEpisode,
            row_id: CanonicalRowId::Uuid(episode_id),
            source_stream_key: format!("stage-episode:{episode_id}"),
            version: 1,
        };
        let at = |hour| {
            Utc.with_ymd_and_hms(2026, 7, 13, hour, 0, 0)
                .single()
                .expect("fixed timestamp")
        };
        let episode = StageEpisode {
            episode_id,
            scope: OperationScope {
                project_scope_id,
                source_operation_id: operation_id,
                organization_id_at_time: organization_id,
                scope_snapshot_hash: "fixture-scope-snapshot".to_string(),
            },
            stage_execution_id: Uuid::from_u128(0x7105),
            stage_run_unit_id: Some(Uuid::from_u128(0x7106)),
            worker_run_id: Some(Uuid::from_u128(0x7107)),
            candidate_attempt_id: None,
            stage_kind: "enumeration".to_string(),
            wave: Some(1),
            verdict: EpisodeVerdict::Passed,
            deliverable_submission_id: Some(Uuid::from_u128(0x7108)),
            handoff_id: Some(Uuid::from_u128(0x7109)),
            reason_codes: vec!["gate_passed".to_string()],
            fact_refs: Vec::<SourceRef>::new(),
            evidence_ids: vec![71],
            started_at: at(10),
            ended_at: at(11),
        };
        let event = KnowledgeEventEnvelopeV1 {
            event_id: Uuid::from_u128(0x7110),
            project_scope_id: Some(project_scope_id),
            organization_id_at_time: Some(organization_id),
            source_operation_id: operation_id,
            event_name: KnowledgeEventNameV1::StageEpisodeClosed,
            schema_version: 1,
            payload: KnowledgeEventPayloadV1 {
                source_stream_key: source.source_stream_key.clone(),
                source_version: source.version,
                source,
                structured_payload: serde_json::json!({"schema": "fixture.v1"}),
            },
            occurred_at: at(11),
        };
        (event, episode)
    }

    #[test]
    fn every_catalog_route_has_an_explicit_assertion_authority_policy() {
        for event_name in [
            KnowledgeEventNameV1::StageEpisodeClosed,
            KnowledgeEventNameV1::PostExploitActionPrepared,
            KnowledgeEventNameV1::PostExploitFactTerminal,
            KnowledgeEventNameV1::CleanupObligationTerminal,
        ] {
            assert!(!routes_for(event_name).is_empty());
            assert_eq!(
                assertion_promotion_policy(event_name),
                AssertionPromotionPolicy::DeriveFromCanonicalEvent
            );
        }
        assert!(!routes_for(KnowledgeEventNameV1::CandidateAttemptTerminal).is_empty());
        assert_eq!(
            assertion_promotion_policy(KnowledgeEventNameV1::CandidateAttemptTerminal),
            AssertionPromotionPolicy::DeriveCandidateOrSuppressReasonOnlyBlocked
        );
        assert!(!routes_for(KnowledgeEventNameV1::FactDeltaAccepted).is_empty());
        assert_eq!(
            assertion_promotion_policy(KnowledgeEventNameV1::FactDeltaAccepted),
            AssertionPromotionPolicy::RequiresProducerAssertion,
            "the accepted-delta producer must atomically persist the assertion; the projector cannot invent it"
        );
        assert!(!routes_for(KnowledgeEventNameV1::SourceScopeInvalidated).is_empty());
        assert_eq!(
            assertion_promotion_policy(KnowledgeEventNameV1::SourceScopeInvalidated),
            AssertionPromotionPolicy::ProducerPreclosedInvalidation
        );
        assert!(routes_for(KnowledgeEventNameV1::ReportRevisionFinalized).is_empty());
        assert_eq!(
            assertion_promotion_policy(KnowledgeEventNameV1::ReportRevisionFinalized),
            AssertionPromotionPolicy::NoProjectionRoute
        );
    }

    #[test]
    fn candidate_payload_cannot_override_envelope_scope_or_source_authority() {
        let (mut event, _) = stage_episode_fixture();
        let attempt_id = Uuid::from_u128(0x7120);
        let authoritative_operation_id = event.source_operation_id;
        let authoritative_project_id = event.project_scope_id;
        let authoritative_organization_id = event.organization_id_at_time;
        event.event_name = KnowledgeEventNameV1::CandidateAttemptTerminal;
        event.payload.source = SourceRef {
            source_kind: CanonicalSourceKind::CandidateAttempt,
            row_id: CanonicalRowId::Uuid(attempt_id),
            source_stream_key: format!("candidate-attempt:{attempt_id}"),
            version: 1,
        };
        event.payload.source_stream_key = event.payload.source.source_stream_key.clone();
        event.payload.source_version = event.payload.source.version;
        event.payload.structured_payload = serde_json::json!({
            "attempt_id": attempt_id,
            "candidate_id": Uuid::from_u128(0x7121),
            "approval_id": Uuid::from_u128(0x7122),
            "disposition": "verified",
            "candidate_plan_hash": "candidate-plan",
            "result_hash": "result-hash",
            "finding_id": Uuid::from_u128(0x7123),
            "target_type_at_time": "domain",
            "target_value_at_time": "example.test",
            "target_identity_hash": "target-hash",
            "evidence_ids": [71],
            "proof_evidence_ids": [71],
            "refutation_evidence_ids": [],
            "blocker_evidence_ids": [],
            "fact_delta_count": 0,
            "source_operation_id": Uuid::new_v4(),
            "project_scope_id": Uuid::new_v4(),
            "organization_id_at_time": Uuid::new_v4(),
            "source": {"kind": "candidate_attempt", "value": Uuid::new_v4()},
        });
        let error = parse_structured_payload::<CandidateTerminalPayload>(
            &event,
            KnowledgeEventNameV1::CandidateAttemptTerminal,
            "memory_candidate_terminal_event_payload_invalid",
        )
        .expect_err("payload-selected authority fields must be rejected, not ignored");
        assert_eq!(error.code(), "memory_policy_rejected");
        assert_eq!(event.source_operation_id, authoritative_operation_id);
        assert_eq!(event.project_scope_id, authoritative_project_id);
        assert_eq!(event.organization_id_at_time, authoritative_organization_id);
        assert_eq!(
            exact_source_uuid(
                &event,
                CanonicalSourceKind::CandidateAttempt,
                "source-mismatch"
            )
            .unwrap(),
            attempt_id
        );
    }

    #[test]
    fn candidate_reason_only_blocked_has_the_only_intentional_suppression_policy() {
        let attempt_id = Uuid::from_u128(0x7130);
        let payload: CandidateTerminalPayload = serde_json::from_value(serde_json::json!({
            "attempt_id": attempt_id,
            "candidate_id": Uuid::from_u128(0x7131),
            "approval_id": Uuid::from_u128(0x7132),
            "disposition": "blocked",
            "candidate_plan_hash": "candidate-plan",
            "result_hash": "result-hash",
            "finding_id": null,
            "target_type_at_time": "domain",
            "target_value_at_time": "example.test",
            "target_identity_hash": "target-hash",
            "evidence_ids": [],
            "proof_evidence_ids": [],
            "refutation_evidence_ids": [],
            "blocker_evidence_ids": [],
            "blocker_reason_code": "approval_expired",
            "fact_delta_count": 0,
        }))
        .expect("strict reason-only Candidate payload");
        assert!(matches!(
            payload
                .projection_decision(attempt_id)
                .expect("reason-only blocked policy"),
            CandidateProjectionDecision::SuppressReasonOnlyBlocked
        ));

        for (label, disposition, finding_id, blocker_reason_code, fact_delta_count) in [
            (
                "verified",
                "verified",
                Some(Uuid::from_u128(0x7133)),
                None,
                0,
            ),
            ("refuted", "refuted", None, None, 0),
            ("blocked without reason", "blocked", None, None, 0),
            (
                "blocked reason with unrepresented fact delta",
                "blocked",
                None,
                Some("approval_expired"),
                1,
            ),
        ] {
            let missing: CandidateTerminalPayload = serde_json::from_value(serde_json::json!({
                "attempt_id": attempt_id,
                "candidate_id": Uuid::from_u128(0x7131),
                "approval_id": Uuid::from_u128(0x7132),
                "disposition": disposition,
                "candidate_plan_hash": "candidate-plan",
                "result_hash": "result-hash",
                "finding_id": finding_id,
                "target_type_at_time": "domain",
                "target_value_at_time": "example.test",
                "target_identity_hash": "target-hash",
                "evidence_ids": [],
                "proof_evidence_ids": [],
                "refutation_evidence_ids": [],
                "blocker_evidence_ids": [],
                "blocker_reason_code": blocker_reason_code,
                "fact_delta_count": fact_delta_count,
            }))
            .expect("strict missing-evidence Candidate payload");
            missing.projection_decision(attempt_id).expect_err(label);
        }
    }

    #[test]
    fn candidate_terminal_does_not_infer_fact_delta_evidence_roles() {
        let attempt_id = Uuid::from_u128(0x7140);
        let payload: CandidateTerminalPayload = serde_json::from_value(serde_json::json!({
            "attempt_id": attempt_id,
            "candidate_id": Uuid::from_u128(0x7141),
            "approval_id": Uuid::from_u128(0x7142),
            "disposition": "verified",
            "candidate_plan_hash": "candidate-plan",
            "result_hash": "result-hash",
            "finding_id": Uuid::from_u128(0x7143),
            "target_type_at_time": "domain",
            "target_value_at_time": "example.test",
            "target_identity_hash": "target-hash",
            "evidence_ids": [71, 72],
            "proof_evidence_ids": [71],
            "refutation_evidence_ids": [],
            "blocker_evidence_ids": [],
            "blocker_reason_code": null,
            "fact_delta_count": 1,
        }))
        .expect("strict Candidate payload with FactDelta evidence");
        let error = payload.projection_decision(attempt_id).expect_err(
            "Candidate terminal evidence cannot stand in for accepted FactDelta authority",
        );
        assert!(error
            .to_string()
            .contains("memory_candidate_terminal_fact_delta_evidence_untyped"));
    }

    #[test]
    fn cleanup_terminal_payload_variants_share_one_strict_projection_policy() {
        let obligation_id = Uuid::from_u128(0x7150);
        let absence: CleanupTerminalPayload = serde_json::from_value(serde_json::json!({
            "terminal_kind": "independent_absence",
            "obligation_id": obligation_id,
            "terminal_status": "verified_absent",
            "resource_identity_hash": "resource-hash",
            "cleanup_attempt_id": Uuid::from_u128(0x7151),
            "absence_check_id": Uuid::from_u128(0x7152),
            "evidence_ids": [81],
        }))
        .expect("strict independent-absence payload");
        assert_eq!(
            absence
                .projection_decision(obligation_id)
                .expect("independent absence projection"),
            (AssertionKind::CleanupAttestation, vec![81])
        );

        let waiver: CleanupTerminalPayload = serde_json::from_value(serde_json::json!({
            "terminal_kind": "operator_waiver",
            "obligation_id": obligation_id,
            "terminal_status": "waived_by_user",
            "resource_identity_hash": "resource-hash",
            "waiver_id": Uuid::from_u128(0x7153),
            "residual_risk": {"severity": "low", "reason": "accepted by operator"},
            "evidence_ids": [82],
        }))
        .expect("strict operator-waiver payload");
        assert_eq!(
            waiver
                .projection_decision(obligation_id)
                .expect("operator waiver projection"),
            (AssertionKind::ResidualRisk, vec![82])
        );

        let malformed: CleanupTerminalPayload = serde_json::from_value(serde_json::json!({
            "terminal_kind": "operator_waiver",
            "obligation_id": obligation_id,
            "terminal_status": "waived_by_user",
            "resource_identity_hash": "resource-hash",
            "waiver_id": Uuid::from_u128(0x7153),
            "residual_risk": null,
            "evidence_ids": [82],
        }))
        .expect("shape-valid but semantically invalid waiver payload");
        malformed
            .projection_decision(obligation_id)
            .expect_err("waiver residual risk must be a structured object");
    }

    #[tokio::test]
    async fn runtime_constructor_does_not_start_workers() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .expect("construct lazy pool without network I/O");
        let runtime = KnowledgeMemoryRuntime::new(Arc::new(pool), None);
        assert!(!runtime.is_running());
        assert_eq!(runtime.start_count(), 0);
        assert_eq!(runtime.owner_count(), 0);
    }

    #[tokio::test]
    async fn runtime_settings_share_one_explicit_loopback_embedding_provider() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .expect("construct lazy pool without network I/O");
        let mut settings = golish_settings::GolishSettings::default();
        let disabled = KnowledgeMemoryRuntime::from_settings(Arc::new(pool.clone()), &settings);
        assert!(disabled.query_embedding_provider().is_none());

        settings.ai.ollama.base_url = "http://127.0.0.1:11434/v1".to_string();
        settings.ai.ollama.embedding_model = Some("qwen3-embedding:4b".to_string());
        let enabled = KnowledgeMemoryRuntime::from_settings(Arc::new(pool.clone()), &settings);
        assert_eq!(
            enabled
                .query_embedding_provider()
                .expect("explicit local provider is shared with query retrieval")
                .dimension(),
            EMBEDDING_DIMENSION_V1
        );

        settings.ai.ollama.base_url = "http://192.0.2.1:11434/v1".to_string();
        let rejected = KnowledgeMemoryRuntime::from_settings(Arc::new(pool), &settings);
        assert!(rejected.query_embedding_provider().is_none());
    }

    #[test]
    fn stage_episode_promoter_derives_one_deterministic_assertion_for_the_full_dag() {
        let (event, episode) = stage_episode_fixture();
        let first = assertion_from_stage_episode(&event, &episode)
            .expect("persisted episode must promote an assertion");
        let second = assertion_from_stage_episode(&event, &episode)
            .expect("replay must derive the same assertion");
        assert_eq!(first, second);
        assert_eq!(first.source, event.payload.source);
        assert_eq!(first.evidence_ids, vec![71]);
        golish_memory_app::project_assertion(&first)
            .expect("episode assertion must remain graph-projectable");
        assert_eq!(
            routes_for(KnowledgeEventNameV1::StageEpisodeClosed),
            vec![
                golish_memory_domain::ProjectorRoute {
                    projector: ProjectorId::AssertionPromoterV1,
                    depends_on: None,
                },
                golish_memory_domain::ProjectorRoute {
                    projector: ProjectorId::DocumentProjectorV1,
                    depends_on: Some(ProjectorId::AssertionPromoterV1),
                },
                golish_memory_domain::ProjectorRoute {
                    projector: ProjectorId::EmbeddingProjectorV1,
                    depends_on: Some(ProjectorId::DocumentProjectorV1),
                },
                golish_memory_domain::ProjectorRoute {
                    projector: ProjectorId::GraphProjectorV1,
                    depends_on: Some(ProjectorId::AssertionPromoterV1),
                },
            ]
        );
    }

    #[test]
    fn stage_episode_promoter_fails_closed_without_evidence() {
        let (event, mut episode) = stage_episode_fixture();
        episode.evidence_ids.clear();
        let error = assertion_from_stage_episode(&event, &episode)
            .expect_err("an evidence-free episode cannot become durable knowledge");
        assert_eq!(error.code(), "memory_policy_rejected");
    }
}
