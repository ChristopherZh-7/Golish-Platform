use std::collections::BTreeSet;

use golish_agent_kit::harness::{
    admit_investigation_operator_tool, investigation_tool_catalog_json,
    load_embedded_investigation_tool_catalog, load_embedded_stage_spec,
    InvestigationOperatorToolCatalogV1, InvestigationToolAdmissionRequestV1,
    InvestigationToolAvailabilityV1, InvestigationToolContractStatusV1,
    InvestigationToolExecutionClassV1, StageKind, INVESTIGATION_OPERATOR_TOOL_CONTRACT_V1,
    INVESTIGATION_TOOL_CONFIG_IDS,
};

#[test]
fn admission_catalog_references_existing_tool_config_ids_exactly_once() {
    let catalog = load_embedded_investigation_tool_catalog().unwrap();
    let actual = catalog
        .tools
        .iter()
        .map(|profile| profile.tool_config_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual,
        INVESTIGATION_TOOL_CONFIG_IDS
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(actual.len(), catalog.tools.len());
}

#[test]
fn stage_spec_binds_the_exact_catalog_contract_and_hash() {
    let catalog = load_embedded_investigation_tool_catalog().unwrap();
    let stage = load_embedded_stage_spec(StageKind::Investigation).unwrap();
    let reference = stage
        .operator_tool_catalog
        .expect("Investigation catalog ref");
    assert_eq!(
        reference.contract_version,
        INVESTIGATION_OPERATOR_TOOL_CONTRACT_V1
    );
    assert_eq!(reference.canonical_sha256, catalog.contract_sha256());
}

#[test]
fn cognitive_roles_never_receive_operator_catalog_tools() {
    let catalog = load_embedded_investigation_tool_catalog().unwrap();
    for tool_config_id in INVESTIGATION_TOOL_CONFIG_IDS {
        let denied = admit_investigation_operator_tool(
            &catalog,
            &InvestigationToolAdmissionRequestV1 {
                tool_config_id: tool_config_id.to_string(),
                actor_is_cognitive: true,
                target_kind: "endpoint".to_string(),
                exact_scope_authority: true,
                target_write_guard_passed: true,
                fuel_reserved: true,
                lease_current: true,
                jit_authorized: true,
                exact_credential_grant: true,
                external_service_authorized: true,
                adapter_contract_id: "forged".to_string(),
                adapter_contract_digest: "a".repeat(64),
            },
        )
        .unwrap_err();
        assert_eq!(
            denied.reason_code,
            "cognitive_actor_has_no_operator_authority"
        );
    }
}

#[test]
fn catalog_members_without_typed_adapters_are_not_runnable() {
    let catalog = load_embedded_investigation_tool_catalog().unwrap();
    assert!(catalog.tools.iter().all(|profile| {
        profile.contract_status != InvestigationToolContractStatusV1::Ready
            && profile.typed_adapter.is_none()
    }));
    for profile in &catalog.tools {
        let denied = admit_investigation_operator_tool(
            &catalog,
            &InvestigationToolAdmissionRequestV1 {
                tool_config_id: profile.tool_config_id.clone(),
                actor_is_cognitive: false,
                target_kind: profile.target_kinds[0].clone(),
                exact_scope_authority: true,
                target_write_guard_passed: true,
                fuel_reserved: true,
                lease_current: true,
                jit_authorized: true,
                exact_credential_grant: true,
                external_service_authorized: true,
                adapter_contract_id: "forged".to_string(),
                adapter_contract_digest: "a".repeat(64),
            },
        )
        .unwrap_err();
        assert_eq!(denied.reason_code, "tool_contract_not_ready");
    }
}

#[test]
fn external_oast_and_stateful_fuzz_are_disabled_without_runtime_authority() {
    let catalog = load_embedded_investigation_tool_catalog().unwrap();
    for profile in &catalog.tools {
        if matches!(
            profile.execution_class,
            InvestigationToolExecutionClassV1::StatefulFuzz
                | InvestigationToolExecutionClassV1::ExternalOast
        ) {
            assert_eq!(
                profile.default_availability,
                InvestigationToolAvailabilityV1::Disabled
            );
            assert_eq!(
                profile.contract_status,
                InvestigationToolContractStatusV1::Disabled
            );
        }
    }
}

#[test]
fn malformed_catalog_policy_fails_closed() {
    let mut source: serde_json::Value =
        serde_json::from_str(investigation_tool_catalog_json()).unwrap();
    source["tools"][0]["tool_config_id"] = source["tools"][1]["tool_config_id"].clone();
    assert!(InvestigationOperatorToolCatalogV1::parse_and_validate(&source.to_string()).is_err());

    let mut source: serde_json::Value =
        serde_json::from_str(investigation_tool_catalog_json()).unwrap();
    let oast = source["tools"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|profile| profile["tool_config_id"] == "interactsh-client")
        .unwrap();
    oast["default_availability"] = serde_json::json!("jit_only");
    assert!(InvestigationOperatorToolCatalogV1::parse_and_validate(&source.to_string()).is_err());
}
