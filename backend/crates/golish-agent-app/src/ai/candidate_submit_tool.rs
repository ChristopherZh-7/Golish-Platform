//! Exact CandidateAttempt submission boundary for the V2 verifier.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};

use golish_agent_kit::db_traits::{
    RuntimeMemoryRepository, RuntimeWorkerFence, SubmitCandidateAttempt,
};
use golish_agent_kit::harness::attack_execution::{
    AttemptDisposition, CandidateAttemptResult, FactDeltaDraft, VerifiedFindingDraft,
};
use golish_core::Tool;

/// Model-visible business fields. Attempt/Candidate/approval/plan and worker
/// identities are deliberately absent and are overwritten from AgentToolContext.
#[derive(Debug, Deserialize)]
struct CandidateAttemptSubmissionArgs {
    disposition: AttemptDisposition,
    #[serde(default)]
    proof_evidence_ids: Vec<i64>,
    #[serde(default)]
    refutation_evidence_ids: Vec<i64>,
    #[serde(default)]
    blocker_evidence_ids: Vec<i64>,
    #[serde(default)]
    blocker_reason_code: Option<String>,
    #[serde(default)]
    finding: Option<VerifiedFindingDraft>,
    #[serde(default)]
    fact_deltas: Vec<FactDeltaDraft>,
}

pub struct SubmitCandidateAttemptTool {
    repository: Arc<dyn RuntimeMemoryRepository>,
}

impl SubmitCandidateAttemptTool {
    pub fn new(repository: Arc<dyn RuntimeMemoryRepository>) -> Self {
        Self { repository }
    }
}

#[async_trait::async_trait]
impl Tool for SubmitCandidateAttemptTool {
    fn name(&self) -> &'static str {
        "submit_candidate_attempt"
    }

    fn description(&self) -> &'static str {
        "Submit the terminal business result for the exact scheduler-bound CandidateAttempt. Identity, approved plan and worker lease are reloaded by the server."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "disposition": { "type": "string", "enum": ["verified", "refuted", "blocked"] },
                "proof_evidence_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 } },
                "refutation_evidence_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 } },
                "blocker_evidence_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 } },
                "blocker_reason_code": { "type": ["string", "null"] },
                "finding": {
                    "type": ["object", "null"],
                    "additionalProperties": false,
                    "properties": {
                        "title": { "type": "string", "minLength": 1 },
                        "severity": { "type": "string", "enum": ["info", "low", "medium", "high", "critical"] },
                        "cvss": { "type": ["number", "null"], "minimum": 0, "maximum": 10 },
                        "affected_target": { "type": "string", "minLength": 1 },
                        "description": { "type": "string", "minLength": 1 },
                        "reproduction_steps": { "type": "array", "minItems": 1, "items": { "type": "string", "minLength": 1 } },
                        "remediation": { "type": "string", "minLength": 1 }
                    },
                    "required": ["title", "severity", "affected_target", "description", "reproduction_steps", "remediation"]
                },
                "fact_deltas": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "fact_kind": { "type": "string", "enum": ["created", "updated", "refuted", "new_surface"] },
                            "canonical_ref_kind": { "type": "string", "minLength": 1 },
                            "canonical_ref_id": { "type": "string", "format": "uuid" },
                            "canonical_ref_version": { "type": "integer", "minimum": 1 },
                            "canonical_ref_hash": { "type": "string", "minLength": 1 },
                            "summary": { "type": "string", "minLength": 1 },
                            "evidence_ids": { "type": "array", "minItems": 1, "items": { "type": "integer", "minimum": 1 } }
                        },
                        "required": ["fact_kind", "canonical_ref_kind", "canonical_ref_id", "canonical_ref_version", "canonical_ref_hash", "summary", "evidence_ids"]
                    }
                }
            },
            "required": ["disposition"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let submitted: CandidateAttemptSubmissionArgs = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(error) => {
                return Ok(json!({
                    "status": "rejected",
                    "code": "ATTACK_ATTEMPT_RESULT_INVALID",
                    "reason": error.to_string(),
                }));
            }
        };
        let Some(context) = golish_core::current_agent_tool_context() else {
            return Ok(json!({
                "status": "rejected",
                "code": "ATTACK_VERIFIER_CONTEXT_REQUIRED",
            }));
        };
        let (
            Some(operation_id),
            Some(stage_execution_id),
            Some(stage_run_unit_id),
            Some(organization_id),
            Some(worker_lease),
            Some(candidate_attempt),
        ) = (
            context.operation_id,
            context.stage_execution_id,
            context.stage_run_unit_id,
            context.organization_id,
            context.worker_lease,
            context.candidate_attempt,
        )
        else {
            return Ok(json!({
                "status": "rejected",
                "code": "ATTACK_VERIFIER_CONTEXT_INCOMPLETE",
            }));
        };
        if worker_lease.stage_run_unit_id != stage_run_unit_id {
            return Ok(json!({
                "status": "rejected",
                "code": "ATTACK_VERIFIER_UNIT_MISMATCH",
            }));
        }
        let result = CandidateAttemptResult {
            attempt_id: candidate_attempt.attempt_id,
            candidate_plan_hash: candidate_attempt.candidate_plan_hash.clone(),
            disposition: submitted.disposition,
            proof_evidence_ids: submitted.proof_evidence_ids,
            refutation_evidence_ids: submitted.refutation_evidence_ids,
            blocker_evidence_ids: submitted.blocker_evidence_ids,
            blocker_reason_code: submitted.blocker_reason_code,
            finding: submitted.finding,
            fact_deltas: submitted.fact_deltas,
        };
        let persisted = self
            .repository
            .submit_candidate_attempt(SubmitCandidateAttempt {
                candidate_attempt,
                fence: RuntimeWorkerFence {
                    operation_id,
                    stage_execution_id,
                    stage_run_unit_id,
                    worker_run_id: worker_lease.worker_run_id,
                    lease_token: worker_lease.lease_token,
                    attempt_epoch: worker_lease.attempt_epoch,
                    // The repository reloads the live checkpoint under the exact
                    // worker lease because AgentToolContext intentionally does
                    // not expose checkpoint state to tool implementations.
                    expected_checkpoint_version: 0,
                },
                organization_id,
                result,
            })
            .await;
        match persisted {
            Ok(persisted) => Ok(json!({
                "status": "submitted",
                "attempt_id": persisted.attempt_id,
                "result_hash": persisted.result_hash,
                "replayed": persisted.replayed,
            })),
            Err(error) => Ok(json!({
                "status": "rejected",
                "code": "ATTACK_ATTEMPT_SUBMISSION_REJECTED",
                "reason": error.to_string(),
            })),
        }
    }
}
