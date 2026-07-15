//! Versioned immutable Candidate classifier registry and canonical plan hashing.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::state::AttackExecutionError;
use super::types::{
    AttemptEvidenceRole, CandidateBudget, CandidateClassificationInput, CandidateExecutionPlan,
    CandidateTargetClass, PlannedCandidateAction, SideEffectClass, VerificationRiskClass,
    CANDIDATE_CLASSIFIER_VERSION_V2, CANDIDATE_EXECUTOR_CONTRACT_ANONYMOUS_REPLAY_V2,
    CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
    CANDIDATE_EXECUTOR_CONTRACT_NUCLEI_REPLAY_V2, CANDIDATE_PLAN_SCHEMA_V2,
    CANDIDATE_RECIPE_VERSION_ANONYMOUS_REPLAY_V2,
    CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2, CANDIDATE_RECIPE_VERSION_NUCLEI_REPLAY_V2,
};

const WEB_TARGETS: &[CandidateTargetClass] = &[
    CandidateTargetClass::Url,
    CandidateTargetClass::Domain,
    CandidateTargetClass::Other,
];
const HOST_TARGETS: &[CandidateTargetClass] = &[
    CandidateTargetClass::Url,
    CandidateTargetClass::Domain,
    CandidateTargetClass::Ip,
    CandidateTargetClass::Other,
];

pub const VERIFICATION_CAPABILITY_IDS: &[&str] = &[
    "verify.nuclei_template_replay",
    "verify.anonymous_request_replay",
    "verify.directory_entry_replay",
    "verify.sql_injection",
    "verify.input_validation",
    "verify.command_injection",
    "verify.authorization",
    "verify.authentication",
    "verify.session_management",
    "verify.security_configuration",
    "verify.transport_security",
    "verify.information_disclosure",
    "verify.known_vulnerability",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateClassifierRecipe {
    pub technique: &'static str,
    pub target_classes: &'static [CandidateTargetClass],
    pub risk_class: VerificationRiskClass,
    pub capability_id: &'static str,
    pub action_kind: &'static str,
    pub side_effect_class: SideEffectClass,
    pub required_evidence_role: AttemptEvidenceRole,
    pub budget: CandidateBudget,
}

const ACTIVE_BUDGET: CandidateBudget = CandidateBudget {
    max_actions: 1,
    max_requests: 8,
    max_runtime_ms: 120_000,
};

/// Immutable, auditable v1 registry. It is deliberately a static table rather
/// than model output or environment configuration.
pub const CANDIDATE_CLASSIFIER_REGISTRY_V1: &[CandidateClassifierRecipe] = &[
    CandidateClassifierRecipe {
        technique: "WSTG-INPV-05",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::Exploit,
        capability_id: "verify.sql_injection",
        action_kind: "bounded_sql_injection_probe",
        side_effect_class: SideEffectClass::Exploit,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-INPV-01",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::ActiveSafe,
        capability_id: "verify.input_validation",
        action_kind: "bounded_input_reflection_probe",
        side_effect_class: SideEffectClass::ActiveProbe,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-INPV-12",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::Exploit,
        capability_id: "verify.command_injection",
        action_kind: "bounded_command_injection_probe",
        side_effect_class: SideEffectClass::Exploit,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-ATHN-04",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::ActiveSafe,
        capability_id: "verify.authorization",
        action_kind: "bounded_authorization_probe",
        side_effect_class: SideEffectClass::ActiveProbe,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-ATHN-02",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::ActiveSafe,
        capability_id: "verify.authentication",
        action_kind: "bounded_authentication_probe",
        side_effect_class: SideEffectClass::ActiveProbe,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-SESS-02",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::ActiveSafe,
        capability_id: "verify.session_management",
        action_kind: "bounded_session_probe",
        side_effect_class: SideEffectClass::ActiveProbe,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-CONF-05",
        target_classes: HOST_TARGETS,
        risk_class: VerificationRiskClass::DeterministicSafe,
        capability_id: "verify.security_configuration",
        action_kind: "inspect_security_configuration",
        side_effect_class: SideEffectClass::ReadOnly,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-CRYP-03",
        target_classes: HOST_TARGETS,
        risk_class: VerificationRiskClass::DeterministicSafe,
        capability_id: "verify.transport_security",
        action_kind: "inspect_transport_security",
        side_effect_class: SideEffectClass::ReadOnly,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "WSTG-INFO",
        target_classes: WEB_TARGETS,
        risk_class: VerificationRiskClass::DeterministicSafe,
        capability_id: "verify.information_disclosure",
        action_kind: "inspect_information_disclosure",
        side_effect_class: SideEffectClass::ReadOnly,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
    CandidateClassifierRecipe {
        technique: "GOLISH-NDAY",
        target_classes: HOST_TARGETS,
        risk_class: VerificationRiskClass::Exploit,
        capability_id: "verify.known_vulnerability",
        action_kind: "bounded_known_vulnerability_probe",
        side_effect_class: SideEffectClass::Exploit,
        required_evidence_role: AttemptEvidenceRole::Proof,
        budget: ACTIVE_BUDGET,
    },
];

pub fn classifier_recipe_for(
    technique: &str,
    target_class: CandidateTargetClass,
) -> Option<&'static CandidateClassifierRecipe> {
    CANDIDATE_CLASSIFIER_REGISTRY_V1.iter().find(|recipe| {
        recipe.technique == technique && recipe.target_classes.contains(&target_class)
    })
}

/// Stable technique allowlist for a surface-analysis work item. The order is
/// the immutable v1 registry order; callers must not derive this from model
/// prose or a mutable environment registry.
pub fn supported_candidate_techniques(target_class: CandidateTargetClass) -> Vec<&'static str> {
    CANDIDATE_CLASSIFIER_REGISTRY_V1
        .iter()
        .filter(|recipe| recipe.target_classes.contains(&target_class))
        .map(|recipe| recipe.technique)
        .collect()
}

