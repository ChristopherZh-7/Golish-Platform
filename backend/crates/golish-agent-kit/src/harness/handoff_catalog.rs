//! Closed, server-built StageHandoff catalog.
//!
//! Model deliverables may suggest canonical keys, but cannot provide trusted
//! timestamps, content hashes, ownership or a persisted handoff payload. This
//! module bounds those suggestions and constructs the typed final-seal command
//! consumed by the runtime-memory repository.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db_traits::{FinalizeUnitPass, RuntimeStageUnitStatus, RuntimeWorkerFence};

pub const MAX_CANONICAL_REFS: usize = 256;
pub const MAX_TYPED_CLAIMS: usize = 128;
pub const MAX_EVIDENCE_IDS: usize = 1024;
pub const MAX_HANDOFF_INPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanonicalFactKey {
    Organization {
        organization_id: Uuid,
    },
    Target {
        target_id: Uuid,
    },
    TargetAsset {
        target_asset_id: Uuid,
    },
    DnsRecord {
        organization_id: Uuid,
        domain: String,
        record_type: String,
        value: String,
    },
    ApiEndpoint {
        api_endpoint_id: Uuid,
    },
    DirectoryEntry {
        directory_entry_id: Uuid,
    },
    JsAnalysisResult {
        js_analysis_result_id: Uuid,
    },
    Fingerprint {
        fingerprint_id: Uuid,
    },
    TechniqueOutcome {
        organization_id: Uuid,
        run_id: String,
        asset: String,
        technique: String,
    },
    /// One immutable, server-frozen attack-candidate reasoning cell. The DB
    /// resolver binds this key back to the exact operation/org manifest before
    /// it can enter a handoff.
    AttackCandidateWorkItem {
        work_item_id: Uuid,
    },
    Finding {
        finding_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalFactRef {
    pub key: CanonicalFactKey,
    pub organization_id: Uuid,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub content_sha256: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TypedHandoffClaim {
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageHandoffPayload {
    pub canonical_fact_refs: Vec<CanonicalFactRef>,
    pub typed_claims: Vec<Value>,
    pub coverage_watermark: Value,
    pub evidence_ids: Vec<i64>,
}

/// Trusted inputs assembled by deterministic Gate/runtime code. This type does
/// not implement `Deserialize`; model JSON cannot construct a final seal.
#[derive(Debug, Clone)]
pub struct ServerFinalSealInput {
    pub fence: RuntimeWorkerFence,
    pub deliverable_submission_id: Uuid,
    pub expected_unit_row_version: i64,
    pub scope_hash: String,
    pub aggregate_pass_token_hash: Option<String>,
    pub canonical_fact_keys: Vec<CanonicalFactKey>,
    pub typed_claims: Vec<TypedHandoffClaim>,
    pub coverage_watermark: Value,
    pub evidence_ids: Vec<i64>,
    pub terminal_checkpoint: Value,
    pub deterministic_gate_details: Value,
    /// Candidate-only, server-derived acceptance material. Keeping it inside
    /// the bounded seal input makes the Gate hash bind the exact frozen
    /// manifest and every terminal decision; non-candidate stages use `None`.
    pub candidate_acceptance: Option<crate::harness::attack_execution::CandidateAcceptance>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandoffCatalogError {
    #[error("handoff catalog input is too large")]
    TooLarge,
    #[error("handoff catalog input is invalid: {0}")]
    Invalid(&'static str),
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize JSON key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Build a final-seal command entirely from server runtime/Gate state. The DB
/// repository still re-resolves every canonical key and independently verifies
/// all hashes and owner tuples under locks.
pub fn build_server_final_seal(
    input: ServerFinalSealInput,
) -> Result<FinalizeUnitPass, HandoffCatalogError> {
    if input.canonical_fact_keys.len() > MAX_CANONICAL_REFS
        || input.typed_claims.len() > MAX_TYPED_CLAIMS
        || input.evidence_ids.len() > MAX_EVIDENCE_IDS
    {
        return Err(HandoffCatalogError::TooLarge);
    }
    if input.scope_hash.trim().is_empty()
        || !input.coverage_watermark.is_object()
        || input.terminal_checkpoint.is_null()
        || !input.deterministic_gate_details.is_object()
        || input.evidence_ids.iter().any(|id| *id <= 0)
        || input
            .typed_claims
            .iter()
            .any(|claim| claim.kind.trim().is_empty() || !claim.payload.is_object())
    {
        return Err(HandoffCatalogError::Invalid("shape"));
    }
    let typed_claims = input
        .typed_claims
        .into_iter()
        .map(|claim| {
            serde_json::json!({
                "kind": claim.kind,
                "payload": claim.payload,
            })
        })
        .collect::<Vec<_>>();
    let bounded = serde_json::json!({
        "canonical_fact_keys": input.canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": input.coverage_watermark,
        "evidence_ids": input.evidence_ids,
        "terminal_checkpoint": input.terminal_checkpoint,
        "deterministic_gate_details": input.deterministic_gate_details,
        "candidate_acceptance": input.candidate_acceptance,
    });
    if canonical_json(&bounded).len() > MAX_HANDOFF_INPUT_BYTES {
        return Err(HandoffCatalogError::TooLarge);
    }
    let gate_decision = serde_json::json!({
        "outcome": "pass",
        "operation_id": input.fence.operation_id,
        "stage_execution_id": input.fence.stage_execution_id,
        "stage_run_unit_id": input.fence.stage_run_unit_id,
        "deliverable_submission_id": input.deliverable_submission_id,
        "scope_hash": input.scope_hash,
        "seal_material_sha256": sha256_json(&bounded),
        "details": bounded["deterministic_gate_details"].clone(),
    });
    Ok(FinalizeUnitPass {
        fence: input.fence,
        deliverable_submission_id: input.deliverable_submission_id,
        expected_unit_status: RuntimeStageUnitStatus::Running,
        expected_unit_row_version: input.expected_unit_row_version,
        scope_hash: bounded["scope_hash"]
            .as_str()
            .unwrap_or_else(|| gate_decision["scope_hash"].as_str().expect("scope hash"))
            .to_string(),
        gate_decision_hash: sha256_json(&gate_decision),
        gate_decision,
        aggregate_pass_token_hash: input.aggregate_pass_token_hash,
        canonical_fact_keys: serde_json::from_value(bounded["canonical_fact_keys"].clone())
            .expect("canonical fact keys roundtrip"),
        typed_claims,
        coverage_watermark: bounded["coverage_watermark"].clone(),
        evidence_ids: serde_json::from_value(bounded["evidence_ids"].clone())
            .expect("evidence ids roundtrip"),
        terminal_checkpoint: bounded["terminal_checkpoint"].clone(),
        candidate_acceptance: serde_json::from_value(bounded["candidate_acceptance"].clone())
            .expect("candidate acceptance roundtrip"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_catalog_is_closed_and_rejects_unknown_kind() {
        let finding_id = Uuid::new_v4();
        assert!(matches!(
            CanonicalFactKey::Finding { finding_id },
            CanonicalFactKey::Finding { finding_id: id } if id == finding_id
        ));
        assert!(
            serde_json::from_value::<CanonicalFactKey>(serde_json::json!({
                "kind": "future_model_fact",
                "id": Uuid::new_v4(),
            }))
            .is_err()
        );
    }

    #[test]
    fn server_builder_binds_gate_hash_to_exact_runtime_identity() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let deliverable_submission_id = Uuid::new_v4();
        let seal = build_server_final_seal(ServerFinalSealInput {
            fence: RuntimeWorkerFence {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                worker_run_id: Uuid::new_v4(),
                lease_token: Uuid::new_v4(),
                attempt_epoch: 1,
                expected_checkpoint_version: 3,
            },
            deliverable_submission_id,
            expected_unit_row_version: 2,
            scope_hash: "scope-sha".to_string(),
            aggregate_pass_token_hash: None,
            canonical_fact_keys: vec![CanonicalFactKey::Finding {
                finding_id: Uuid::new_v4(),
            }],
            typed_claims: vec![TypedHandoffClaim {
                kind: "coverage_complete".to_string(),
                payload: serde_json::json!({"terminal": true}),
            }],
            coverage_watermark: serde_json::json!({"cells": 1}),
            evidence_ids: vec![7],
            terminal_checkpoint: serde_json::json!({"terminal": true}),
            deterministic_gate_details: serde_json::json!({"rules": ["coverage"]}),
            candidate_acceptance: None,
        })
        .expect("build bounded server seal");
        assert_eq!(seal.fence.operation_id, operation_id);
        assert_eq!(seal.deliverable_submission_id, deliverable_submission_id);
        assert_eq!(seal.gate_decision["outcome"], "pass");
        assert_eq!(seal.gate_decision_hash.len(), 64);
    }

    #[test]
    fn candidate_acceptance_is_inside_the_hashed_final_seal_material() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let worker_run_id = Uuid::new_v4();
        let lease_token = Uuid::new_v4();
        let deliverable_submission_id = Uuid::new_v4();
        let work_item_id = Uuid::new_v4();
        let candidate_id = Uuid::new_v4();
        let acceptance = |rationale: &str| {
            serde_json::from_value::<crate::harness::attack_execution::CandidateAcceptance>(
                serde_json::json!({
                    "wave_run_id": Uuid::new_v4(),
                    "wave_unit_id": Uuid::new_v4(),
                    "manifest_hash": "sha256:manifest",
                    "expected_work_item_ids": [work_item_id],
                    "candidates": [{
                        "candidate_id": candidate_id,
                        "work_item_id": work_item_id,
                        "hypothesis": "bounded hypothesis",
                        "technique": "WSTG-INPV-05",
                        "rationale": rationale,
                        "prior_refs": ["audit:7"],
                        "suggested_approach": "bounded_probe",
                        "priority": "high",
                        "execution_plan": {
                            "schema_version": "candidate-plan-v1",
                            "classifier_version": "candidate-classifier-v1",
                            "candidate_id": candidate_id,
                            "target_identity_hash": "sha256:target",
                            "actions": [],
                            "budget": {
                                "max_actions": 1,
                                "max_requests": 1,
                                "max_runtime_ms": 1000
                            },
                            "foreground_only": true
                        },
                        "candidate_plan_hash": "sha256:plan",
                        "risk_class": "exploit",
                        "evidence_ids": [7]
                    }],
                    "no_candidate_decisions": []
                }),
            )
            .expect("deserialize server Candidate acceptance fixture")
        };
        let build = |candidate_acceptance| {
            build_server_final_seal(ServerFinalSealInput {
                fence: RuntimeWorkerFence {
                    operation_id,
                    stage_execution_id,
                    stage_run_unit_id,
                    worker_run_id,
                    lease_token,
                    attempt_epoch: 1,
                    expected_checkpoint_version: 0,
                },
                deliverable_submission_id,
                expected_unit_row_version: 0,
                scope_hash: "scope-sha".to_string(),
                aggregate_pass_token_hash: None,
                canonical_fact_keys: vec![CanonicalFactKey::AttackCandidateWorkItem {
                    work_item_id,
                }],
                typed_claims: vec![],
                coverage_watermark: serde_json::json!({"kind": "candidate_manifest_v1"}),
                evidence_ids: vec![7],
                terminal_checkpoint: serde_json::json!({"terminal": true}),
                deterministic_gate_details: serde_json::json!({
                    "source": "authoritative_org_gate"
                }),
                candidate_acceptance: Some(candidate_acceptance),
            })
            .expect("build Candidate final seal")
        };
        let first = build(acceptance("first rationale"));
        let drifted = build(acceptance("drifted rationale"));
        assert_ne!(first.gate_decision_hash, drifted.gate_decision_hash);
        assert_ne!(
            first.gate_decision["seal_material_sha256"],
            drifted.gate_decision["seal_material_sha256"]
        );
        assert!(first.candidate_acceptance.is_some());
    }
}
