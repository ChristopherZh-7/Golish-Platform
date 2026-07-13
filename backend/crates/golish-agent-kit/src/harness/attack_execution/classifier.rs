//! Versioned immutable Candidate classifier registry and canonical plan hashing.

use std::collections::BTreeMap;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::state::AttackExecutionError;
use super::types::{
    AttemptEvidenceRole, CandidateBudget, CandidateClassificationInput, CandidateExecutionPlan,
    CandidateTargetClass, PlannedCandidateAction, SideEffectClass, VerificationRiskClass,
    CANDIDATE_CLASSIFIER_VERSION_V1, CANDIDATE_PLAN_SCHEMA_V1,
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
        technique: "WSTG-ATHZ-04",
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

pub fn classify_candidate(
    candidate: &CandidateClassificationInput,
) -> Result<CandidateExecutionPlan, AttackExecutionError> {
    if candidate.candidate_id.is_nil()
        || candidate.target_identity_hash.trim().is_empty()
        || candidate.target_value.trim().is_empty()
        || candidate.hypothesis.trim().is_empty()
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

    let mut prior_refs = candidate
        .prior_refs
        .iter()
        .map(|reference| reference.trim())
        .filter(|reference| !reference.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    prior_refs.sort();
    prior_refs.dedup();

    let canonical_args = canonicalize_json(&json!({
        "background": false,
        "hypothesis": candidate.hypothesis.trim(),
        "prior_refs": prior_refs,
        "target": candidate.target_value.trim(),
        "technique": recipe.technique,
    }));
    Ok(CandidateExecutionPlan {
        schema_version: CANDIDATE_PLAN_SCHEMA_V1.to_string(),
        classifier_version: CANDIDATE_CLASSIFIER_VERSION_V1.to_string(),
        candidate_id: candidate.candidate_id,
        target_identity_hash: candidate.target_identity_hash.trim().to_string(),
        actions: vec![PlannedCandidateAction {
            ordinal: 0,
            capability_id: recipe.capability_id.to_string(),
            action_kind: recipe.action_kind.to_string(),
            canonical_args,
            side_effect_class: recipe.side_effect_class,
            required_evidence_role: recipe.required_evidence_role,
        }],
        budget: recipe.budget,
        foreground_only: true,
    })
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