pub fn classify_candidate(
    candidate: &CandidateClassificationInput,
) -> Result<CandidateExecutionPlan, AttackExecutionError> {
    if candidate.candidate_id.is_nil()
        || candidate.target_identity_hash.trim().is_empty()
        || candidate.target_value.trim().is_empty()
        || candidate.hypothesis.trim().is_empty()
        || !candidate.observation.is_object()
        || serde_json::to_vec(&candidate.observation).map_or(true, |bytes| bytes.len() > 64 * 1024)
        || candidate.observation_hash.trim().is_empty()
        || candidate.observation_hash.len() > 128
    {
        return Err(AttackExecutionError::new(
            "ATTACK_CANDIDATE_IDENTITY_INVALID",
            "candidate and frozen target identity are required",
        ));
    }
    let recipe = classifier_recipe_for(candidate.technique.trim(), candidate.target_class)
        .ok_or_else(|| {
            AttackExecutionError::new(
                "ATTACK_CAPABILITY_UNSUPPORTED",
                format!(
                    "no v1 verifier recipe for technique={} target_class={:?}",
                    candidate.technique, candidate.target_class
                ),
            )
        })?;

    validate_observation_hash(&candidate.observation, candidate.observation_hash.trim())?;

    let mut prior_refs = candidate
        .prior_refs
        .iter()
        .map(|reference| reference.trim())
        .filter(|reference| !reference.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    prior_refs.sort();
    prior_refs.dedup();

    let (capability_id, action_kind, recipe_version, executor_contract_version, canonical_args) =
        match candidate.observation.get("schema").and_then(Value::as_str) {
            Some("nuclei_match_v1") => {
                let target_id = required_observation_uuid(&candidate.observation, "target_id")?;
                let expected_target_id = candidate
                    .target_live_id
                    .filter(|id| !id.is_nil())
                    .ok_or_else(|| {
                        observation_identity_mismatch(
                            "Nuclei replay requires a frozen live target id",
                        )
                    })?;
                let technique = required_observation_str(&candidate.observation, "technique")?;
                if target_id != expected_target_id || technique != recipe.technique {
                    return Err(observation_identity_mismatch(
                        "Nuclei observation target or technique differs from the frozen Candidate",
                    ));
                }
                let matched_url = required_observation_str(&candidate.observation, "matched_url")?;
                let template_id = required_observation_str(&candidate.observation, "template_id")?;
                let matched_origin = golish_pentest_domain::canonical_web_origin(matched_url)
                    .ok_or_else(|| {
                        observation_invalid("Nuclei matched_url is not an HTTP(S) URL")
                    })?;
                let candidate_origin =
                    golish_pentest_domain::canonical_web_origin(candidate.target_value.trim())
                        .ok_or_else(|| {
                            observation_identity_mismatch(
                                "Nuclei Candidate target is not an exact HTTP(S) origin",
                            )
                        })?;
                if matched_origin.key != candidate_origin.key {
                    return Err(observation_identity_mismatch(
                        "Nuclei matched_url escaped the frozen Candidate origin",
                    ));
                }
                if !safe_template_id(template_id) {
                    return Err(observation_invalid(
                        "Nuclei observation carries an invalid template id",
                    ));
                }
                (
                    "verify.nuclei_template_replay",
                    "nuclei_template_replay",
                    CANDIDATE_RECIPE_VERSION_NUCLEI_REPLAY_V2,
                    CANDIDATE_EXECUTOR_CONTRACT_NUCLEI_REPLAY_V2,
                    canonicalize_json(&json!({
                        "background": false,
                        "executor_contract_version": CANDIDATE_EXECUTOR_CONTRACT_NUCLEI_REPLAY_V2,
                        "hypothesis": candidate.hypothesis.trim(),
                        "matched_url": matched_url,
                        "observation": candidate.observation.clone(),
                        "observation_hash": candidate.observation_hash.trim(),
                        "prior_refs": prior_refs,
                        "recipe_version": CANDIDATE_RECIPE_VERSION_NUCLEI_REPLAY_V2,
                        "target": candidate.target_value.trim(),
                        "target_id": target_id,
                        "template_id": template_id,
                        "technique": recipe.technique,
                    })),
                )
            }
            Some("anonymous_access_v1") => {
                if recipe.technique != "WSTG-ATHN-04" {
                    return Err(observation_identity_mismatch(
                        "anonymous-access observation requires the frozen WSTG-ATHN-04 technique",
                    ));
                }
                let target_id = candidate
                    .target_live_id
                    .filter(|id| !id.is_nil())
                    .ok_or_else(|| {
                        observation_identity_mismatch(
                            "anonymous-access replay requires a frozen live target id",
                        )
                    })?;
                let endpoint_id = required_observation_uuid(&candidate.observation, "endpoint_id")?;
                let endpoint_row_sha256 =
                    required_observation_str(&candidate.observation, "endpoint_row_sha256")?;
                let request_plan_sha256 =
                    required_observation_str(&candidate.observation, "request_plan_sha256")?;
                let method = required_observation_str(&candidate.observation, "method")?;
                let path = required_observation_str(&candidate.observation, "path")?;
                let query_bindings = candidate
                    .observation
                    .get("query_bindings")
                    .filter(|value| valid_query_bindings(value))
                    .ok_or_else(|| {
                        observation_invalid("anonymous-access query bindings are malformed")
                    })?;
                if !valid_sha256(endpoint_row_sha256)
                    || !valid_sha256(request_plan_sha256)
                    || !matches!(method, "GET" | "HEAD")
                    || !valid_request_path(path)
                    || candidate
                        .observation
                        .get("no_auth")
                        .and_then(Value::as_bool)
                        != Some(true)
                    || candidate
                        .observation
                        .get("network_attempted")
                        .and_then(Value::as_bool)
                        != Some(true)
                    || candidate.observation.get("verdict").and_then(Value::as_str)
                        != Some("suspicious")
                    || candidate
                        .observation
                        .get("authority_current_after")
                        .and_then(Value::as_bool)
                        != Some(true)
                {
                    return Err(observation_invalid(
                        "anonymous-access observation is not a current safe suspicious replay",
                    ));
                }
                (
                    "verify.anonymous_request_replay",
                    "anonymous_request_replay",
                    CANDIDATE_RECIPE_VERSION_ANONYMOUS_REPLAY_V2,
                    CANDIDATE_EXECUTOR_CONTRACT_ANONYMOUS_REPLAY_V2,
                    canonicalize_json(&json!({
                        "background": false,
                        "endpoint_id": endpoint_id,
                        "endpoint_row_sha256": endpoint_row_sha256,
                        "executor_contract_version": CANDIDATE_EXECUTOR_CONTRACT_ANONYMOUS_REPLAY_V2,
                        "hypothesis": candidate.hypothesis.trim(),
                        "method": method,
                        "no_auth": true,
                        "observation": candidate.observation.clone(),
                        "observation_hash": candidate.observation_hash.trim(),
                        "path": path,
                        "prior_refs": prior_refs,
                        "query_bindings": query_bindings,
                        "recipe_version": CANDIDATE_RECIPE_VERSION_ANONYMOUS_REPLAY_V2,
                        "request_plan_sha256": request_plan_sha256,
                        "target": candidate.target_value.trim(),
                        "target_id": target_id,
                        "technique": recipe.technique,
                    })),
                )
            }
            Some("directory_entry_observation_v1") => {
                if recipe.technique != "WSTG-INFO" {
                    return Err(observation_identity_mismatch(
                        "directory-entry observation requires the frozen WSTG-INFO technique",
                    ));
                }
                let expected_target_id = candidate
                    .target_live_id
                    .filter(|id| !id.is_nil())
                    .ok_or_else(|| {
                        observation_identity_mismatch(
                            "directory-entry replay requires a frozen live target id",
                        )
                    })?;
                let target_id = required_observation_uuid(&candidate.observation, "target_id")?;
                if target_id != expected_target_id {
                    return Err(observation_identity_mismatch(
                        "directory-entry observation target differs from the frozen Candidate",
                    ));
                }
                let directory_entry_id =
                    required_observation_uuid(&candidate.observation, "directory_entry_id")?;
                let directory_entry_row_sha256 =
                    required_observation_str(&candidate.observation, "directory_entry_row_sha256")?;
                let url = required_observation_str(&candidate.observation, "url")?;
                let method = required_observation_str(&candidate.observation, "method")?;
                let source_tool = required_observation_str(&candidate.observation, "source_tool")?;
                let status_code = candidate
                    .observation
                    .get("status_code")
                    .and_then(Value::as_i64)
                    .filter(|status| (200..=299).contains(status))
                    .ok_or_else(|| {
                        observation_invalid(
                            "directory-entry observation requires an exact successful status",
                        )
                    })?;
                let content_length = candidate
                    .observation
                    .get("content_length")
                    .and_then(Value::as_i64)
                    .filter(|length| (0..=i32::MAX as i64).contains(length))
                    .ok_or_else(|| {
                        observation_invalid(
                            "directory-entry observation has invalid content length",
                        )
                    })?;
                let content_type = candidate
                    .observation
                    .get("content_type")
                    .and_then(Value::as_str)
                    .filter(|value| {
                        value.len() <= 256
                            && !value
                                .chars()
                                .any(|character| matches!(character, '\0' | '\r' | '\n'))
                    })
                    .ok_or_else(|| {
                        observation_invalid("directory-entry observation has invalid content type")
                    })?;
                let source_evidence_id = candidate
                    .observation
                    .get("source_evidence_id")
                    .and_then(Value::as_i64)
                    .filter(|id| *id > 0)
                    .ok_or_else(|| {
                        observation_invalid(
                            "directory-entry observation requires exact producer evidence",
                        )
                    })?;
                let source_evidence_ref = format!("audit:{source_evidence_id}");
                if method != "GET"
                    || source_tool != "route_probe"
                    || candidate
                        .observation
                        .get("network_attempted")
                        .and_then(Value::as_bool)
                        != Some(true)
                    || candidate
                        .observation
                        .get("authority_current_after")
                        .and_then(Value::as_bool)
                        != Some(true)
                    || !prior_refs
                        .iter()
                        .any(|reference| reference == &source_evidence_ref)
                {
                    return Err(observation_invalid(
                        "directory-entry observation lacks its exact safe producer contract",
                    ));
                }
                validate_directory_entry_url(candidate.target_value.trim(), url)?;
                validate_directory_entry_row_hash(
                    directory_entry_id,
                    target_id,
                    url,
                    status_code,
                    content_length,
                    content_type,
                    source_tool,
                    directory_entry_row_sha256,
                )?;
                (
                    "verify.directory_entry_replay",
                    "directory_entry_replay",
                    CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2,
                    CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
                    canonicalize_json(&json!({
                        "authority_current_after": true,
                        "background": false,
                        "content_length": content_length,
                        "content_type": content_type,
                        "directory_entry_id": directory_entry_id,
                        "directory_entry_row_sha256": directory_entry_row_sha256,
                        "executor_contract_version": CANDIDATE_EXECUTOR_CONTRACT_DIRECTORY_ENTRY_REPLAY_V2,
                        "follow_redirects": false,
                        "hypothesis": candidate.hypothesis.trim(),
                        "method": "GET",
                        "network_attempted": true,
                        "no_auth": true,
                        "observation": candidate.observation.clone(),
                        "observation_hash": candidate.observation_hash.trim(),
                        "prior_refs": prior_refs,
                        "recipe_version": CANDIDATE_RECIPE_VERSION_DIRECTORY_ENTRY_REPLAY_V2,
                        "source_evidence_id": source_evidence_id,
                        "source_tool": source_tool,
                        "status_code": status_code,
                        "target": candidate.target_value.trim(),
                        "target_id": target_id,
                        "technique": recipe.technique,
                        "url": url,
                    })),
                )
            }
            Some(schema @ ("surface_analysis_v1" | "surface_analysis_v2")) => {
                validate_surface_analysis_identity(candidate)?;
                if schema == "surface_analysis_v2" {
                    return Err(AttackExecutionError::new(
                        "ATTACK_FACT_DELTA_ENRICHMENT_REQUIRED",
                        "delta-local surface analysis must finish with new typed evidence before a Candidate can be classified",
                    ));
                }
                return Err(AttackExecutionError::new(
                    "ATTACK_EXECUTOR_CONTRACT_UNAVAILABLE",
                    format!(
                        "technique={} has no typed V2 verifier adapter; legacy generic execution is quarantined",
                        recipe.technique
                    ),
                ));
            }
            _ => {
                return Err(AttackExecutionError::new(
                    "ATTACK_OBSERVATION_SCHEMA_UNSUPPORTED",
                    "Candidate observation schema has no immutable verifier classifier",
                ));
            }
        };
    Ok(CandidateExecutionPlan {
        schema_version: CANDIDATE_PLAN_SCHEMA_V2.to_string(),
        classifier_version: CANDIDATE_CLASSIFIER_VERSION_V2.to_string(),
        recipe_version: recipe_version.to_string(),
        executor_contract_version: executor_contract_version.to_string(),
        candidate_id: candidate.candidate_id,
        target_identity_hash: candidate.target_identity_hash.trim().to_string(),
        actions: vec![PlannedCandidateAction {
            ordinal: 0,
            capability_id: capability_id.to_string(),
            action_kind: action_kind.to_string(),
            recipe_version: recipe_version.to_string(),
            executor_contract_version: executor_contract_version.to_string(),
            canonical_args,
            side_effect_class: recipe.side_effect_class,
            required_evidence_role: recipe.required_evidence_role,
        }],
        budget: recipe.budget,
        foreground_only: true,
    })
}

fn observation_invalid(message: impl Into<String>) -> AttackExecutionError {
    AttackExecutionError::new("ATTACK_OBSERVATION_INVALID", message)
}

fn observation_identity_mismatch(message: impl Into<String>) -> AttackExecutionError {
    AttackExecutionError::new("ATTACK_OBSERVATION_IDENTITY_MISMATCH", message)
}

fn required_observation_str<'a>(
    observation: &'a Value,
    field: &str,
) -> Result<&'a str, AttackExecutionError> {
    observation
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| observation_invalid(format!("Candidate observation is missing {field}")))
}

