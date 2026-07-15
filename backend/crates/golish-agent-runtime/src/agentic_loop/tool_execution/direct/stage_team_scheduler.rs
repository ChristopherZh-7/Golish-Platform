//! Pure construction helpers for the durable Stage Team Scheduler.
//!
//! Runtime execution lives in `stage_run_call`; this module deliberately owns
//! only deterministic TeamPlan/WorkItem material so retries and restarts seed
//! byte-identical rows before any provider dispatch.

use golish_agent_kit::db_traits::{
    NewStageWorkerOutput, RuntimeStageTeamPlanStatus, RuntimeStageWorkItemStatus, SeedStageRuntime,
    SeedStageTeamRuntime, StageTeamPlanSeed, StageTeamPlanView, StageWorkItemSeed,
    StageWorkItemView, StageWorkerOutputDisposition, StageWorkerOutputView,
};
use golish_agent_kit::harness::{CanonicalFactKey, StageSpec};
use golish_sub_agents::StageTeamLeaderBinding;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_TEAM_OUTPUT_VALUES: usize = 128;
const MAX_TEAM_OUTPUT_SUMMARY_CHARS: usize = 4_096;
const MAX_STAGE_TEAM_REPAIR_GENERATIONS: usize = 2;
// Reserve a bounded Controller-repair/child retry budget up front so a valid
// Gate repair cannot be created and then become unclaimable merely because the
// initial Controller/child WorkerRun allowance was exhausted.
const MAX_REPAIR_WORKER_RUNS_PER_GENERATION: usize = 4;

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonicalize_json(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

pub(super) fn sha256_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(&canonicalize_json(value))
        .expect("Stage Team plan material is JSON serializable");
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{digest}")
}

/// Project a claimed WorkItem into the narrow host authority understood by
/// `golish-sub-agents`. Only the exact Company Controller receives it; every
/// dynamic child intentionally receives no binding.
pub(super) fn stage_team_leader_binding_for_claim(
    plan: &StageTeamPlanView,
    item: &StageWorkItemView,
) -> Option<StageTeamLeaderBinding> {
    let is_company_controller = plan
        .dynamic_request_policy
        .get("coordination_mode")
        .and_then(Value::as_str)
        == Some("company_controller");
    (is_company_controller
        && plan.status == RuntimeStageTeamPlanStatus::Active
        && item.status == RuntimeStageWorkItemStatus::Running
        && item.stage_team_plan_id == plan.id
        && item.stage_run_unit_id == plan.stage_run_unit_id
        && item.organization_id == plan.organization_id
        && item.stable_key == "leader:primary"
        && item.role == plan.leader_role
        && plan.aggregator_role.as_deref() == Some(item.role.as_str())
        && item.is_aggregator
        && !item.required_for_barrier
        && item.conflict_key.as_deref() == Some("stage_unit_finalizer"))
    .then_some(StageTeamLeaderBinding {
        stage_team_plan_id: plan.id,
        leader_work_item_id: item.id,
        expected_dispatch_epoch: plan.dispatch_epoch,
        expected_plan_row_version: plan.row_version,
        expected_work_item_row_version: item.row_version,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageChildReport {
    business_disposition: String,
    summary: String,
    #[serde(default)]
    fact_refs: Vec<Value>,
    #[serde(default)]
    evidence_ids: Vec<i64>,
    #[serde(default)]
    checked_empty_units: Vec<Value>,
    #[serde(default)]
    blocker_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StageChildOutputViolation {
    pub failure_code: String,
    pub detail: String,
}

fn stage_child_output_violation(code: &str, detail: &str) -> StageChildOutputViolation {
    StageChildOutputViolation {
        failure_code: code.to_string(),
        detail: detail.to_string(),
    }
}

pub(super) fn strip_matching_legacy_chain_marker(
    response: &str,
    expected_chain_id: Option<uuid::Uuid>,
) -> &str {
    let trimmed = response.trim();
    let Some(expected_chain_id) = expected_chain_id else {
        return trimmed;
    };
    let Some((body, marker)) = trimmed.rsplit_once("\n\n[sub_agent_session_id:") else {
        return trimmed;
    };
    let Some(marker) = marker.strip_suffix(']') else {
        return trimmed;
    };
    match uuid::Uuid::parse_str(marker.trim()) {
        Ok(marker_chain_id) if marker_chain_id == expected_chain_id => body.trim_end(),
        _ => trimmed,
    }
}

fn json_object_from_response(
    response: &str,
    expected_chain_id: Option<uuid::Uuid>,
) -> Option<Value> {
    let trimmed = strip_matching_legacy_chain_marker(response, expected_chain_id);
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value.is_object().then_some(value);
    }
    // Providers occasionally wrap the required object with one short sentence.
    // Accept exactly one fenced payload, but reject ambiguous/multiple fences.
    if trimmed.match_indices("```").count() != 2 {
        return None;
    }
    let fence_start = trimmed.find("```")?;
    let after_start = &trimmed[fence_start + 3..];
    let fence_end = after_start.find("```")?;
    let fenced = after_start[..fence_end].trim();
    let fenced = if let Some(after_language) = fenced.strip_prefix("json") {
        if after_language
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            after_language.trim()
        } else {
            fenced
        }
    } else {
        fenced
    };
    serde_json::from_str::<Value>(fenced)
        .ok()
        .filter(Value::is_object)
}

#[cfg(test)]
fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_TEAM_OUTPUT_SUMMARY_CHARS).collect()
}

