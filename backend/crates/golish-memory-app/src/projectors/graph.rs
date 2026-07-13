use async_trait::async_trait;
use golish_graphiti::{
    ApplyProjectionResult, GraphGeneration, GraphScopeKey, ProjectionWriteDisposition,
    RebuildAttestation, TemporalGraphClient, TemporalGraphInvalidation, TemporalGraphProjection,
    TEMPORAL_GRAPH_SCHEMA_V1,
};
use golish_memory_domain::assertion::AssertionStatus;
use golish_memory_domain::classification::AssertionVisibility;
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, ProjectorId,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::KnowledgeAssertion;
use uuid::Uuid;

use crate::graph_projection::{project_assertion, project_invalidation, ProjectionError};
use crate::ports::MemoryError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphProjectionDelivery {
    pub event: KnowledgeEventEnvelopeV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphDeliveryOutcome {
    Succeeded,
    SucceededSuppressed { reason_code: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphProjectorTick {
    NoWork,
    Succeeded { event_id: Uuid },
    SucceededSuppressed { event_id: Uuid, reason_code: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphRebuildScope {
    Organization {
        project_scope_id: ProjectScopeId,
        organization_id_at_time: Uuid,
    },
    GlobalSanitized,
}

impl GraphRebuildScope {
    pub fn scope_key(&self) -> GraphScopeKey {
        match self {
            Self::Organization {
                project_scope_id,
                organization_id_at_time,
            } => GraphScopeKey::organization(*project_scope_id, *organization_id_at_time),
            Self::GlobalSanitized => GraphScopeKey::global_sanitized(),
        }
    }

    pub const fn project_scope_id(&self) -> Option<ProjectScopeId> {
        match self {
            Self::Organization {
                project_scope_id, ..
            } => Some(*project_scope_id),
            Self::GlobalSanitized => None,
        }
    }

    pub const fn organization_id_at_time(&self) -> Option<Uuid> {
        match self {
            Self::Organization {
                organization_id_at_time,
                ..
            } => Some(*organization_id_at_time),
            Self::GlobalSanitized => None,
        }
    }
}

#[async_trait]
pub trait GraphProjectionDeliveryPort: Send + Sync {
    /// This seam is intentionally graph-specific: implementations must claim
    /// only `graph-projector@1` deliveries and never consume document or
    /// embedding work.
    async fn claim_graph_delivery(
        &self,
        worker_id: &str,
    ) -> Result<Option<GraphProjectionDelivery>, MemoryError>;

    async fn complete_graph_delivery(
        &self,
        event_id: Uuid,
        worker_id: &str,
        outcome: GraphDeliveryOutcome,
    ) -> Result<(), MemoryError>;

    async fn retry_graph_delivery(
        &self,
        event_id: Uuid,
        worker_id: &str,
        error_code: &str,
    ) -> Result<(), MemoryError>;
}

#[async_trait]
pub trait GraphAssertionReader: Send + Sync {
    async fn load_promoted_assertions(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError>;

    async fn list_active_assertions_for_rebuild(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError>;
}

#[async_trait]
pub trait TemporalGraphProjectionPort: Send + Sync {
    async fn apply_projection(
        &self,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError>;

    async fn apply_projection_to_generation(
        &self,
        generation_id: Uuid,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError>;

    async fn close_assertion_lineage(
        &self,
        invalidation: &TemporalGraphInvalidation,
    ) -> Result<(u64, u64), MemoryError>;

    async fn begin_rebuild(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<GraphGeneration, MemoryError>;

    async fn generation_attestation(
        &self,
        generation_id: Uuid,
    ) -> Result<RebuildAttestation, MemoryError>;

    async fn activate_rebuild(
        &self,
        generation_id: Uuid,
        attestation: &RebuildAttestation,
    ) -> Result<GraphGeneration, MemoryError>;

    async fn fail_rebuild(&self, generation_id: Uuid, reason: &str) -> Result<(), MemoryError>;
}

#[async_trait]
impl TemporalGraphProjectionPort for TemporalGraphClient {
    async fn apply_projection(
        &self,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError> {
        TemporalGraphClient::apply_projection(self, projection)
            .await
            .map_err(graph_port_error)
    }

    async fn apply_projection_to_generation(
        &self,
        generation_id: Uuid,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError> {
        TemporalGraphClient::apply_projection_to_generation(self, generation_id, projection)
            .await
            .map_err(graph_port_error)
    }

    async fn close_assertion_lineage(
        &self,
        invalidation: &TemporalGraphInvalidation,
    ) -> Result<(u64, u64), MemoryError> {
        TemporalGraphClient::close_assertion_lineage(self, invalidation)
            .await
            .map_err(graph_port_error)
    }

    async fn begin_rebuild(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<GraphGeneration, MemoryError> {
        TemporalGraphClient::begin_rebuild(
            self,
            &scope.scope_key(),
            scope.project_scope_id(),
            scope.organization_id_at_time(),
            TEMPORAL_GRAPH_SCHEMA_V1,
        )
        .await
        .map_err(graph_port_error)
    }

    async fn generation_attestation(
        &self,
        generation_id: Uuid,
    ) -> Result<RebuildAttestation, MemoryError> {
        TemporalGraphClient::generation_attestation(self, generation_id)
            .await
            .map_err(graph_port_error)
    }

    async fn activate_rebuild(
        &self,
        generation_id: Uuid,
        attestation: &RebuildAttestation,
    ) -> Result<GraphGeneration, MemoryError> {
        TemporalGraphClient::activate_rebuild(self, generation_id, attestation)
            .await
            .map_err(graph_port_error)
    }

    async fn fail_rebuild(&self, generation_id: Uuid, reason: &str) -> Result<(), MemoryError> {
        TemporalGraphClient::fail_rebuild(self, generation_id, reason)
            .await
            .map_err(graph_port_error)
    }
}

pub struct GraphProjector<'a, D, R, G> {
    deliveries: &'a D,
    assertions: &'a R,
    graph: &'a G,
    worker_id: String,
}

impl<'a, D, R, G> GraphProjector<'a, D, R, G>
where
    D: GraphProjectionDeliveryPort,
    R: GraphAssertionReader,
    G: TemporalGraphProjectionPort,
{
    pub fn new(
        deliveries: &'a D,
        assertions: &'a R,
        graph: &'a G,
        worker_id: impl Into<String>,
    ) -> Result<Self, MemoryError> {
        let worker_id = worker_id.into();
        if worker_id.trim().is_empty() {
            return Err(MemoryError::Policy(
                "knowledge_graph_worker_id_empty".to_string(),
            ));
        }
        Ok(Self {
            deliveries,
            assertions,
            graph,
            worker_id,
        })
    }

    pub const fn projector_id() -> ProjectorId {
        ProjectorId::GraphProjectorV1
    }

    pub async fn run_once(&self) -> Result<GraphProjectorTick, MemoryError> {
        let Some(delivery) = self
            .deliveries
            .claim_graph_delivery(&self.worker_id)
            .await?
        else {
            return Ok(GraphProjectorTick::NoWork);
        };
        let event_id = delivery.event.event_id;
        let result = self.project_delivery(&delivery).await;
        match result {
            Ok(outcome) => {
                self.deliveries
                    .complete_graph_delivery(event_id, &self.worker_id, outcome.clone())
                    .await?;
                Ok(match outcome {
                    GraphDeliveryOutcome::Succeeded => GraphProjectorTick::Succeeded { event_id },
                    GraphDeliveryOutcome::SucceededSuppressed { reason_code } => {
                        GraphProjectorTick::SucceededSuppressed {
                            event_id,
                            reason_code,
                        }
                    }
                })
            }
            Err(error) => {
                self.deliveries
                    .retry_graph_delivery(event_id, &self.worker_id, error.code())
                    .await?;
                Err(error)
            }
        }
    }

    async fn project_delivery(
        &self,
        delivery: &GraphProjectionDelivery,
    ) -> Result<GraphDeliveryOutcome, MemoryError> {
        delivery
            .event
            .validate()
            .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
        let mut assertions = self
            .assertions
            .load_promoted_assertions(delivery.event.event_id)
            .await?;
        assertions.sort_by(assertion_order);
        if assertions.is_empty() {
            return Ok(GraphDeliveryOutcome::SucceededSuppressed {
                reason_code: "knowledge_graph_assertions_empty".to_string(),
            });
        }
        for assertion in &assertions {
            validate_delivery_lineage(&delivery.event, assertion)?;
        }

        if delivery.event.event_name == KnowledgeEventNameV1::SourceScopeInvalidated {
            for assertion in &assertions {
                let valid_to = assertion.valid_to.ok_or_else(|| {
                    MemoryError::Policy("knowledge_graph_invalidation_boundary_missing".to_string())
                })?;
                self.graph
                    .close_assertion_lineage(&project_invalidation(assertion, valid_to))
                    .await?;
            }
            return Ok(GraphDeliveryOutcome::Succeeded);
        }

        let mut projected = 0usize;
        let mut stale = 0usize;
        for assertion in &assertions {
            let projection = project_assertion(assertion).map_err(projection_error)?;
            projected += 1;
            let result = self.graph.apply_projection(&projection).await?;
            if matches!(result.disposition, ProjectionWriteDisposition::Stale { .. }) {
                stale += 1;
            }
        }
        if projected == 0 {
            return Ok(GraphDeliveryOutcome::SucceededSuppressed {
                reason_code: "knowledge_graph_no_supported_assertions".to_string(),
            });
        }
        if projected == stale {
            return Ok(GraphDeliveryOutcome::SucceededSuppressed {
                reason_code: "knowledge_graph_stale_source_version".to_string(),
            });
        }
        Ok(GraphDeliveryOutcome::Succeeded)
    }

    pub async fn rebuild_scope(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<GraphGeneration, MemoryError> {
        let assertions = self
            .assertions
            .list_active_assertions_for_rebuild(scope)
            .await?;
        rebuild_graph_scope_from_assertions(self.graph, scope, assertions).await
    }
}

pub async fn rebuild_graph_scope_from_assertions<G>(
    graph: &G,
    scope: &GraphRebuildScope,
    mut assertions: Vec<KnowledgeAssertion>,
) -> Result<GraphGeneration, MemoryError>
where
    G: TemporalGraphProjectionPort,
{
    for assertion in &assertions {
        validate_rebuild_scope(scope, assertion)?;
    }
    let mut max_versions = std::collections::BTreeMap::new();
    for assertion in &assertions {
        max_versions
            .entry(assertion.source.source_stream_key.clone())
            .and_modify(|version: &mut i64| *version = (*version).max(assertion.source.version))
            .or_insert(assertion.source.version);
    }
    assertions.retain(|assertion| {
        max_versions.get(&assertion.source.source_stream_key) == Some(&assertion.source.version)
    });
    assertions.sort_by(assertion_order);
    let generation = graph.begin_rebuild(scope).await?;
    let rebuild = populate_and_activate_rebuild(graph, generation.generation_id, &assertions).await;
    match rebuild {
        Ok(activated) => Ok(activated),
        Err(error) => {
            graph
                .fail_rebuild(generation.generation_id, error.code())
                .await?;
            Err(error)
        }
    }
}

async fn populate_and_activate_rebuild<G>(
    graph: &G,
    generation_id: Uuid,
    assertions: &[KnowledgeAssertion],
) -> Result<GraphGeneration, MemoryError>
where
    G: TemporalGraphProjectionPort,
{
    for assertion in assertions {
        let projection = project_assertion(assertion).map_err(projection_error)?;
        let result = graph
            .apply_projection_to_generation(generation_id, &projection)
            .await?;
        if matches!(result.disposition, ProjectionWriteDisposition::Stale { .. }) {
            return Err(MemoryError::Policy(
                "knowledge_graph_rebuild_source_stale".to_string(),
            ));
        }
    }
    let attestation = graph.generation_attestation(generation_id).await?;
    graph.activate_rebuild(generation_id, &attestation).await
}

fn validate_delivery_lineage(
    event: &KnowledgeEventEnvelopeV1,
    assertion: &KnowledgeAssertion,
) -> Result<(), MemoryError> {
    assertion
        .validate_integrity()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    let invalidation = event.event_name == KnowledgeEventNameV1::SourceScopeInvalidated;
    if (invalidation && assertion.status == AssertionStatus::Active)
        || (!invalidation && assertion.status != AssertionStatus::Active)
    {
        return Err(MemoryError::Policy(
            "knowledge_graph_event_assertion_status_mismatch".to_string(),
        ));
    }
    if assertion.source_operation_id != event.source_operation_id
        || assertion.source != event.payload.source
        || assertion.source.source_stream_key != event.payload.source_stream_key
        || assertion.source.version != event.payload.source_version
    {
        return Err(MemoryError::Policy(
            "knowledge_graph_event_lineage_mismatch".to_string(),
        ));
    }
    match &assertion.visibility {
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id,
            organization_id_at_time,
        } if event.project_scope_id == Some(*project_scope_id)
            && event.organization_id_at_time == Some(*organization_id_at_time) => {}
        AssertionVisibility::GlobalSanitized
            if event.project_scope_id.is_none() && event.organization_id_at_time.is_none() => {}
        _ => {
            return Err(MemoryError::Policy(
                "knowledge_graph_event_scope_mismatch".to_string(),
            ))
        }
    }
    Ok(())
}

fn validate_rebuild_scope(
    scope: &GraphRebuildScope,
    assertion: &KnowledgeAssertion,
) -> Result<(), MemoryError> {
    assertion
        .validate_integrity()
        .map_err(|error| MemoryError::Policy(error.code().to_string()))?;
    if assertion.status != AssertionStatus::Active {
        return Err(MemoryError::Policy(
            "knowledge_graph_rebuild_assertion_not_active".to_string(),
        ));
    }
    let matches = match (scope, &assertion.visibility) {
        (
            GraphRebuildScope::Organization {
                project_scope_id,
                organization_id_at_time,
            },
            AssertionVisibility::OrganizationLongTerm {
                project_scope_id: assertion_project,
                organization_id_at_time: assertion_organization,
            },
        ) => {
            project_scope_id == assertion_project
                && organization_id_at_time == assertion_organization
        }
        (GraphRebuildScope::GlobalSanitized, AssertionVisibility::GlobalSanitized) => true,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(MemoryError::Policy(
            "knowledge_graph_rebuild_scope_mismatch".to_string(),
        ))
    }
}

fn assertion_order(left: &KnowledgeAssertion, right: &KnowledgeAssertion) -> std::cmp::Ordering {
    left.source
        .source_stream_key
        .cmp(&right.source.source_stream_key)
        .then_with(|| left.source.version.cmp(&right.source.version))
        .then_with(|| {
            left.identity
                .identity_hash
                .cmp(&right.identity.identity_hash)
        })
        .then_with(|| left.assertion_id.cmp(&right.assertion_id))
}

fn projection_error(error: ProjectionError) -> MemoryError {
    MemoryError::GraphProjection(error.code().to_string())
}

fn graph_port_error(error: golish_graphiti::GraphError) -> MemoryError {
    MemoryError::Port(format!("{}: {error}", error.code()))
}