fn required_observation_uuid(
    observation: &Value,
    field: &str,
) -> Result<uuid::Uuid, AttackExecutionError> {
    let value = required_observation_str(observation, field)?;
    uuid::Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or_else(|| observation_invalid(format!("Candidate observation has invalid {field}")))
}

fn validate_observation_hash(
    observation: &Value,
    declared_hash: &str,
) -> Result<(), AttackExecutionError> {
    let bytes = serde_json::to_vec(&canonicalize_json(observation)).map_err(|error| {
        observation_invalid(format!("serialize frozen Candidate observation: {error}"))
    })?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if declared_hash != format!("sha256:{digest}") {
        return Err(AttackExecutionError::new(
            "ATTACK_OBSERVATION_HASH_MISMATCH",
            "Candidate observation bytes do not match the frozen observation hash",
        ));
    }
    Ok(())
}

fn safe_template_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_sha256(value: &str) -> bool {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_request_path(value: &str) -> bool {
    value.starts_with('/') && value.len() <= 2_048 && !value.contains('?') && !value.contains('#')
}

fn valid_query_bindings(value: &Value) -> bool {
    let Some(bindings) = value.as_array() else {
        return false;
    };
    if bindings.len() > 16 {
        return false;
    }
    let mut names = std::collections::BTreeSet::new();
    bindings.iter().all(|binding| {
        let Some(binding) = binding.as_object().filter(|binding| binding.len() == 2) else {
            return false;
        };
        let Some(name) = binding.get("name").and_then(Value::as_str) else {
            return false;
        };
        let Some(value) = binding.get("value").and_then(Value::as_str) else {
            return false;
        };
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            && names.insert(name)
            && !value.is_empty()
            && value.len() <= 64
            && value.is_ascii()
            && value.trim() == value
    })
}

fn validate_surface_analysis_identity(
    candidate: &CandidateClassificationInput,
) -> Result<(), AttackExecutionError> {
    let identity = candidate
        .observation
        .get("target_identity")
        .and_then(Value::as_object)
        .ok_or_else(|| observation_invalid("surface analysis target identity is missing"))?;
    let target_value = identity.get("value").and_then(Value::as_str);
    let target_hash = identity.get("sha256").and_then(Value::as_str);
    let target_id = candidate
        .observation
        .get("target_id")
        .and_then(Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok());
    let target_id_matches = match candidate.target_live_id {
        Some(expected) => target_id == Some(expected),
        None => candidate
            .observation
            .get("target_id")
            .is_some_and(Value::is_null),
    };
    if target_value != Some(candidate.target_value.trim())
        || target_hash != Some(candidate.target_identity_hash.trim())
        || !target_id_matches
    {
        return Err(observation_identity_mismatch(
            "surface analysis target differs from the frozen Candidate identity",
        ));
    }
    if candidate
        .observation
        .get("upstream_query_required")
        .and_then(Value::as_bool)
        != Some(true)
        || !candidate
            .observation
            .get("formulaic_coverage")
            .is_some_and(Value::is_array)
    {
        return Err(observation_invalid(
            "surface analysis observation is missing its bounded context contract",
        ));
    }
    Ok(())
}

fn validate_directory_entry_url(
    frozen_target: &str,
    observed_url: &str,
) -> Result<(), AttackExecutionError> {
    let observed = url::Url::parse(observed_url)
        .map_err(|_| observation_invalid("directory-entry URL is malformed"))?;
    if !matches!(observed.scheme(), "http" | "https")
        || !observed.username().is_empty()
        || observed.password().is_some()
        || observed.query().is_some()
        || observed.fragment().is_some()
        || observed.path().is_empty()
        || observed.path().len() > 2_048
        || observed
            .path()
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n' | '{' | '}' | '<' | '>' | '*'))
    {
        return Err(observation_invalid(
            "directory-entry URL is not an exact safe read-only path",
        ));
    }
    let observed_origin = golish_pentest_domain::canonical_web_origin(observed_url)
        .ok_or_else(|| observation_invalid("directory-entry URL has no canonical origin"))?;
    let target_origin =
        golish_pentest_domain::canonical_web_origin(frozen_target).ok_or_else(|| {
            observation_identity_mismatch(
                "directory-entry Candidate target is not an exact HTTP(S) origin",
            )
        })?;
    if observed_origin.key != target_origin.key {
        return Err(observation_identity_mismatch(
            "directory-entry URL escaped the frozen Candidate origin",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_directory_entry_row_hash(
    directory_entry_id: uuid::Uuid,
    target_id: uuid::Uuid,
    url: &str,
    status_code: i64,
    content_length: i64,
    content_type: &str,
    source_tool: &str,
    declared_hash: &str,
) -> Result<(), AttackExecutionError> {
    let material = canonicalize_json(&json!({
        "content_length": content_length,
        "content_type": content_type,
        "id": directory_entry_id,
        "status_code": status_code,
        "target_id": target_id,
        "tool": source_tool,
        "url": url,
    }));
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| observation_invalid(format!("serialize directory entry: {error}")))?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if declared_hash != format!("sha256:{digest}") {
        return Err(AttackExecutionError::new(
            "ATTACK_OBSERVATION_HASH_MISMATCH",
            "directory-entry observation does not match its frozen row hash",
        ));
    }
    Ok(())
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

pub fn canonical_plan_hash(plan: &CandidateExecutionPlan) -> Result<String, AttackExecutionError> {
    let value = serde_json::to_value(plan).map_err(|error| {
        AttackExecutionError::new(
            "ATTACK_PLAN_CANONICALIZATION_FAILED",
            format!("serialize candidate plan: {error}"),
        )
    })?;
    let bytes = serde_json::to_vec(&canonicalize_json(&value)).map_err(|error| {
        AttackExecutionError::new(
            "ATTACK_PLAN_CANONICALIZATION_FAILED",
            format!("encode canonical candidate plan: {error}"),
        )
    })?;
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{digest}"))
}
