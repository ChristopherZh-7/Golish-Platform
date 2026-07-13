//! Stable pure-domain DTOs for Candidate execution.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::harness::types::FindingSeverity;

pub const CANDIDATE_PLAN_SCHEMA_V1: &str = "candidate-plan-v1";
pub const CANDIDATE_CLASSIFIER_VERSION_V1: &str = "candidate-classifier-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateTargetClass {
    Domain,
    Ip,
    Url,
    Cidr,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationRiskClass {
    DeterministicSafe,
    ActiveSafe,
    Exploit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    ReadOnly,
    ActiveProbe,
    Exploit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptEvidenceRole {
    Proof,
    Refutation,
    Blocker,
    FactDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateBudget {
    pub max_actions: u32,
    pub max_requests: u32,
    pub max_runtime_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateExecutionPlan {
    pub schema_version: String,
    pub classifier_version: String,
    pub candidate_id: Uuid,
    pub target_identity_hash: String,
    pub actions: Vec<PlannedCandidateAction>,
    pub budget: CandidateBudget,
    pub foreground_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedCandidateAction {
    pub ordinal: u32,
    pub capability_id: String,
    pub action_kind: String,
    pub canonical_args: serde_json::Value,
    pub side_effect_class: SideEffectClass,
    pub required_evidence_role: AttemptEvidenceRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptDisposition {
    Verified,
    Refuted,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactDeltaDraft {
    pub fact_kind: String,
    pub canonical_ref_kind: String,
    pub canonical_ref_id: Uuid,
    pub canonical_ref_version: i64,
    pub canonical_ref_hash: String,
    pub summary: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateAttemptResult {
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub disposition: AttemptDisposition,
    pub proof_evidence_ids: Vec<i64>,
    pub refutation_evidence_ids: Vec<i64>,
    pub blocker_evidence_ids: Vec<i64>,
    pub blocker_reason_code: Option<String>,
    pub finding: Option<VerifiedFindingDraft>,
    pub fact_deltas: Vec<FactDeltaDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiedFindingDraft {
    pub title: String,
    pub severity: FindingSeverity,
    pub cvss: Option<f64>,
    pub affected_target: String,
    pub description: String,
    pub reproduction_steps: Vec<String>,
    pub remediation: String,
}

/// Frozen server input to the versioned classifier. Model-supplied tool/risk
/// proposals are intentionally absent; the registry owns those decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateClassificationInput {
    pub candidate_id: Uuid,
    pub target_identity_hash: String,
    pub target_class: CandidateTargetClass,
    pub target_value: String,
    pub hypothesis: String,
    pub technique: String,
    pub prior_refs: Vec<String>,
}

/// One immutable server-seeded reasoning cell. The model sees only its
/// `work_item_key`; all remaining fields are authoritative DB projections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateManifestWorkItem {
    pub work_item_id: Uuid,
    pub work_item_key: String,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateManifestSnapshot {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub manifest_hash: String,
    pub work_items: Vec<CandidateManifestWorkItem>,
}

/// Formulaic scanner output admitted at attack-candidate stage entry. It can
/// create only an observation seed/work-item; Candidate and Finding fields do
/// not exist in this contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormulaicCandidateObservation {
    pub work_item_key: String,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub evidence_ids: Vec<i64>,
}

/// Trusted stage-entry request. Entry identity is the upstream vuln_triage
/// final-sealed handoff, never a model field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SeedCandidateManifest {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub entry_stage_execution_id: Uuid,
    pub entry_stage_run_unit_id: Uuid,
    pub entry_deliverable_submission_id: Uuid,
    pub wave_generation: i32,
    pub wave_ordinal: i32,
    pub policy_snapshot: serde_json::Value,
    pub policy_hash: String,
    pub max_waves: i32,
    pub max_candidates_total: i32,
    pub max_chain_depth: i32,
    pub max_attempts_total: i32,
    pub observations: Vec<FormulaicCandidateObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedCandidateDecision {
    pub candidate_id: Uuid,
    pub work_item_id: Uuid,
    pub hypothesis: String,
    pub technique: Option<String>,
    pub rationale: String,
    pub prior_refs: Vec<String>,
    pub suggested_approach: String,
    pub priority: String,
    pub execution_plan: CandidateExecutionPlan,
    pub candidate_plan_hash: String,
    pub risk_class: VerificationRiskClass,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedNoCandidateDecision {
    pub work_item_id: Uuid,
    pub reason_code: String,
    pub detail: String,
    pub evidence_ids: Vec<i64>,
}

/// Server-derived payload attached to the final PASS transaction. Trusted
/// operation/scope/org/current-submission identities come from `FinalizeUnitPass`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateAcceptance {
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub manifest_hash: String,
    pub expected_work_item_ids: Vec<Uuid>,
    pub candidates: Vec<AcceptedCandidateDecision>,
    pub no_candidate_decisions: Vec<AcceptedNoCandidateDecision>,
}
