use golish_agent_kit::harness::{
    application_model_operation_graph, load_embedded_phase_map, load_embedded_profile,
    load_embedded_stage_spec, ApplicationModelOperationContract, StageKind,
};

#[test]
fn legacy_and_v1_operation_graphs_have_frozen_distinct_candidate_crossings() {
    let profile = load_embedded_profile("pentest")
        .expect("load pentest profile")
        .expect("pentest profile exists");
    let allowed = profile.allowed_stage_set();

    let legacy =
        application_model_operation_graph(ApplicationModelOperationContract::LegacyNoModel)
            .expect("legacy operation graph")
            .project(&allowed);
    assert!(!legacy.contains(StageKind::ApplicationUnderstanding));
    assert_eq!(
        legacy.next_stages(StageKind::VulnTriage),
        vec![StageKind::AttackCandidate, StageKind::Reporting]
    );

    let v1 =
        application_model_operation_graph(ApplicationModelOperationContract::ApplicationModelV1)
            .expect("Application Model v1 operation graph")
            .project(&allowed);
    assert!(v1.contains(StageKind::ApplicationUnderstanding));
    assert_eq!(
        v1.next_stages(StageKind::VulnTriage),
        vec![StageKind::ApplicationUnderstanding, StageKind::Reporting]
    );
    assert_eq!(
        v1.next_stages(StageKind::ApplicationUnderstanding),
        vec![StageKind::AttackCandidate]
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
    assert_eq!(
        spec.allowed_next_stages,
        vec![StageKind::AttackCandidate, StageKind::Reporting]
    );
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
    let pentest = load_embedded_profile("pentest")
        .expect("load pentest profile")
        .expect("pentest profile exists");
    assert!(pentest
        .allowed_stage_set()
        .contains(&StageKind::ApplicationUnderstanding));
    for profile_id in ["assessment", "bug_bounty", "cloud_assessment"] {
        let profile = load_embedded_profile(profile_id)
            .expect("load non-Candidate profile")
            .expect("profile exists");
        assert!(!profile
            .allowed_stage_set()
            .contains(&StageKind::ApplicationUnderstanding));
    }
}

#[test]
fn application_model_operation_contract_parser_is_closed() {
    assert_eq!(
        ApplicationModelOperationContract::try_from("legacy_no_model").expect("legacy contract"),
        ApplicationModelOperationContract::LegacyNoModel
    );
    assert_eq!(
        ApplicationModelOperationContract::try_from("application_model_v1").expect("v1 contract"),
        ApplicationModelOperationContract::ApplicationModelV1
    );
    assert!(ApplicationModelOperationContract::try_from("latest_if_available").is_err());
}
