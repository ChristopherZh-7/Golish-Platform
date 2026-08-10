use golish_agent_kit::harness::{
    load_embedded_profile, load_embedded_stage_spec, operation_graph_for_topology, StageKind,
    StageTopologyContract, EMBEDDED_PROFILE_IDS,
};
use sha2::{Digest, Sha256};

const LEGACY_GRAPH_BYTES: &[u8] =
    include_bytes!("../../../../resources/harness/graph/operation_graph.json");

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn legacy_operations_keep_candidate_then_verification_byte_for_byte() {
    assert_eq!(
        sha256(LEGACY_GRAPH_BYTES),
        "963963730640b4799e2b72cde6052a98d3765f8f7ad8e0ea1286c2ce13e656fd",
        "the historical graph resource must not be rewritten for unified topology"
    );

    let graph = operation_graph_for_topology(StageTopologyContract::LegacyCandidateVerificationV1)
        .expect("legacy graph");
    assert!(!graph.nodes.contains(&StageKind::ApplicationUnderstanding));
    assert!(!graph.nodes.contains(&StageKind::Investigation));
    assert_eq!(
        graph
            .project(&graph.nodes.iter().copied().collect())
            .next_stages(StageKind::AttackCandidate),
        vec![StageKind::Verification, StageKind::Reporting]
    );
}

#[test]
fn unified_investigation_profile_has_one_stage_and_no_outer_split() {
    let graph = operation_graph_for_topology(StageTopologyContract::UnifiedInvestigationV1)
        .expect("unified graph");
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|stage| **stage == StageKind::Investigation)
            .count(),
        1
    );
    assert!(graph.nodes.contains(&StageKind::ApplicationUnderstanding));
    assert!(!graph.nodes.contains(&StageKind::AttackCandidate));
    assert!(!graph.nodes.contains(&StageKind::Verification));
}

#[test]
fn unified_investigation_tail_is_vuln_au_investigation_reporting_only() {
    let graph = operation_graph_for_topology(StageTopologyContract::UnifiedInvestigationV1)
        .expect("unified graph");
    let dag = graph.project(&graph.nodes.iter().copied().collect());

    assert_eq!(
        dag.next_stages(StageKind::VulnTriage),
        vec![StageKind::ApplicationUnderstanding]
    );
    assert_eq!(
        dag.next_stages(StageKind::ApplicationUnderstanding),
        vec![StageKind::Investigation]
    );
    assert_eq!(
        dag.next_stages(StageKind::Investigation),
        vec![StageKind::Reporting]
    );
    assert!(!graph
        .edges
        .iter()
        .any(|edge| { edge.from == StageKind::VulnTriage && edge.to == StageKind::Investigation }));
}

#[test]
fn investigation_stage_spec_is_tool_free_non_finding_and_ai_owned_task_orchestrator() {
    let spec = load_embedded_stage_spec(StageKind::Investigation).expect("Investigation spec");
    assert_eq!(spec.id, "investigation");
    assert_eq!(spec.kind, StageKind::Investigation);
    assert_eq!(
        spec.requires_stages,
        vec![StageKind::ApplicationUnderstanding]
    );
    assert_eq!(spec.allowed_next_stages, vec![StageKind::Reporting]);
    assert!(!spec.findings_allowed);
    assert!(spec.allowed_tool_types.is_empty());
    let scheduler = spec
        .team_scheduler
        .as_ref()
        .expect("Investigation needs one durable Primary governance envelope");
    assert_eq!(scheduler.aggregator_kind, "investigation_primary");
    assert_eq!(scheduler.aggregator_role, "investigation");
    assert_eq!(
        scheduler.allowed_dynamic_request_kinds,
        vec!["analysis_task", "verification_task", "cognitive_support"]
    );
    assert_eq!(scheduler.risk_lane, "cognitive_only");
    assert!(spec.candidate_analysis_team.is_none());
}

#[test]
fn operation_frozen_topology_projects_each_profile_without_mixing_outer_stages() {
    for profile_id in EMBEDDED_PROFILE_IDS {
        let profile = load_embedded_profile(profile_id)
            .unwrap_or_else(|error| panic!("{profile_id} profile parse failed: {error}"))
            .unwrap_or_else(|| panic!("{profile_id} profile missing"));
        let raw = profile.allowed_stage_set();
        assert!(
            !raw.contains(&StageKind::ApplicationUnderstanding)
                && !raw.contains(&StageKind::Investigation),
            "profile catalog stays topology-neutral"
        );

        let legacy = profile
            .allowed_stage_set_for_topology(StageTopologyContract::LegacyCandidateVerificationV1)
            .expect("legacy projection");
        assert!(!legacy.contains(&StageKind::ApplicationUnderstanding));
        assert!(!legacy.contains(&StageKind::Investigation));

        let unified = profile
            .allowed_stage_set_for_topology(StageTopologyContract::UnifiedInvestigationV1)
            .expect("unified projection");
        let profile_has_attack_analysis =
            raw.contains(&StageKind::AttackCandidate) && raw.contains(&StageKind::Verification);
        assert!(
            !unified.contains(&StageKind::AttackCandidate)
                && !unified.contains(&StageKind::Verification),
            "unified projection must never retain the legacy outer split"
        );
        assert_eq!(
            unified.contains(&StageKind::ApplicationUnderstanding),
            profile_has_attack_analysis
        );
        assert_eq!(
            unified.contains(&StageKind::Investigation),
            profile_has_attack_analysis
        );

        let unified_graph =
            operation_graph_for_topology(StageTopologyContract::UnifiedInvestigationV1)
                .expect("unified graph")
                .project(&unified);
        if profile_has_attack_analysis {
            assert_eq!(
                unified_graph.next_stages(StageKind::VulnTriage),
                vec![StageKind::ApplicationUnderstanding]
            );
            assert_eq!(
                unified_graph.next_stages(StageKind::ApplicationUnderstanding),
                vec![StageKind::Investigation]
            );
        }
    }
}

#[test]
fn stage_topology_contract_freeze_material_is_exact_and_unknown_fails_closed() {
    for topology in StageTopologyContract::ALL {
        topology
            .freeze_material()
            .validate()
            .expect("server-owned topology material");
    }
    assert!(StageTopologyContract::try_parse("future_or_client_topology").is_err());
}
