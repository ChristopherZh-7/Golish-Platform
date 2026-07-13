use chrono::{TimeZone, Utc};
use golish_memory_app::graph_projection::{
    project_assertion, project_invalidation, ProjectionError,
};
use golish_memory_domain::assertion::{
    AssertionKind, AssertionObject, AssertionStatus, KnowledgeAssertion,
};
use golish_memory_domain::classification::{AssertionVisibility, KnowledgeClassification};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalRowId, CanonicalSourceKind, SourceRef};
use uuid::Uuid;

fn entity_assertion(assertion_id: u128, stream: &str, version: i64) -> KnowledgeAssertion {
    let mut assertion = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "target:7/host:10.0.0.5",
        "graph.entity.host",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "target:7/host:10.0.0.5",
            "display_name": "10.0.0.5",
            "properties": {
                "address_family": "ipv4",
                "os_family": "linux",
                "raw_response": "must-not-project"
            }
        })),
        AssertionKind::Observation,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Int64(assertion_id as i64),
            source_stream_key: stream.to_string(),
            version,
        },
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("valid host assertion");
    assertion.assertion_id = Uuid::from_u128(assertion_id);
    assertion
}

#[test]
fn shared_identity_retains_independent_assertion_lineage() {
    let first = entity_assertion(11, "target:7", 1);
    let second = entity_assertion(12, "dns:9", 4);

    let first_projection = project_assertion(&first).expect("first projection");
    let second_projection = project_assertion(&second).expect("second projection");

    assert_eq!(
        first_projection.entities[0].canonical_ref,
        second_projection.entities[0].canonical_ref
    );
    assert_eq!(
        first_projection.entities[0].scope_key,
        second_projection.entities[0].scope_key
    );
    assert_ne!(
        first_projection.entity_lineages[0].assertion_id,
        second_projection.entity_lineages[0].assertion_id
    );
    assert!(first_projection.entities[0]
        .properties
        .get("raw_response")
        .is_none());

    let invalidation = project_invalidation(
        &first,
        Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("fixed timestamp"),
    );
    assert_eq!(invalidation.close_assertion_id, first.assertion_id);
    assert_ne!(invalidation.close_assertion_id, second.assertion_id);
}

#[test]
fn global_sanitized_allows_only_safe_technique_projection() {
    let technique = KnowledgeAssertion::new_for_test(
        AssertionVisibility::GlobalSanitized,
        "technique:T1190",
        "graph.entity.technique",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "technique:T1190",
            "display_name": "Exploit Public-Facing Application",
            "properties": {"technique_id": "T1190", "category": "initial_access"}
        })),
        AssertionKind::TechniqueExperience,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Text("global-technique-T1190".to_string()),
            source_stream_key: "global-technique:T1190".to_string(),
            version: 1,
        },
        KnowledgeClassification::Internal,
    )
    .expect("valid sanitized technique");
    assert_eq!(
        project_assertion(&technique)
            .expect("technique")
            .entities
            .len(),
        1
    );

    let unsafe_host = KnowledgeAssertion::new_for_test(
        AssertionVisibility::GlobalSanitized,
        "host:10.0.0.5",
        "graph.entity.host",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "host:10.0.0.5",
            "display_name": "10.0.0.5"
        })),
        AssertionKind::TechniqueExperience,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Text("global-host".to_string()),
            source_stream_key: "global-host:10.0.0.5".to_string(),
            version: 1,
        },
        KnowledgeClassification::Internal,
    )
    .expect("domain-valid but graph-policy-invalid host");
    assert_eq!(
        project_assertion(&unsafe_host).expect_err("global host must fail"),
        ProjectionError::GlobalEntityMustBeTechnique
    );
}

#[test]
fn unknown_predicate_fails_closed() {
    let assertion = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "target:7/host:10.0.0.5",
        "model.prose.guess",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "target:7/host:10.0.0.5",
            "display_name": "10.0.0.5"
        })),
        AssertionKind::Observation,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Int64(13),
            source_stream_key: "target:7".to_string(),
            version: 2,
        },
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("valid unsupported assertion");
    assert_eq!(
        project_assertion(&assertion).expect_err("unknown predicate must fail"),
        ProjectionError::UnsupportedPredicate("model.prose.guess".to_string())
    );
}

#[test]
fn mutated_assertion_identity_is_rejected_before_projection() {
    let mut assertion = entity_assertion(14, "target:7", 3);
    assertion.identity.predicate = "graph.entity.target".to_string();

    assert_eq!(
        project_assertion(&assertion).expect_err("mutated identity must fail"),
        ProjectionError::InvalidAssertion
    );
}

#[test]
fn allowlisted_property_cannot_smuggle_nested_material() {
    let assertion = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "target:7/host:10.0.0.6",
        "graph.entity.host",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "target:7/host:10.0.0.6",
            "display_name": "10.0.0.6",
            "properties": {
                "os_family": {"opaque": "material-that-must-not-project"}
            }
        })),
        AssertionKind::Observation,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Int64(15),
            source_stream_key: "target:7".to_string(),
            version: 4,
        },
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("domain-valid nested property");

    assert_eq!(
        project_assertion(&assertion).expect_err("nested property must fail closed"),
        ProjectionError::PropertyValueRejected
    );
}

#[test]
fn canonical_and_display_fields_are_bounded_and_control_free() {
    let assertion = KnowledgeAssertion::new_for_test(
        AssertionVisibility::OrganizationLongTerm {
            project_scope_id: ProjectScopeId(Uuid::from_u128(1)),
            organization_id_at_time: Uuid::from_u128(2),
        },
        "target:7/host:bad",
        "graph.entity.host",
        AssertionObject::Json(serde_json::json!({
            "canonical_ref": "host:bad\nforged",
            "display_name": "bad"
        })),
        AssertionKind::Observation,
        AssertionStatus::Active,
        SourceRef {
            source_kind: CanonicalSourceKind::FactDelta,
            row_id: CanonicalRowId::Int64(16),
            source_stream_key: "target:7".to_string(),
            version: 5,
        },
        KnowledgeClassification::CustomerConfidential,
    )
    .expect("domain-valid control-character fixture");

    assert_eq!(
        project_assertion(&assertion).expect_err("control characters must fail"),
        ProjectionError::FieldInvalid("canonical_ref")
    );
}
