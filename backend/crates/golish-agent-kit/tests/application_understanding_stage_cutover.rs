use golish_agent_kit::harness::{
    load_embedded_phase_map, load_embedded_profile, load_embedded_stage_spec,
    operation_graph_for_topology, StageKind, StageTopologyContract, EMBEDDED_PROFILE_IDS,
};

#[test]
fn legacy_and_unified_operation_graphs_have_frozen_distinct_investigation_crossings() {
    let legacy = operation_graph_for_topology(StageTopologyContract::LegacyCandidateVerificationV1)
        .expect("legacy operation graph");
    assert!(!legacy.nodes.contains(&StageKind::ApplicationUnderstanding));
    assert_eq!(
        legacy
            .edges
            .iter()
            .filter(|edge| edge.from == StageKind::VulnTriage)
            .map(|edge| edge.to)
            .collect::<Vec<_>>(),
        vec![StageKind::AttackCandidate, StageKind::Reporting]
    );

    let v1 = operation_graph_for_topology(StageTopologyContract::UnifiedInvestigationV1)
        .expect("Unified Investigation graph");
    assert!(v1.nodes.contains(&StageKind::ApplicationUnderstanding));
    assert_eq!(
        v1.edges
            .iter()
            .filter(|edge| edge.from == StageKind::VulnTriage)
            .map(|edge| edge.to)
            .collect::<Vec<_>>(),
        vec![StageKind::ApplicationUnderstanding]
    );
    assert_eq!(
        v1.edges
            .iter()
            .filter(|edge| edge.from == StageKind::ApplicationUnderstanding)
            .map(|edge| edge.to)
            .collect::<Vec<_>>(),
        vec![StageKind::Investigation]
    );
}

#[test]
fn application_understanding_stage_is_reasoning_only_and_in_the_vuln_phase() {
    let spec = load_embedded_stage_spec(StageKind::ApplicationUnderstanding)
        .expect("load Application Understanding spec");
    assert_eq!(spec.kind, StageKind::ApplicationUnderstanding);
    assert_eq!(
        spec.specialist.as_deref(),
        Some("application_understanding")
    );
    assert_eq!(spec.requires_stages, vec![StageKind::VulnTriage]);
    assert_eq!(spec.allowed_next_stages, vec![StageKind::Investigation]);
    assert!(spec.allowed_tool_types.is_empty());
    assert!(!spec.findings_allowed);

    let phases = load_embedded_phase_map().expect("load phase map");
    assert_eq!(
        phases
            .phase_of(StageKind::ApplicationUnderstanding)
            .expect("AU phase")
            .id,
        "vuln"
    );
    for profile_id in EMBEDDED_PROFILE_IDS {
        let profile = load_embedded_profile(profile_id)
            .expect("load profile")
            .expect("profile exists");
        assert!(!profile
            .allowed_stage_set()
            .contains(&StageKind::ApplicationUnderstanding));
    }
}

#[test]
fn unified_investigation_topology_parser_is_closed() {
    assert_eq!(
        StageTopologyContract::try_parse("unified_investigation_v1").expect("unified contract"),
        StageTopologyContract::UnifiedInvestigationV1
    );
    assert!(StageTopologyContract::try_parse("latest_if_available").is_err());
}
