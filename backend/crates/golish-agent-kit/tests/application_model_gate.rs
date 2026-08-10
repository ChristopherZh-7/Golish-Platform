use golish_agent_kit::harness::{
    validate_application_model_gate_truth, ApplicationModelAuthorityKind, ApplicationModelGateCode,
    ApplicationModelGateDisposition, ApplicationModelGateSnapshot,
    ApplicationModelInputDecisionTruth, ApplicationModelInputDisposition,
    ApplicationModelItemTruth, ApplicationModelTruthState,
};
use uuid::Uuid;

fn tagged_hash(hex_digit: char) -> String {
    format!("sha256:{}", hex_digit.to_string().repeat(64))
}

fn valid_snapshot() -> ApplicationModelGateSnapshot {
    ApplicationModelGateSnapshot {
        authority_kind: ApplicationModelAuthorityKind::Model,
        operation_id: Uuid::new_v4(),
        scope_snapshot_id: Uuid::new_v4(),
        stage_execution_id: Uuid::new_v4(),
        stage_run_unit_id: Uuid::new_v4(),
        organization_id: Uuid::new_v4(),
        manifest_hash: tagged_hash('a'),
        expected_manifest_hash: tagged_hash('a'),
        schema_version: Some("application_model.v1".to_string()),
        model_hash: Some(tagged_hash('b')),
        expected_model_hash: Some(tagged_hash('b')),
        replay_material_hash: tagged_hash('c'),
        expected_replay_material_hash: tagged_hash('c'),
        manifest_input_keys: vec!["api:/orders/{id}".to_string()],
        authorized_evidence_ids: vec![41],
        decisions: vec![ApplicationModelInputDecisionTruth {
            input_key: "api:/orders/{id}".to_string(),
            disposition: ApplicationModelInputDisposition::Incorporated,
            item_keys: vec!["workflow:order_read".to_string()],
            duplicate_input_key: None,
            reason_code: None,
        }],
        items: vec![ApplicationModelItemTruth {
            item_key: "workflow:order_read".to_string(),
            truth_state: ApplicationModelTruthState::Observed,
            source_input_keys: vec!["api:/orders/{id}".to_string()],
            evidence_ids: vec![41],
            observed_evidence_ids: vec![41],
            referenced_item_keys: Vec::new(),
        }],
        foreign_reference_keys: Vec::new(),
        forbidden_activity_refs: Vec::new(),
        pending_producer_refs: Vec::new(),
    }
}

#[test]
fn application_model_gate_observed_model_with_exact_closure_passes() {
    assert_eq!(
        validate_application_model_gate_truth(&valid_snapshot()),
        Ok(())
    );
}

#[test]
fn application_model_gate_missing_input_decision_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.decisions.clear();

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(
        block.code,
        ApplicationModelGateCode::InputCoverageIncomplete
    );
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
    assert_eq!(block.refs, vec!["api:/orders/{id}"]);
}

#[test]
fn application_model_gate_foreign_reference_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot
        .foreign_reference_keys
        .push("evidence:foreign:99".to_string());

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ForeignReference);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(block.refs, vec!["evidence:foreign:99"]);
}

#[test]
fn application_model_gate_non_positive_observed_evidence_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.authorized_evidence_ids = vec![0];
    snapshot.items[0].evidence_ids = vec![0];
    snapshot.items[0].observed_evidence_ids = vec![0];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(
        block.code,
        ApplicationModelGateCode::ObservedEvidenceMissing
    );
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
    assert_eq!(block.refs, vec!["workflow:order_read"]);
}

#[test]
fn application_model_gate_inferred_item_cannot_claim_observed_proof() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].truth_state = ApplicationModelTruthState::Inferred;

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::TruthStateConflict);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
    assert_eq!(block.refs, vec!["workflow:order_read"]);
}

#[test]
fn application_model_gate_nil_identity_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.operation_id = Uuid::nil();

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::IdentityMismatch);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(block.refs, vec!["operation_id"]);
}

#[test]
fn application_model_gate_manifest_drift_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.expected_manifest_hash = tagged_hash('d');

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ManifestDrift);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
}