fn retain_typed_fact_refs(values: Vec<Value>) -> (Vec<Value>, usize) {
    let original_len = values.len();
    let mut retained = BTreeMap::new();
    for value in values {
        let Ok(key) = serde_json::from_value::<CanonicalFactKey>(value) else {
            continue;
        };
        let Ok(value) = serde_json::to_value(key) else {
            continue;
        };
        let value = canonicalize_json(&value);
        let Ok(identity) = serde_json::to_string(&value) else {
            continue;
        };
        retained.entry(identity).or_insert(value);
    }
    let retained = retained.into_values().collect::<Vec<_>>();
    let discarded = original_len.saturating_sub(retained.len());
    (retained, discarded)
}

#[cfg(test)]
fn fallback_blocked_output(
    item: &StageWorkItemView,
    worker_run_id: uuid::Uuid,
    code: &str,
    detail: &str,
) -> NewStageWorkerOutput {
    let canonical_output = canonicalize_json(&json!({
        "detail": bounded_text(detail),
        "schema_version": 1,
        "stable_work_key": item.stable_key,
        "status": "blocked",
    }));
    let hash_material = json!({
        "blocker_code": code,
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "blocked",
        "evidence_ids": [],
        "fact_refs": [],
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker_run_id,
    });
    NewStageWorkerOutput {
        work_item_id: item.id,
        worker_run_id,
        output_schema: item.output_schema.clone(),
        disposition: StageWorkerOutputDisposition::Blocked,
        canonical_output,
        fact_refs: Vec::new(),
        evidence_ids: Vec::new(),
        checked_empty_units: Vec::new(),
        blocker_code: Some(code.to_string()),
        output_sha256: sha256_json(&hash_material),
    }
}

/// Convert a bounded SubAgent child report into the immutable DB output
/// contract. Protocol/authority violations remain retryable execution failures;
/// only a valid `blocked` report becomes an immutable business blocker.
pub(super) fn stage_child_completion_from_result(
    item: &StageWorkItemView,
    worker_run_id: uuid::Uuid,
    result_value: &Value,
    execution_success: bool,
) -> Result<NewStageWorkerOutput, StageChildOutputViolation> {
    if !execution_success {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_EXECUTION_FAILED",
            result_value
                .get("error")
                .or_else(|| result_value.get("response"))
                .and_then(Value::as_str)
                .unwrap_or("stage child execution failed without a structured result"),
        ));
    }
    let expected_chain_id = result_value
        .get("chain_id")
        .and_then(Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok());
    let Some(report_value) = result_value
        .get("response")
        .and_then(Value::as_str)
        .and_then(|response| json_object_from_response(response, expected_chain_id))
    else {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "stage child did not return the required single JSON object",
        ));
    };
    let Ok(mut report) = serde_json::from_value::<StageChildReport>(report_value) else {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "stage child JSON did not match stage_worker_output.v1",
        ));
    };
    report.evidence_ids.sort_unstable();
    report.evidence_ids.dedup();
    if report.summary.trim().is_empty()
        || report.summary.chars().count() > MAX_TEAM_OUTPUT_SUMMARY_CHARS
        || report.fact_refs.len() > MAX_TEAM_OUTPUT_VALUES
        || report.evidence_ids.len() > MAX_TEAM_OUTPUT_VALUES
        || report.checked_empty_units.len() > MAX_TEAM_OUTPUT_VALUES
        || report.evidence_ids.iter().any(|id| *id <= 0)
    {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "stage child output exceeded bounds or contained invalid evidence ids",
        ));
    }
    let disposition = match report.business_disposition.as_str() {
        "found" => StageWorkerOutputDisposition::Found,
        "checked_empty" => StageWorkerOutputDisposition::CheckedEmpty,
        "blocked"
            if report
                .blocker_code
                .as_deref()
                .is_some_and(|code| !code.trim().is_empty()) =>
        {
            StageWorkerOutputDisposition::Blocked
        }
        _ => {
            return Err(stage_child_output_violation(
                "STAGE_TEAM_WORKER_OUTPUT_INVALID",
                "stage child disposition/blocker contract was invalid",
            ));
        }
    };
    let (fact_refs, discarded_fact_ref_count) = retain_typed_fact_refs(report.fact_refs);
    report.fact_refs = fact_refs;
    if !report.checked_empty_units.is_empty() && report.evidence_ids.is_empty() {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "checked_empty_units require booked evidence",
        ));
    }
    if (disposition == StageWorkerOutputDisposition::Found
        && report.fact_refs.is_empty()
        && report.evidence_ids.is_empty())
        || (disposition == StageWorkerOutputDisposition::CheckedEmpty
            && (report.checked_empty_units.is_empty() || report.evidence_ids.is_empty()))
    {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "stage child disposition had no valid canonical fact or evidence authority",
        ));
    }
    if disposition == StageWorkerOutputDisposition::Blocked
        && report
            .blocker_code
            .as_deref()
            .is_some_and(|code| code.eq_ignore_ascii_case("no_registrable_domain"))
    {
        return Err(stage_child_output_violation(
            report
                .blocker_code
                .as_deref()
                .unwrap_or("no_registrable_domain"),
            "stage child dependency is not ready; retry after sibling discovery",
        ));
    }
    let canonical_output = canonicalize_json(&json!({
        "discarded_invalid_fact_refs": discarded_fact_ref_count,
        "schema_version": 1,
        "stable_work_key": item.stable_key,
        "summary": report.summary,
    }));
    let hash_material = canonicalize_json(&json!({
        "blocker_code": report.blocker_code,
        "canonical_output": canonical_output,
        "checked_empty_units": report.checked_empty_units,
        "disposition": disposition.as_str(),
        "evidence_ids": report.evidence_ids,
        "fact_refs": report.fact_refs,
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker_run_id,
    }));
    Ok(NewStageWorkerOutput {
        work_item_id: item.id,
        worker_run_id,
        output_schema: item.output_schema.clone(),
        disposition,
        canonical_output,
        fact_refs: report.fact_refs,
        evidence_ids: report.evidence_ids,
        checked_empty_units: report.checked_empty_units,
        blocker_code: report.blocker_code,
        output_sha256: sha256_json(&hash_material),
    })
}

