use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use golish_graphiti::{
    ApplyProjectionResult, GraphGeneration, GraphScopeKey, ProjectionWriteDisposition,
    RebuildAttestation, TemporalGraphInvalidation, TemporalGraphProjection,
};
use golish_memory_app::{
    GraphAssertionReader, GraphDeliveryOutcome, GraphProjectionDelivery,
    GraphProjectionDeliveryPort, GraphProjector, GraphProjectorTick, GraphRebuildScope,
    MemoryError, TemporalGraphProjectionPort,
};
use golish_memory_domain::assertion::{
    AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::event_catalog::{
    KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use uuid::Uuid;

fn source(stream: &str, version: i64) -> SourceRef {
    SourceRef {
        source_kind: CanonicalSourceKind::FactDelta,
        row_id: CanonicalRowId::Text(format!("{stream}:{version}")),
        source_stream_key: stream.to_string(),
        version,
    }
}

fn event(event_id: Uuid, source: SourceRef) -> KnowledgeEventEnvelopeV1 {
    KnowledgeEventEnvelopeV1 {
        event_id,
        project_scope_id: Some(ProjectScopeId(Uuid::from_u128(1))),
        organization_id_at_time: Some(Uuid::from_u128(2)),
        source_operation_id: Uuid::nil(),
        event_name: KnowledgeEventNameV1::FactDeltaAccepted,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source_stream_key: source.source_stream_key.clone(),
            source_version: source.version,
            source,
            structured_payload: serde_json::json!({"ignored_by_graph": true}),
        },
        occurred_at: Utc
            .with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("fixed timestamp"),
    }
}

fn host_assertion(stream: &str, version: i64) -> KnowledgeAssertion {
    let mut assertion = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "host:10.0.0.5",
        "graph.entity.host",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "host:10.0.0.5",
            "display_name": "10.0.0.5",
            "properties": {"address_family": "ipv4"}
        })),
        AssertionKind::Observation,
        AssertionStatus::Active,
        source(stream, version),
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("valid host assertion");
    assertion.assertion_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{stream}:{version}").as_bytes(),
    );
    assertion
}

#[derive(Default)]
struct FakeDeliveries {
    queued: Mutex<VecDeque<GraphProjectionDelivery>>,
    outcomes: Mutex<Vec<GraphDeliveryOutcome>>,
    retry_codes: Mutex<Vec<String>>,
    fail_next_completion: Mutex<bool>,
    document_delivery_still_pending: Mutex<bool>,
}

#[async_trait]
impl GraphProjectionDeliveryPort for FakeDeliveries {
    async fn claim_graph_delivery(
        &self,
        _worker_id: &str,
    ) -> Result<Option<GraphProjectionDelivery>, MemoryError> {
        Ok(self.queued.lock().expect("queue mutex").front().cloned())
    }

    async fn complete_graph_delivery(
        &self,
        _event_id: Uuid,
        _worker_id: &str,
        outcome: GraphDeliveryOutcome,
    ) -> Result<(), MemoryError> {
        let mut fail = self.fail_next_completion.lock().expect("failure mutex");
        if *fail {
            *fail = false;
            return Err(MemoryError::Port("simulated ack crash".to_string()));
        }
        self.queued.lock().expect("queue mutex").pop_front();
        self.outcomes.lock().expect("outcome mutex").push(outcome);
        Ok(())
    }