#[test]
fn application_model_gate_replay_drift_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.expected_replay_material_hash = tagged_hash('d');

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ReplayDrift);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
}

#[test]
fn application_model_gate_identity_hashes_require_lowercase_tagged_sha256() {
    type HashSetter = fn(&mut ApplicationModelGateSnapshot, String);

    let fields: [(&str, HashSetter); 4] = [
        ("manifest_hash", |snapshot, value| {
            snapshot.manifest_hash = value
        }),
        ("expected_manifest_hash", |snapshot, value| {
            snapshot.expected_manifest_hash = value;
        }),
        ("replay_material_hash", |snapshot, value| {
            snapshot.replay_material_hash = value;
        }),
        ("expected_replay_material_hash", |snapshot, value| {
            snapshot.expected_replay_material_hash = value;
        }),
    ];
    let invalid_hashes = [
        "sha256:abc".to_string(),
        format!("sha256:{}", "A".repeat(64)),
        "a".repeat(64),
    ];

    for (field, set_hash) in fields {
        for invalid_hash in &invalid_hashes {
            let mut snapshot = valid_snapshot();
            set_hash(&mut snapshot, invalid_hash.clone());

            let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

            assert_eq!(block.code, ApplicationModelGateCode::IdentityMismatch);
            assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
            assert_eq!(block.refs, vec![field], "accepted invalid {field}");
        }
    }
}

#[test]
fn application_model_gate_model_hashes_require_lowercase_tagged_sha256() {
    type HashSetter = fn(&mut ApplicationModelGateSnapshot, String);

    let fields: [(&str, HashSetter); 2] = [
        ("model_hash", |snapshot, value| {
            snapshot.model_hash = Some(value)
        }),
        ("expected_model_hash", |snapshot, value| {
            snapshot.expected_model_hash = Some(value);
        }),
    ];
    let invalid_hashes = [
        "sha256:abc".to_string(),
        format!("sha256:{}", "B".repeat(64)),
        "b".repeat(64),
    ];

    for (field, set_hash) in fields {
        for invalid_hash in &invalid_hashes {
            let mut snapshot = valid_snapshot();
            set_hash(&mut snapshot, invalid_hash.clone());

            let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

            assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
            assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
            assert_eq!(block.refs, vec![field], "accepted invalid {field}");
        }
    }
}

#[test]
fn application_model_gate_duplicate_input_decision_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.decisions.push(snapshot.decisions[0].clone());

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(
        block.code,
        ApplicationModelGateCode::InputCoverageIncomplete
    );
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_missing_referenced_item_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.decisions[0].item_keys = vec!["workflow:missing".to_string()];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_observed_item_without_proof_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].evidence_ids.clear();
    snapshot.items[0].observed_evidence_ids.clear();

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(
        block.code,
        ApplicationModelGateCode::ObservedEvidenceMissing
    );
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_unauthorized_evidence_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].evidence_ids = vec![99];
    snapshot.items[0].observed_evidence_ids = vec![99];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ForeignReference);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
    assert_eq!(block.refs, vec!["99"]);
}

#[test]
fn application_model_gate_item_without_manifest_source_is_truth_conflict() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].source_input_keys.clear();

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::TruthStateConflict);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_forbidden_activity_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.forbidden_activity_refs = vec!["browser:open".to_string()];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ForbiddenToolActivity);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
}

#[test]
fn application_model_gate_pending_producer_is_hold() {
    let mut snapshot = valid_snapshot();
    snapshot.pending_producer_refs = vec!["worker:leased:7".to_string()];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::ProducerBarrierOpen);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Hold);
}

#[test]
fn application_model_gate_all_unknown_with_stable_reason_passes() {
    let mut snapshot = valid_snapshot();
    snapshot.authorized_evidence_ids.clear();
    snapshot.items.clear();
    snapshot.decisions[0] = ApplicationModelInputDecisionTruth {
        input_key: "api:/orders/{id}".to_string(),
        disposition: ApplicationModelInputDisposition::Unknown,
        item_keys: Vec::new(),
        duplicate_input_key: None,
        reason_code: Some("insufficient_recon_evidence".to_string()),
    };

    assert_eq!(validate_application_model_gate_truth(&snapshot), Ok(()));
}