#[cfg(test)]
fn stage_child_output_from_result(
    item: &StageWorkItemView,
    worker_run_id: uuid::Uuid,
    result_value: &Value,
    execution_success: bool,
) -> NewStageWorkerOutput {
    stage_child_completion_from_result(item, worker_run_id, result_value, execution_success)
        .unwrap_or_else(|violation| {
            fallback_blocked_output(
                item,
                worker_run_id,
                &violation.failure_code,
                &violation.detail,
            )
        })
}

pub(super) fn stage_child_objective(
    spec: &StageSpec,
    organization_name: &str,
    organization_id: uuid::Uuid,
    item: &StageWorkItemView,
) -> String {
    format!(
        "Run one bounded SubAgent child WorkItem for stage {stage}. Organization: {organization_name} \
         (organization_id: {organization_id}). Durable work_item_id: {work_item_id}; role: {role}; \
         stable key: {stable_key}; frozen input: {input}. Work ONLY on this bounded assignment/subject and \
         use only tools allowed by the current stage. Do not call submit_stage_deliverable and do \
         not spawn another agent. Finish with exactly one JSON object and no prose using this \
         schema: {{\"business_disposition\":\"found|checked_empty|blocked\",\"summary\":\"...\",\
         \"fact_refs\":[],\"evidence_ids\":[],\"checked_empty_units\":[],\"blocker_code\":null}}. \
         Non-empty fact_refs \
         must likewise be exact CanonicalFactKey JSON objects returned by tools; never invent string refs, \
         and leave fact_refs empty when no typed key was returned. \
         A blocked disposition requires a stable blocker_code. A found result may retain independently \
         checked-empty provider/asset subunits in checked_empty_units; those subunits do not downgrade \
         the overall found result. Any non-empty checked_empty_units requires booked evidence. Evidence \
         ids must come from evidence actually booked by this WorkItem. A checked_empty business disposition \
         MUST include at least one exact checked_empty_units entry and booked evidence; never return \
         checked_empty with an empty checked_empty_units array.",
        stage = spec.kind.as_str(),
        work_item_id = item.id,
        role = item.role,
        stable_key = item.stable_key,
        input = item.input_refs,
    )
}