    async fn retry_graph_delivery(
        &self,
        _event_id: Uuid,
        _worker_id: &str,
        error_code: &str,
    ) -> Result<(), MemoryError> {
        self.retry_codes
            .lock()
            .expect("retry mutex")
            .push(error_code.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct FakeAssertions {
    by_event: Mutex<HashMap<Uuid, Vec<KnowledgeAssertion>>>,
    rebuild: Mutex<Vec<KnowledgeAssertion>>,
}

#[async_trait]
impl GraphAssertionReader for FakeAssertions {
    async fn load_promoted_assertions(
        &self,
        event_id: Uuid,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        Ok(self
            .by_event
            .lock()
            .expect("assertion mutex")
            .get(&event_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn list_active_assertions_for_rebuild(
        &self,
        _scope: &GraphRebuildScope,
    ) -> Result<Vec<KnowledgeAssertion>, MemoryError> {
        Ok(self.rebuild.lock().expect("rebuild mutex").clone())
    }
}

struct FakeGraphState {
    stored_assertions: HashMap<Uuid, Uuid>,
    apply_calls: usize,
    active_generation: Uuid,
    building_generation: Option<Uuid>,
    active_seen_during_build: Vec<Uuid>,
    failed_generations: Vec<Uuid>,
    fail_build_apply: bool,
}

struct FakeGraph {
    state: Mutex<FakeGraphState>,
}

impl Default for FakeGraph {
    fn default() -> Self {
        Self {
            state: Mutex::new(FakeGraphState {
                stored_assertions: HashMap::new(),
                apply_calls: 0,
                active_generation: Uuid::from_u128(0x10),
                building_generation: None,
                active_seen_during_build: Vec::new(),
                failed_generations: Vec::new(),
                fail_build_apply: false,
            }),
        }
    }
}

fn projection_assertion_id(projection: &TemporalGraphProjection) -> Uuid {
    projection
        .entity_lineages
        .first()
        .expect("projection lineage")
        .assertion_id
}

#[async_trait]
impl TemporalGraphProjectionPort for FakeGraph {
    async fn apply_projection(
        &self,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError> {
        let mut state = self.state.lock().expect("graph mutex");
        state.apply_calls += 1;
        let assertion_id = projection_assertion_id(projection);
        let generation_id = state.active_generation;
        let disposition = if state
            .stored_assertions
            .insert(assertion_id, generation_id)
            .is_some()
        {
            ProjectionWriteDisposition::ExactReplay
        } else {
            ProjectionWriteDisposition::Applied
        };
        Ok(ApplyProjectionResult {
            disposition,
            generation_id,
            entity_count: projection.entities.len(),
            relation_count: projection.relations.len(),
        })
    }

    async fn apply_projection_to_generation(
        &self,
        generation_id: Uuid,
        projection: &TemporalGraphProjection,
    ) -> Result<ApplyProjectionResult, MemoryError> {
        let mut state = self.state.lock().expect("graph mutex");
        let active = state.active_generation;
        state.active_seen_during_build.push(active);
        if state.fail_build_apply {
            return Err(MemoryError::Port(
                "simulated rebuild write failure".to_string(),
            ));
        }
        state
            .stored_assertions
            .insert(projection_assertion_id(projection), generation_id);
        Ok(ApplyProjectionResult {
            disposition: ProjectionWriteDisposition::Applied,
            generation_id,
            entity_count: projection.entities.len(),
            relation_count: projection.relations.len(),
        })
    }

    async fn close_assertion_lineage(
        &self,
        invalidation: &TemporalGraphInvalidation,
    ) -> Result<(u64, u64), MemoryError> {
        let removed = self
            .state
            .lock()
            .expect("graph mutex")
            .stored_assertions
            .remove(&invalidation.close_assertion_id)
            .is_some();
        Ok((u64::from(removed), 0))
    }

    async fn begin_rebuild(
        &self,
        scope: &GraphRebuildScope,
    ) -> Result<GraphGeneration, MemoryError> {
        let mut state = self.state.lock().expect("graph mutex");
        if state.building_generation.is_some() {
            return Err(MemoryError::Port("concurrent rebuild rejected".to_string()));
        }
        let generation_id = Uuid::new_v4();
        state.building_generation = Some(generation_id);
        Ok(GraphGeneration {
            generation_id,
            scope_key: scope.scope_key(),
            projection_schema_version: 1,
            status: "building".to_string(),
            build_hash: None,
            entity_count: None,
            relation_count: None,
        })
    }

    async fn generation_attestation(
        &self,
        generation_id: Uuid,
    ) -> Result<RebuildAttestation, MemoryError> {
        let state = self.state.lock().expect("graph mutex");
        let count = state
            .stored_assertions
            .values()
            .filter(|stored_generation| **stored_generation == generation_id)
            .count() as i64;
        Ok(RebuildAttestation {
            build_hash: format!("{count:064x}"),
            entity_count: count,
            relation_count: 0,
        })
    }

    async fn activate_rebuild(
        &self,
        generation_id: Uuid,
        _attestation: &RebuildAttestation,
    ) -> Result<GraphGeneration, MemoryError> {
        let mut state = self.state.lock().expect("graph mutex");
        if state.building_generation != Some(generation_id) {
            return Err(MemoryError::Port("stale rebuild activation".to_string()));
        }
        state.active_generation = generation_id;
        state.building_generation = None;
        Ok(GraphGeneration {
            generation_id,
            scope_key: GraphScopeKey::organization(
                ProjectScopeId(Uuid::from_u128(1)),
                Uuid::from_u128(2),
            ),
            projection_schema_version: 1,
            status: "active".to_string(),
            build_hash: Some("0".repeat(64)),
            entity_count: Some(1),
            relation_count: Some(0),
        })
    }

    async fn fail_rebuild(&self, generation_id: Uuid, _reason: &str) -> Result<(), MemoryError> {
        let mut state = self.state.lock().expect("graph mutex");
        state.failed_generations.push(generation_id);
        if state.building_generation == Some(generation_id) {
            state.building_generation = None;
        }
        Ok(())
    }
}

#[tokio::test]
async fn ack_crash_replays_exactly_without_consuming_document_delivery() {
    let event_id = Uuid::from_u128(0x20);
    let source = source("fact:20", 1);
    let deliveries = FakeDeliveries::default();
    deliveries
        .queued
        .lock()
        .expect("queue mutex")
        .push_back(GraphProjectionDelivery {
            event: event(event_id, source),
        });
    *deliveries
        .fail_next_completion
        .lock()
        .expect("failure mutex") = true;
    *deliveries
        .document_delivery_still_pending
        .lock()
        .expect("document mutex") = true;
    let assertions = FakeAssertions::default();
    assertions
        .by_event
        .lock()
        .expect("assertion mutex")
        .insert(event_id, vec![host_assertion("fact:20", 1)]);
    let graph = FakeGraph::default();
    let projector =
        GraphProjector::new(&deliveries, &assertions, &graph, "graph-worker").expect("worker");

    projector.run_once().await.expect_err("simulated ack crash");
    assert_eq!(
        graph
            .state
            .lock()
            .expect("graph mutex")
            .stored_assertions
            .len(),
        1
    );

    assert_eq!(
        projector.run_once().await.expect("replayed delivery"),
        GraphProjectorTick::Succeeded { event_id }
    );
    let state = graph.state.lock().expect("graph mutex");
    assert_eq!(state.apply_calls, 2);
    assert_eq!(state.stored_assertions.len(), 1);
    assert!(*deliveries
        .document_delivery_still_pending
        .lock()
        .expect("document mutex"));
}

#[tokio::test]
async fn unknown_predicate_is_retryable_failure_not_terminal_suppression() {
    let event_id = Uuid::from_u128(0x30);
    let source = source("fact:30", 1);
    let deliveries = FakeDeliveries::default();
    deliveries
        .queued
        .lock()
        .expect("queue mutex")
        .push_back(GraphProjectionDelivery {
            event: event(event_id, source.clone()),
        });
    let assertions = FakeAssertions::default();
    let unsupported = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "host:10.0.0.5",
        "model.prose.guess",
        AssertionObject::Json(serde_json::json!({"value": "guess"})),
        AssertionKind::Observation,
        AssertionStatus::Active,
        source,
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("domain-valid unsupported assertion");
    assertions
        .by_event
        .lock()
        .expect("assertion mutex")
        .insert(event_id, vec![unsupported]);
    let graph = FakeGraph::default();
    let projector =
        GraphProjector::new(&deliveries, &assertions, &graph, "graph-worker").expect("worker");

    let error = projector
        .run_once()
        .await
        .expect_err("unknown schema must not be swallowed");
    assert_eq!(error.code(), "knowledge_graph_predicate_unsupported");
    assert_eq!(
        deliveries
            .retry_codes
            .lock()
            .expect("retry mutex")
            .as_slice(),
        ["knowledge_graph_predicate_unsupported"]
    );
    assert!(deliveries
        .outcomes
        .lock()
        .expect("outcome mutex")
        .is_empty());
}

#[tokio::test]
async fn rebuild_keeps_old_generation_visible_until_cutover_and_failure_preserves_it() {
    let deliveries = FakeDeliveries::default();
    let assertions = FakeAssertions::default();
    assertions
        .rebuild
        .lock()
        .expect("rebuild mutex")
        .push(host_assertion("fact:40", 1));
    let graph = FakeGraph::default();
    let old_generation = graph.state.lock().expect("graph mutex").active_generation;
    let projector =
        GraphProjector::new(&deliveries, &assertions, &graph, "graph-worker").expect("worker");
    let scope = GraphRebuildScope::Organization {
        project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
        organization_id_at_time: Uuid::from_u128(2),
    };

    let activated = projector
        .rebuild_scope(&scope)
        .await
        .expect("activate rebuilt generation");
    {
        let state = graph.state.lock().expect("graph mutex");
        assert!(state
            .active_seen_during_build
            .iter()
            .all(|generation| *generation == old_generation));
        assert_eq!(state.active_generation, activated.generation_id);
    }

    graph.state.lock().expect("graph mutex").fail_build_apply = true;
    let active_before_failure = graph.state.lock().expect("graph mutex").active_generation;
    projector
        .rebuild_scope(&scope)
        .await
        .expect_err("failed generation must not cut over");
    let state = graph.state.lock().expect("graph mutex");
    assert_eq!(state.active_generation, active_before_failure);
    assert_eq!(state.failed_generations.len(), 1);
}

#[tokio::test]
async fn rebuild_keeps_all_latest_version_assertions_and_drops_older_stream_versions() {
    let deliveries = FakeDeliveries::default();
    let assertions = FakeAssertions::default();
    let older = host_assertion("versioned-rebuild", 1);
    let latest = host_assertion("versioned-rebuild", 2);
    assertions
        .rebuild
        .lock()
        .expect("rebuild mutex")
        .extend([older.clone(), latest.clone()]);
    let graph = FakeGraph::default();
    let projector =
        GraphProjector::new(&deliveries, &assertions, &graph, "graph-worker").expect("worker");

    projector
        .rebuild_scope(&GraphRebuildScope::Organization {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        })
        .await
        .expect("latest-only rebuild");
    let state = graph.state.lock().expect("graph mutex");
    assert!(!state.stored_assertions.contains_key(&older.assertion_id));
    assert!(state.stored_assertions.contains_key(&latest.assertion_id));
}