#[test]
fn application_model_gate_unknown_with_free_text_reason_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.authorized_evidence_ids.clear();
    snapshot.items.clear();
    snapshot.decisions[0] = ApplicationModelInputDecisionTruth {
        input_key: "api:/orders/{id}".to_string(),
        disposition: ApplicationModelInputDisposition::Unknown,
        item_keys: Vec::new(),
        duplicate_input_key: None,
        reason_code: Some("Not enough evidence".to_string()),
    };

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_terminal_no_input_passes() {
    let mut snapshot = valid_snapshot();
    snapshot.authority_kind = ApplicationModelAuthorityKind::TerminalNoInput;
    snapshot.schema_version = None;
    snapshot.model_hash = None;
    snapshot.expected_model_hash = None;
    snapshot.manifest_input_keys.clear();
    snapshot.authorized_evidence_ids.clear();
    snapshot.decisions.clear();
    snapshot.items.clear();

    assert_eq!(validate_application_model_gate_truth(&snapshot), Ok(()));
}

#[test]
fn application_model_gate_terminal_no_input_rejects_model_payload() {
    let mut snapshot = valid_snapshot();
    snapshot.authority_kind = ApplicationModelAuthorityKind::TerminalNoInput;

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_block_refs_are_sorted_and_deduplicated() {
    let mut snapshot = valid_snapshot();
    snapshot.foreign_reference_keys = vec![
        "z-ref".to_string(),
        "a-ref".to_string(),
        "z-ref".to_string(),
    ];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.refs, vec!["a-ref", "z-ref"]);
}

#[test]
fn application_model_gate_codes_have_stable_wire_names() {
    assert_eq!(
        ApplicationModelGateCode::TruthStateConflict.as_str(),
        "APPLICATION_MODEL_TRUTH_STATE_CONFLICT"
    );
    assert_eq!(
        serde_json::to_string(&ApplicationModelGateCode::TruthStateConflict).unwrap(),
        "\"truth_state_conflict\""
    );
}

#[test]
fn application_model_gate_duplicate_item_key_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.items.push(snapshot.items[0].clone());

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(block.disposition, ApplicationModelGateDisposition::Rework);
}

#[test]
fn application_model_gate_invalid_internal_item_reference_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].referenced_item_keys = vec!["workflow:missing".to_string()];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
}

#[test]
fn application_model_gate_incorporated_decision_requires_an_item() {
    let mut snapshot = valid_snapshot();
    snapshot.decisions[0].item_keys.clear();

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
}

#[test]
fn application_model_gate_observation_role_must_reference_item_evidence() {
    let mut snapshot = valid_snapshot();
    snapshot.items[0].observed_evidence_ids = vec![42];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
}

#[test]
fn application_model_gate_orphan_item_is_rework() {
    let mut snapshot = valid_snapshot();
    snapshot.items.push(ApplicationModelItemTruth {
        item_key: "workflow:orphan".to_string(),
        truth_state: ApplicationModelTruthState::Inferred,
        source_input_keys: vec!["api:/orders/{id}".to_string()],
        evidence_ids: vec![41],
        observed_evidence_ids: Vec::new(),
        referenced_item_keys: Vec::new(),
    });

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::SchemaInvalid);
    assert_eq!(block.refs, vec!["workflow:orphan"]);
}

#[test]
fn application_model_gate_incorporated_item_must_trace_to_that_input() {
    let mut snapshot = valid_snapshot();
    snapshot
        .manifest_input_keys
        .push("api:/customers/{id}".to_string());
    snapshot.decisions.push(ApplicationModelInputDecisionTruth {
        input_key: "api:/customers/{id}".to_string(),
        disposition: ApplicationModelInputDisposition::Unknown,
        item_keys: Vec::new(),
        duplicate_input_key: None,
        reason_code: Some("not_enough_context".to_string()),
    });
    snapshot.items[0].source_input_keys = vec!["api:/customers/{id}".to_string()];

    let block = validate_application_model_gate_truth(&snapshot).unwrap_err();

    assert_eq!(block.code, ApplicationModelGateCode::TruthStateConflict);
    assert_eq!(block.refs, vec!["api:/orders/{id}"]);
}