pub(super) fn controller_final_objective(
    spec: &StageSpec,
    organization_name: &str,
    organization_id: uuid::Uuid,
    outputs: &[StageWorkerOutputView],
) -> Result<String, &'static str> {
    let manifest = outputs
        .iter()
        .map(|output| {
            json!({
                "business_disposition": output.disposition.as_str(),
                "canonical_output": output.canonical_output,
                "evidence_ids": output.evidence_ids,
                "output_sha256": output.output_sha256,
                "work_item_id": output.work_item_id,
            })
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_string(&manifest).map_err(|_| "team_output_not_serializable")?;
    Ok(format!(
        "Continue as the same Company Controller for stage {stage}. Organization: \
         {organization_name} (organization_id: {organization_id}). This is your final submission \
         turn and the request epoch is closed; do not dispatch more SubAgents. Reconcile the \
         immutable child-output manifest below with CURRENT database/evidence-ledger truth. Child \
         prose is not gate authority. Close any exact remaining deterministic gaps using \
         stage-allowed tools, then call submit_stage_deliverable exactly once. This Company \
         Controller is the only Worker allowed to submit the final Unit deliverable.\n\n\
         IMMUTABLE CHILD OUTPUT MANIFEST:\n{encoded}",
        stage = spec.kind.as_str(),
    ))
}

pub(super) fn build_stage_team_seed(
    spec: &StageSpec,
    base: SeedStageRuntime,
) -> Result<Option<SeedStageTeamRuntime>, &'static str> {
    let Some(policy) = spec.team_scheduler.as_ref() else {
        return Ok(None);
    };
    if !policy.enabled_in_v2_only {
        return Ok(None);
    }
    if policy.schema_version != 1
        || policy.aggregator_kind.trim().is_empty()
        || policy.aggregator_role.trim().is_empty()
        || policy.max_company_units_active == 0
        || policy.global_provider_cap == 0
        || policy.max_workers == 0
        || policy.allowed_roles.is_empty()
        || policy.max_dynamic_requests == 0
        || policy.max_dynamic_subject_refs == 0
        || policy.allowed_dynamic_request_kinds.is_empty()
        || !policy
            .allowed_roles
            .iter()
            .any(|role| role == &policy.aggregator_role)
    {
        return Err("invalid_stage_team_policy");
    }

    let leader_manifest = canonicalize_json(&json!({
        "coordination_mode": "company_controller",
        "role": policy.aggregator_role,
        "stage": spec.kind.as_str(),
    }));
    let work_items = vec![StageWorkItemSeed {
        stable_key: "leader:primary".to_string(),
        work_item_kind: policy.aggregator_kind.clone(),
        role: policy.aggregator_role.clone(),
        input_sha256: sha256_json(&leader_manifest),
        input_manifest: leader_manifest,
        conflict_key: Some("stage_unit_finalizer".to_string()),
        priority: 0,
        required_for_barrier: false,
        is_aggregator: true,
        attempt_policy: json!({"max_attempts": 3}),
        budget: json!({}),
        output_schema: "stage_unit_aggregate.v1".to_string(),
        created_by: "server_seed".to_string(),
    }];

    let mut allowed_roles = policy.allowed_roles.clone();
    allowed_roles.sort();
    allowed_roles.dedup();
    let plan_material = json!({
        "aggregator_kind": policy.aggregator_kind,
        "aggregator_role": policy.aggregator_role,
        "allowed_roles": allowed_roles,
        "child_budget": {},
        "child_output_schema": "stage_worker_output.v1",
        "coordination_mode": "company_controller",
        "global_provider_cap": policy.global_provider_cap,
        "allowed_dynamic_request_kinds": policy.allowed_dynamic_request_kinds,
        "max_company_units_active": policy.max_company_units_active,
        "max_dynamic_subject_refs": policy.max_dynamic_subject_refs,
        "max_dynamic_requests": policy.max_dynamic_requests,
        "max_workers": policy.max_workers,
        "organization_scope_implicit": true,
        "risk_lane": policy.risk_lane,
        "schema_version": policy.schema_version,
        "stage": spec.kind.as_str(),
        "work_items": work_items
            .iter()
            .map(|item| json!({
                "input_sha256": item.input_sha256,
                "is_aggregator": item.is_aggregator,
                "required_for_barrier": item.required_for_barrier,
                "role": item.role,
                "stable_key": item.stable_key,
                "work_item_kind": item.work_item_kind,
            }))
            .collect::<Vec<_>>(),
    });
    let plan_material = canonicalize_json(&plan_material);
    let plan_sha256 = sha256_json(&plan_material);
    let created_from_stage_spec_hash = sha256_json(
        &serde_json::to_value(policy).map_err(|_| "stage_team_policy_not_serializable")?,
    );

    // `max_workers` is the concurrency cap from StageSpec, not the lifetime
    // number of WorkerRuns. The Lead and every dynamic child may consume up to
    // three retry attempts, so the lifetime budget is derived independently.
    let maximum_dynamic_requests = usize::try_from(policy.max_dynamic_requests)
        .map_err(|_| "stage_team_worker_limit_overflow")?;
    let maximum_work_items = work_items
        .len()
        .checked_add(maximum_dynamic_requests)
        .ok_or("stage_team_worker_limit_overflow")?;
    let initial_worker_runs = maximum_work_items
        .checked_mul(3)
        .ok_or("stage_team_worker_limit_overflow")?;
    let repair_worker_runs = MAX_STAGE_TEAM_REPAIR_GENERATIONS
        .checked_mul(MAX_REPAIR_WORKER_RUNS_PER_GENERATION)
        .ok_or("stage_team_worker_limit_overflow")?;
    let maximum_worker_runs = initial_worker_runs
        .checked_add(repair_worker_runs)
        .ok_or("stage_team_worker_limit_overflow")?;

    Ok(Some(SeedStageTeamRuntime {
        base,
        plan: StageTeamPlanSeed {
            schema_version: 1,
            plan_version: 1,
            plan_sha256,
            leader_role: policy.aggregator_role.clone(),
            allowed_roles,
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some(policy.aggregator_role.clone()),
            max_workers_total: i32::try_from(maximum_worker_runs)
                .map_err(|_| "stage_team_worker_limit_overflow")?,
            max_workers_active: i32::try_from(policy.max_workers)
                .map_err(|_| "stage_team_active_worker_limit_overflow")?,
            dynamic_requests_enabled: true,
            dynamic_request_policy: json!({
                "allowed_request_kinds": policy.allowed_dynamic_request_kinds,
                "canonical_subject_refs_only": true,
                "child_budget": {},
                "child_output_schema": "stage_worker_output.v1",
                "coordination_mode": "company_controller",
                "global_provider_cap": policy.global_provider_cap,
                "max_company_units_active": policy.max_company_units_active,
                // The current runtime-memory schema binds one durable Gate gap
                // to the stable Company Controller WorkerRun. Freeze that
                // explicit limit instead of inheriting the two-generation
                // sibling-Aggregator repair budget.
                "max_controller_gate_repairs": 1,
                "max_requests": policy.max_dynamic_requests,
                "max_repair_generations": MAX_STAGE_TEAM_REPAIR_GENERATIONS,
                "max_subject_refs": policy.max_dynamic_subject_refs,
                "organization_scope_implicit": true,
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash,
        },
        work_items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::harness::{load_embedded_stage_spec, StageKind};
    use uuid::Uuid;

    fn base_seed() -> SeedStageRuntime {
        SeedStageRuntime {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_kind: StageKind::TargetIntel.as_str().to_string(),
            unit_generation: 1,
            specialist: "recon".to_string(),
            worker_generation: 1,
            work_item_kind: "organization".to_string(),
            work_item_key: StageKind::TargetIntel.as_str().to_string(),
            agent_path_prefix: "main>stage_run:target_intel".to_string(),
            organization_ids: None,
        }
    }

    fn company_controller_spec(
        max_company_units_active: u32,
        global_provider_cap: u32,
    ) -> StageSpec {
        let spec = load_embedded_stage_spec(StageKind::TargetIntel).expect("target_intel spec");
        let mut value = serde_json::to_value(spec).expect("serializable stage spec");
        value["team_scheduler"]["max_company_units_active"] = json!(max_company_units_active);
        value["team_scheduler"]["global_provider_cap"] = json!(global_provider_cap);
        serde_json::from_value::<StageSpec>(value).expect("controller stage spec")
    }

    fn controller_claim_views() -> (StageTeamPlanView, StageWorkItemView) {
        let seeded = build_stage_team_seed(&company_controller_spec(2, 8), base_seed())
            .expect("valid controller policy")
            .expect("team enabled");
        let plan_id = Uuid::new_v4();
        let unit_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let plan = StageTeamPlanView {
            id: plan_id,
            operation_id: seeded.base.operation_id,
            stage_execution_id: seeded.base.stage_execution_id,
            stage_run_unit_id: unit_id,
            scope_snapshot_id: Uuid::new_v4(),
            organization_id,
            stage_kind: seeded.base.stage_kind,
            unit_generation: seeded.base.unit_generation,
            schema_version: seeded.plan.schema_version,
            plan_version: seeded.plan.plan_version,
            plan_sha256: seeded.plan.plan_sha256,
            leader_role: seeded.plan.leader_role,
            allowed_roles: seeded.plan.allowed_roles,
            aggregator_kind: seeded.plan.aggregator_kind,
            aggregator_role: seeded.plan.aggregator_role,
            max_workers_total: seeded.plan.max_workers_total,
            max_workers_active: seeded.plan.max_workers_active,
            dynamic_requests_enabled: seeded.plan.dynamic_requests_enabled,
            dynamic_request_policy: seeded.plan.dynamic_request_policy,
            dispatch_epoch: 11,
            requests_closed_at: None,
            final_submitter_kind: seeded.plan.final_submitter_kind,
            final_submitter_worker_run_id: None,
            created_from_stage_spec_hash: seeded.plan.created_from_stage_spec_hash,
            status: RuntimeStageTeamPlanStatus::Active,
            row_version: 13,
        };
        let seeded_item = seeded.work_items.into_iter().next().expect("leader seed");
        let item = StageWorkItemView {
            id: Uuid::new_v4(),
            stage_team_plan_id: plan.id,
            stage_run_unit_id: plan.stage_run_unit_id,
            organization_id: plan.organization_id,
            stable_key: seeded_item.stable_key,
            work_item_kind: seeded_item.work_item_kind,
            role: seeded_item.role,
            input_refs: seeded_item.input_manifest,
            input_manifest_hash: seeded_item.input_sha256,
            priority: seeded_item.priority,
            required_for_barrier: seeded_item.required_for_barrier,
            is_aggregator: seeded_item.is_aggregator,
            conflict_key: seeded_item.conflict_key,
            attempt_policy: seeded_item.attempt_policy,
            budget: seeded_item.budget,
            output_schema: seeded_item.output_schema,
            created_by: seeded_item.created_by,
            status: RuntimeStageWorkItemStatus::Running,
            row_version: 17,
        };
        (plan, item)
    }

    #[test]
    fn trusted_leader_binding_is_exact_and_leader_only() {
        let (plan, mut item) = controller_claim_views();
        let binding = stage_team_leader_binding_for_claim(&plan, &item)
            .expect("exact company controller leader gets authority");
        assert_eq!(binding.stage_team_plan_id, plan.id);
        assert_eq!(binding.leader_work_item_id, item.id);
        assert_eq!(binding.expected_dispatch_epoch, 11);
        assert_eq!(binding.expected_plan_row_version, 13);
        assert_eq!(binding.expected_work_item_row_version, 17);

        item.stable_key = "dynamic:child".to_string();
        assert!(stage_team_leader_binding_for_claim(&plan, &item).is_none());

        let (mut untrusted_plan, untrusted_leader) = controller_claim_views();
        untrusted_plan.dynamic_request_policy = json!({});
        assert!(stage_team_leader_binding_for_claim(&untrusted_plan, &untrusted_leader).is_none());
    }

    #[test]
    fn target_intel_plan_is_stable_and_seeds_only_the_primary_leader() {
        let spec = load_embedded_stage_spec(StageKind::TargetIntel).expect("target_intel spec");
        let first = build_stage_team_seed(&spec, base_seed())
            .expect("valid policy")
            .expect("team enabled");
        let second = build_stage_team_seed(&spec, first.base.clone())
            .expect("valid policy")
            .expect("team enabled");

        assert_eq!(first.plan.plan_sha256, second.plan.plan_sha256);
        assert_eq!(first.work_items.len(), 1);
        assert_eq!(first.work_items[0].stable_key, "leader:primary");
        assert!(first.plan.max_workers_total >= first.work_items.len() as i32);
        let initial_and_dynamic_items = first.work_items.len()
            + usize::try_from(
                spec.team_scheduler
                    .as_ref()
                    .expect("team scheduler")
                    .max_dynamic_requests,
            )
            .expect("dynamic request limit");
        let reserved_repair_runs =
            MAX_STAGE_TEAM_REPAIR_GENERATIONS * MAX_REPAIR_WORKER_RUNS_PER_GENERATION;
        assert_eq!(
            usize::try_from(first.plan.max_workers_total).expect("worker budget"),
            initial_and_dynamic_items * 3 + reserved_repair_runs
        );
        assert_eq!(
            first.plan.max_workers_active,
            spec.team_scheduler.as_ref().unwrap().max_workers as i32
        );
    }

    #[test]
    fn downstream_company_stages_seed_the_same_controller_shape() {
        for (stage, specialist, child_role) in [
            (StageKind::ExternalAttackSurface, "prober", "prober"),
            (StageKind::Enumeration, "enumerator", "enumerator"),
            (StageKind::VulnTriage, "vuln_scanner", "vuln_scanner"),
        ] {
            let spec = load_embedded_stage_spec(stage).expect("downstream stage spec");
            let mut base = base_seed();
            base.stage_kind = stage.as_str().to_string();
            base.specialist = specialist.to_string();
            base.work_item_key = stage.as_str().to_string();
            base.agent_path_prefix = format!("main>stage_run:{}", stage.as_str());

            let seeded = build_stage_team_seed(&spec, base)
                .expect("valid downstream controller policy")
                .unwrap_or_else(|| panic!("{} must seed a Team", stage.as_str()));

            assert_eq!(seeded.work_items.len(), 1);
            assert_eq!(seeded.work_items[0].stable_key, "leader:primary");
            assert_eq!(seeded.work_items[0].role, "company_stage_controller");
            assert!(seeded.plan.allowed_roles.contains(&child_role.to_string()));
        }
    }

    #[test]
    fn company_controller_plan_seeds_only_the_primary_leader() {
        let spec = company_controller_spec(2, 8);

        let seeded = build_stage_team_seed(&spec, base_seed())
            .expect("valid controller policy")
            .expect("team enabled");

        assert_eq!(seeded.work_items.len(), 1);
        let leader = &seeded.work_items[0];
        assert_eq!(leader.stable_key, "leader:primary");
        assert_eq!(leader.role, seeded.plan.leader_role);
        assert_eq!(
            Some(leader.role.as_str()),
            seeded.plan.aggregator_role.as_deref()
        );
        assert!(leader.is_aggregator);
        assert!(!leader.required_for_barrier);
        assert_eq!(leader.conflict_key.as_deref(), Some("stage_unit_finalizer"));
    }

    #[test]
    fn company_controller_requires_two_layer_concurrency_caps() {
        for field in [
            "max_company_units_active",
            "global_provider_cap",
            "max_workers",
        ] {
            let mut spec = company_controller_spec(2, 8);
            let policy = spec.team_scheduler.as_mut().expect("team policy");
            match field {
                "max_company_units_active" => policy.max_company_units_active = 0,
                "global_provider_cap" => policy.global_provider_cap = 0,
                "max_workers" => policy.max_workers = 0,
                _ => unreachable!(),
            }
            let error = build_stage_team_seed(&spec, base_seed())
                .expect_err("company-controller C, G, and K must all be non-zero");

            assert_eq!(error, "invalid_stage_team_policy");
        }
    }

    #[test]
    fn company_controller_freezes_two_layer_concurrency_and_child_contract() {
        let seeded = build_stage_team_seed(&company_controller_spec(3, 7), base_seed())
            .expect("valid controller policy")
            .expect("team enabled");

        assert_eq!(seeded.plan.max_workers_active, 4, "K includes the Lead");
        assert_eq!(
            seeded.plan.dynamic_request_policy,
            json!({
                "allowed_request_kinds": ["coverage_recheck", "provider_followup"],
                "canonical_subject_refs_only": true,
                "child_budget": {},
                "child_output_schema": "stage_worker_output.v1",
                "coordination_mode": "company_controller",
                "global_provider_cap": 7,
                "max_company_units_active": 3,
                "max_controller_gate_repairs": 1,
                "max_repair_generations": MAX_STAGE_TEAM_REPAIR_GENERATIONS,
                "max_requests": 12,
                "max_subject_refs": 16,
                "organization_scope_implicit": true,
            })
        );
    }

    #[test]
    fn verification_has_no_general_stage_team_plan() {
        let spec = load_embedded_stage_spec(StageKind::Verification).expect("verification spec");
        let mut base = base_seed();
        base.stage_kind = StageKind::Verification.as_str().to_string();
        assert!(build_stage_team_seed(&spec, base)
            .expect("valid absence")
            .is_none());
    }

    fn stage_child_item() -> StageWorkItemView {
        StageWorkItemView {
            id: Uuid::new_v4(),
            stage_team_plan_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            stable_key: "dynamic:child-dns".to_string(),
            work_item_kind: "stage_axis".to_string(),
            role: "intel_provider".to_string(),
            input_refs: json!([{"axis":"DNS"}]),
            input_manifest_hash: format!("sha256:{}", "1".repeat(64)),
            priority: 0,
            required_for_barrier: true,
            is_aggregator: false,
            conflict_key: None,
            attempt_policy: json!({"max_attempts": 3}),
            budget: json!({}),
            output_schema: "stage_worker_output.v1".to_string(),
            created_by: "accepted_worker_request".to_string(),
            status: RuntimeStageWorkItemStatus::Running,
            row_version: 1,
        }
    }

    #[test]
    fn stage_child_result_requires_the_bounded_business_output_contract() {
        let item = stage_child_item();
        let worker_run_id = Uuid::new_v4();
        let valid = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": r#"{"business_disposition":"found","summary":"DNS evidence booked","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":null}"#}),
            true,
        );
        assert_eq!(valid.disposition, StageWorkerOutputDisposition::Found);
        assert_eq!(valid.evidence_ids, vec![41]);
        assert!(valid.output_sha256.starts_with("sha256:"));

        let invalid = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": "I think everything worked"}),
            true,
        );
        assert_eq!(invalid.disposition, StageWorkerOutputDisposition::Blocked);
        assert_eq!(
            invalid.blocker_code.as_deref(),
            Some("STAGE_TEAM_WORKER_OUTPUT_INVALID")
        );
    }

    #[test]
    fn stage_child_completion_accepts_one_fenced_object_with_outer_prose() {
        let item = stage_child_item();
        let worker_run_id = Uuid::new_v4();
        let chain_id = Uuid::new_v4();
        let response = format!(
            "WHOIS child completed.\n\n```json\n{}\n```\n\n[sub_agent_session_id: {chain_id}]",
            r#"{"business_disposition":"found","summary":"WHOIS evidence booked","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":null}"#
        );

        let output = stage_child_completion_from_result(
            &item,
            worker_run_id,
            &json!({"response": response, "chain_id": chain_id.to_string()}),
            true,
        )
        .expect("one bounded fenced object should be accepted");

        assert_eq!(output.disposition, StageWorkerOutputDisposition::Found);
        assert_eq!(output.evidence_ids, vec![41]);
    }

    #[test]
    fn invalid_checked_empty_is_a_retryable_protocol_failure() {
        let item = stage_child_item();
        let violation = stage_child_completion_from_result(
            &item,
            Uuid::new_v4(),
            &json!({
                "response": r#"{"business_disposition":"checked_empty","summary":"No ASN data","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":null}"#
            }),
            true,
        )
        .expect_err("checked_empty without an exact checked unit must retry");

        assert_eq!(violation.failure_code, "STAGE_TEAM_WORKER_OUTPUT_INVALID");
        assert_eq!(
            violation.detail,
            "stage child disposition had no valid canonical fact or evidence authority"
        );
    }

    #[test]
    fn dependency_not_ready_blocker_retries_but_unknown_business_blocker_is_terminal() {
        let item = stage_child_item();
        let dependency = stage_child_completion_from_result(
            &item,
            Uuid::new_v4(),
            &json!({
                "response": r#"{"business_disposition":"blocked","summary":"No registrable domain exists yet","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":"no_registrable_domain"}"#
            }),
            true,
        )
        .expect_err("a registered dependency blocker should consume a bounded retry");
        assert_eq!(dependency.failure_code, "no_registrable_domain");

        let terminal = stage_child_completion_from_result(
            &item,
            Uuid::new_v4(),
            &json!({
                "response": r#"{"business_disposition":"blocked","summary":"Provider credentials unavailable","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":"provider_credentials_unavailable"}"#
            }),
            true,
        )
        .expect("unknown business blockers remain immutable terminal outputs");
        assert_eq!(terminal.disposition, StageWorkerOutputDisposition::Blocked);
        assert_eq!(
            terminal.blocker_code.as_deref(),
            Some("provider_credentials_unavailable")
        );
    }

    #[test]
    fn stage_child_result_canonicalizes_and_dedupes_typed_fact_refs() {
        let item = stage_child_item();
        let worker_run_id = Uuid::new_v4();
        let target_id = Uuid::from_u128(41);
        let response = json!({
            "business_disposition": "found",
            "summary": "CT evidence booked",
            "fact_refs": [
                "ct|example.test|serial-1",
                {"target_id": target_id, "kind": "target"},
                {"kind": "target", "target_id": target_id}
            ],
            "evidence_ids": [41],
            "checked_empty_units": [],
            "blocker_code": null
        })
        .to_string();
        let output = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": response}),
            true,
        );

        assert_eq!(output.disposition, StageWorkerOutputDisposition::Found);
        assert_eq!(
            output.fact_refs,
            vec![json!({"kind": "target", "target_id": target_id})]
        );
        assert_eq!(output.canonical_output["discarded_invalid_fact_refs"], 2);

        let only_invalid = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({
                "response": r#"{"business_disposition":"found","summary":"invented refs","fact_refs":["ct|example.test|serial-1"],"evidence_ids":[],"checked_empty_units":[],"blocker_code":null}"#
            }),
            true,
        );
        assert_eq!(
            only_invalid.disposition,
            StageWorkerOutputDisposition::Blocked
        );
        assert_eq!(
            only_invalid.blocker_code.as_deref(),
            Some("STAGE_TEAM_WORKER_OUTPUT_INVALID")
        );
    }

    #[test]
    fn stage_child_result_accepts_only_the_matching_runtime_chain_marker() {
        let item = stage_child_item();
        let worker_run_id = Uuid::new_v4();
        let chain_id = Uuid::new_v4();
        let response = format!(
            "{}\n\n[sub_agent_session_id: {chain_id}]",
            r#"{"business_disposition":"found","summary":"DNS evidence booked","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":null}"#
        );
        let valid = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": response, "chain_id": chain_id.to_string()}),
            true,
        );
        assert_eq!(valid.disposition, StageWorkerOutputDisposition::Found);
        assert_eq!(valid.evidence_ids, vec![41]);

        let mismatched = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": response, "chain_id": Uuid::new_v4().to_string()}),
            true,
        );
        assert_eq!(
            mismatched.blocker_code.as_deref(),
            Some("STAGE_TEAM_WORKER_OUTPUT_INVALID")
        );

        let missing_chain = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({"response": response}),
            true,
        );
        assert_eq!(
            missing_chain.blocker_code.as_deref(),
            Some("STAGE_TEAM_WORKER_OUTPUT_INVALID")
        );
    }

    #[test]
    fn stage_child_result_preserves_evidenced_empty_subunits_in_a_found_result() {
        let item = stage_child_item();
        let worker_run_id = Uuid::new_v4();
        let mixed = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({
                "response": r#"{"business_disposition":"found","summary":"ENScan found the root while 0.zone checked the child set empty","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[{"provider":"0.zone","asset":"example.test"}],"blocker_code":null}"#
            }),
            true,
        );

        assert_eq!(mixed.disposition, StageWorkerOutputDisposition::Found);
        assert_eq!(mixed.evidence_ids, vec![41]);
        assert_eq!(mixed.checked_empty_units.len(), 1);

        let unevidenced = stage_child_output_from_result(
            &item,
            worker_run_id,
            &json!({
                "response": r#"{"business_disposition":"found","summary":"unattested empty subunit","fact_refs":[{"kind":"target"}],"evidence_ids":[],"checked_empty_units":[{"provider":"0.zone","asset":"example.test"}],"blocker_code":null}"#
            }),
            true,
        );
        assert_eq!(
            unevidenced.blocker_code.as_deref(),
            Some("STAGE_TEAM_WORKER_OUTPUT_INVALID")
        );
    }

    #[test]
    fn controller_final_turn_uses_immutable_child_outputs_and_closes_dispatch() {
        let spec = load_embedded_stage_spec(StageKind::TargetIntel).unwrap();
        let item = stage_child_item();
        let output = stage_child_output_from_result(
            &item,
            Uuid::new_v4(),
            &json!({"response": r#"{"business_disposition":"checked_empty","summary":"No records","fact_refs":[],"evidence_ids":[],"checked_empty_units":[{"axis":"DNS"}],"blocker_code":null}"#}),
            true,
        );
        let view = StageWorkerOutputView {
            id: Uuid::new_v4(),
            stage_team_plan_id: item.stage_team_plan_id,
            work_item_id: item.id,
            worker_run_id: output.worker_run_id,
            disposition: output.disposition,
            canonical_output: output.canonical_output,
            fact_refs: output.fact_refs,
            evidence_ids: output.evidence_ids,
            checked_empty_units: output.checked_empty_units,
            blocker_code: output.blocker_code,
            output_sha256: output.output_sha256,
            created_at: chrono::Utc::now(),
        };
        let prompt =
            controller_final_objective(&spec, "Example Corp", item.organization_id, &[view])
                .unwrap();
        assert!(prompt.contains("same Company Controller"));
        assert!(prompt.contains("final submission turn"));
        assert!(prompt.contains("request epoch is closed"));
        assert!(prompt.contains("IMMUTABLE CHILD OUTPUT MANIFEST"));
        assert!(prompt.contains(&item.id.to_string()));
        assert!(prompt.contains("only Worker allowed"));
    }
}
