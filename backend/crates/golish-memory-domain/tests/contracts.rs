use golish_memory_domain::{
    assertion::{AssertionIdentity, AssertionObject},
    embedding::EMBEDDING_DIMENSION_V1,
    event_catalog::{routes_for, KnowledgeEventNameV1},
    source_ref::{CanonicalRowId, StoredCanonicalRowId},
};

#[test]
fn canonical_bigserial_round_trips_without_uuid_coercion() {
    let id = CanonicalRowId::Int64(42);
    let stored = StoredCanonicalRowId::from_domain(&id).expect("store int64");
    assert_eq!(stored.kind, "int64");
    assert_eq!(stored.value, "42");
    assert_eq!(stored.into_domain().expect("load int64"), id);
}

#[test]
fn assertion_identity_distinguishes_objects_for_same_predicate() {
    let first = AssertionIdentity::derive(
        "target:example.com",
        "http.status",
        &AssertionObject::Json(serde_json::json!(200)),
    )
    .expect("first identity");
    let second = AssertionIdentity::derive(
        "target:example.com",
        "http.status",
        &AssertionObject::Json(serde_json::json!(404)),
    )
    .expect("second identity");

    assert_ne!(first.object_hash, second.object_hash);
    assert_ne!(first.identity_hash, second.identity_hash);
}

#[test]
fn searchable_events_use_the_three_stage_delivery_dag() {
    let routes = routes_for(KnowledgeEventNameV1::FactDeltaAccepted);
    let names = routes
        .iter()
        .map(|route| route.projector.key())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "assertion-promoter@1",
            "document-projector@1",
            "embedding-projector@1",
            "graph-projector@1"
        ]
    );
    assert_eq!(routes[1].depends_on, Some(routes[0].projector));
    assert_eq!(routes[2].depends_on, Some(routes[1].projector));
    assert_eq!(routes[3].depends_on, Some(routes[0].projector));
    assert_eq!(EMBEDDING_DIMENSION_V1, 1536);
}
