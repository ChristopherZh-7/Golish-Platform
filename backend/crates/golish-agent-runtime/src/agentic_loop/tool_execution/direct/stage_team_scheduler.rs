//! Pure construction helpers for the durable Stage Team Scheduler.
//!
//! Runtime execution lives in `stage_run_call`; this module deliberately owns
//! only deterministic TeamPlan/WorkItem material so retries and restarts seed
//! byte-identical rows before any provider dispatch.

use golish_agent_kit::db_traits::{
    CandidateAnalysisArtifactOutputReceipt, EnumerationLaneClosureReceiptV2, EnumerationLaneKindV2,
    NewStageWorkerOutput, RuntimeStageTeamPlanStatus, RuntimeStageWorkItemStatus, SeedStageRuntime,
    SeedStageTeamRuntime, StageTeamPlanSeed, StageTeamPlanView, StageWorkItemSeed,
    StageWorkItemView, StageWorkerOutputDisposition, StageWorkerOutputView,
};
use golish_agent_kit::harness::{CanonicalFactKey, StageKind, StageSpec};
use golish_sub_agents::{
    InvestigationAssetLaneIdentity, StageTeamCompiledActionBinding, StageTeamLeaderBinding,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const MAX_TEAM_OUTPUT_VALUES: usize = 128;
const MAX_TEAM_OUTPUT_SUMMARY_CHARS: usize = 4_096;
const MAX_STAGE_TEAM_REPAIR_GENERATIONS: usize = 2;
const MAX_STAGE_TEAM_CONTROLLER_TURN_RESUMES: usize = 2;
// Reserve a bounded Controller-repair/child retry budget up front so a valid
// Gate repair cannot be created and then become unclaimable merely because the
// initial Controller/child WorkerRun allowance was exhausted.
const MAX_REPAIR_WORKER_RUNS_PER_GENERATION: usize = 4;
const COMPANY_CONTROLLER_COORDINATION_MODE: &str = "company_controller";
const INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE: &str = "investigation_task_orchestrator";
const INVESTIGATION_CONFIGURED_COGNITIVE_ROLES: [&str; 9] = [
    "investigation",
    "pentester",
    "researcher",
    "browser",
    "adviser",
    "coder",
    "installer",
    "enricher",
    "memorist",
];
const INVESTIGATION_DYNAMIC_COGNITIVE_ROLES: [&str; 8] = [
    "pentester",
    "researcher",
    "browser",
    "coder",
    "installer",
    "enricher",
    "memorist",
    "adviser",
];

pub(super) const ENUMERATION_MAX_COMPANY_UNITS_ACTIVE: u32 = 2;
pub(super) const ENUMERATION_MAX_WORKERS_PER_COMPANY: u32 = 3;
pub(super) const ENUMERATION_GLOBAL_HOST_JOB_CAP: u32 = 6;
pub(super) const ENUMERATION_GLOBAL_BROWSER_JOB_CAP: u32 = 2;
pub(super) const ENUMERATION_GLOBAL_PROVIDER_CAP: u32 = 4;
pub(super) const ENUMERATION_MAX_DYNAMIC_REQUESTS_PER_COMPANY: u32 = 256;
/// A formulaic WorkItem has two frozen Worker attempts.  A large but finite
/// route-probe queue may legitimately need more than one WorkItem while each
/// invocation remains bounded.  Successors preserve the exact origin and the
/// tool-owned durable checkpoint. Six generations cap one producer at twelve
/// wrapper invocations without rewriting exhausted history. The final bounded
/// successor exists for response-loss recovery after a producer outcome lands
/// but its v2 authority receipt transaction fails; terminal WorkItems are never
/// rearmed because their deterministic output ids are immutable.
pub(super) const ENUMERATION_MAX_FORMULAIC_GENERATIONS: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EnumerationProducerKind {
    Preflight,
    Content,
    Browser,
    JsApi,
    Parameter,
    Resolution,
    Coverage,
}

impl EnumerationProducerKind {
    pub(super) const fn role(self) -> &'static str {
        match self {
            Self::Preflight | Self::Content => "content_mapper",
            Self::Browser => "browser_runtime",
            Self::JsApi => "js_api_analyzer",
            Self::Parameter => "parameter_analyzer",
            Self::Resolution => "resolution_analyst",
            Self::Coverage => "coverage_reviewer",
        }
    }

    pub(super) const fn wave(self) -> u8 {
        match self {
            Self::Preflight => 0,
            Self::Content | Self::Browser => 1,
            Self::JsApi => 2,
            Self::Parameter => 3,
            Self::Resolution => 4,
            Self::Coverage => 5,
        }
    }

    pub(super) const fn execution_kind(self) -> &'static str {
        match self {
            Self::Resolution => "llm_subagent",
            _ => "host_deterministic",
        }
    }

    pub(super) const fn request_kind(self) -> &'static str {
        match self {
            Self::Resolution => "enumeration_resolution",
            _ => "formulaic_enumeration",
        }
    }

    pub(super) const fn formulaic_tool(self) -> Option<&'static str> {
        match self {
            Self::Preflight => Some("enum_preflight_web_origins"),
            Self::Content => Some("route_probe_paths"),
            Self::Browser => Some("browser_collect_js_api"),
            Self::JsApi => Some("js_extract_apis"),
            Self::Parameter | Self::Coverage => None,
            Self::Resolution => Some("enum_js_get_resolution_cluster"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EnumerationWorklistShard {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub scope_snapshot_id: uuid::Uuid,
    pub target_id: uuid::Uuid,
    pub exact_origin: String,
    pub producer: EnumerationProducerKind,
    pub unresolved_cluster_id: Option<String>,
    pub generation: uuid::Uuid,
    pub attempt: u32,
    /// Exact immutable dependency receipts carried forward from named sibling
    /// WorkerOutputs. The vector is lane ordered by receipt id and is never
    /// reconstructed from a latest/current authority lookup.
    pub dependency_lane_receipts_v2: Vec<EnumerationLaneClosureReceiptV2>,
    pub producer_evidence_audit_ids: Vec<i64>,
}

impl EnumerationWorklistShard {
    fn canonical_dependency_receipts(&self) -> Vec<EnumerationLaneClosureReceiptV2> {
        self.dependency_lane_receipts_v2
            .iter()
            .cloned()
            .map(|mut receipt| {
                receipt.replayed = false;
                receipt
            })
            .collect()
    }

    pub(super) fn stable_key(&self) -> String {
        let dependency_lane_receipts_v2 = self.canonical_dependency_receipts();
        let digest = sha256_json(&json!({
            "exact_origin": self.exact_origin,
            "operation_id": self.operation_id,
            "organization_id": self.organization_id,
            "producer": self.producer,
            "dependency_lane_receipts_v2": dependency_lane_receipts_v2,
            "producer_evidence_audit_ids": self.producer_evidence_audit_ids,
            "scope_snapshot_id": self.scope_snapshot_id,
            "stage_execution_id": self.stage_execution_id,
            "stage_run_unit_id": self.stage_run_unit_id,
            "target_id": self.target_id,
            "unresolved_cluster_id": self.unresolved_cluster_id,
        }));
        let base = format!(
            "enumeration:{}:{}",
            self.producer.role(),
            digest.trim_start_matches("sha256:")
        );
        if self.attempt <= 1 {
            base
        } else {
            format!("{base}:successor:{}", self.attempt)
        }
    }

    pub(super) fn successor(&self) -> Self {
        let mut successor = self.clone();
        successor.attempt = successor.attempt.saturating_add(1);
        successor
    }

    pub(super) fn typed_objective(&self) -> Value {
        let dependency_lane_receipts_v2 = self.canonical_dependency_receipts();
        json!({
            "assignment_schema": "enumeration_formulaic_shard.v2",
            "attempt": self.attempt,
            "exact_origin": self.exact_origin,
            "execution_kind": self.producer.execution_kind(),
            "generation": self.generation,
            "operation_id": self.operation_id,
            "organization_id": self.organization_id,
            "producer": self.producer,
            "dependency_lane_receipts_v2": dependency_lane_receipts_v2,
            "producer_evidence_audit_ids": self.producer_evidence_audit_ids,
            "role": self.producer.role(),
            "scope_snapshot_id": self.scope_snapshot_id,
            "stable_work_key": self.stable_key(),
            "stage_execution_id": self.stage_execution_id,
            "stage_run_unit_id": self.stage_run_unit_id,
            "target_id": self.target_id,
            "unresolved_cluster_id": self.unresolved_cluster_id,
            "wave": self.producer.wave(),
        })
    }

    pub(super) fn subject_refs(&self) -> Vec<Value> {
        vec![json!({
            "kind": "target",
            "target_id": self.target_id,
        })]
    }

    pub(super) fn formulaic_args(&self) -> Option<Value> {
        match self.producer {
            EnumerationProducerKind::Preflight => Some(json!({
                "origins": [{
                    "target_id": self.target_id,
                    "target_url": self.exact_origin,
                }]
            })),
            EnumerationProducerKind::Content => Some(json!({
                "targets": [{
                    "target_id": self.target_id,
                    "base_url": self.exact_origin,
                }],
                "batch_concurrency": 1,
            })),
            EnumerationProducerKind::Browser => Some(json!({
                "target_id": self.target_id,
                "target_url": self.exact_origin,
                "crawl_mode": "standard",
                "ai": false,
                "ai_assist": false,
            })),
            EnumerationProducerKind::JsApi => Some(json!({
                "target_id": self.target_id,
                "target_url": self.exact_origin,
                "ai": false,
            })),
            EnumerationProducerKind::Parameter
            | EnumerationProducerKind::Resolution
            | EnumerationProducerKind::Coverage => None,
        }
    }

    pub(super) fn objective(&self) -> String {
        let mut objective = self.typed_objective();
        objective["instructions"] = if self.producer == EnumerationProducerKind::Resolution {
            json!(
                "Analyze only the assigned unresolved_cluster_id. First call enum_js_get_resolution_cluster with that exact UUID, reason over only its bounded redacted source windows and capture anchors, then call enum_js_submit_resolution exactly once with an evidence-anchored advisory disposition. Do not broaden scope, fetch arbitrary source, invent URL/parameter facts, publish a canonical endpoint, dispatch another worker, or submit a stage deliverable. The host closes the immutable residual receipt after your advisory tool result."
            )
        } else {
            json!(
                "Execute exactly this host-authored shard once. Do not broaden the origin, change the target id, enable nested AI flags, page another worklist, dispatch another worker, or submit a stage deliverable. Producer terminality is validated from the wrapper's persisted exact-origin outcome."
            )
        };
        objective["tool"] = self
            .producer
            .formulaic_tool()
            .map_or(Value::Null, Value::from);
        objective["tool_args"] = self.formulaic_args().unwrap_or(Value::Null);
        objective.to_string()
    }

    pub(super) fn controller_action(&self) -> StageTeamCompiledActionBinding {
        StageTeamCompiledActionBinding {
            action_id: self.stable_key(),
            dedupe_key: self.stable_key(),
            requested_role: self.producer.role().to_string(),
            requested_kind: self.producer.request_kind().to_string(),
            objective: self.objective(),
            subject_refs: self.subject_refs(),
            budget_hint: json!({"max_wrapper_calls": 1}),
        }
    }
}

#[cfg(test)]
pub(super) fn enumeration_shards_for_origin(
    authority: &EnumerationWorklistShard,
    has_unresolved_cluster: bool,
) -> Vec<EnumerationWorklistShard> {
    let producers = [
        EnumerationProducerKind::Preflight,
        EnumerationProducerKind::Content,
        EnumerationProducerKind::Browser,
        EnumerationProducerKind::JsApi,
        EnumerationProducerKind::Parameter,
        EnumerationProducerKind::Resolution,
        EnumerationProducerKind::Coverage,
    ];
    producers
        .into_iter()
        .filter(|producer| {
            *producer != EnumerationProducerKind::Resolution || has_unresolved_cluster
        })
        .map(|producer| EnumerationWorklistShard {
            producer,
            unresolved_cluster_id: (producer == EnumerationProducerKind::Resolution)
                .then(|| authority.unresolved_cluster_id.clone())
                .flatten(),
            dependency_lane_receipts_v2: Vec::new(),
            producer_evidence_audit_ids: Vec::new(),
            ..authority.clone()
        })
        .collect()
}

pub(super) fn enumeration_wave_dependencies_satisfied(
    producer: EnumerationProducerKind,
    terminal_producers: &BTreeSet<EnumerationProducerKind>,
    unresolved_cluster_exists: bool,
) -> bool {
    match producer {
        EnumerationProducerKind::Preflight => true,
        EnumerationProducerKind::Content | EnumerationProducerKind::Browser => {
            terminal_producers.contains(&EnumerationProducerKind::Preflight)
        }
        EnumerationProducerKind::JsApi => {
            terminal_producers.contains(&EnumerationProducerKind::Browser)
        }
        EnumerationProducerKind::Parameter => {
            terminal_producers.contains(&EnumerationProducerKind::Browser)
                && terminal_producers.contains(&EnumerationProducerKind::JsApi)
        }
        EnumerationProducerKind::Resolution => {
            unresolved_cluster_exists
                && terminal_producers.contains(&EnumerationProducerKind::JsApi)
        }
        EnumerationProducerKind::Coverage => {
            terminal_producers.contains(&EnumerationProducerKind::Content)
                && terminal_producers.contains(&EnumerationProducerKind::Parameter)
                && (!unresolved_cluster_exists
                    || terminal_producers.contains(&EnumerationProducerKind::Resolution))
        }
    }
}

/// Derive the complete deterministic producer denominator for one exact Web
/// Origin. Terminal coverage rows do not erase a producer from the current
/// frozen contract: replay is suppressed by the stable shard key/output, not
/// by trusting a coverage projection as execution authority.
pub(super) fn enumeration_required_producers_for_techniques<'a>(
    techniques: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<EnumerationProducerKind>, String> {
    let mut required = BTreeSet::new();
    for technique in techniques {
        // Every frozen exact Web Origin runs the complete Browser -> JsApi ->
        // Parameter receipt chain. Technique cells select additional probes;
        // they do not make the JS/API truth prerequisite optional.
        required.insert(EnumerationProducerKind::Preflight);
        required.insert(EnumerationProducerKind::Browser);
        required.insert(EnumerationProducerKind::JsApi);
        required.insert(EnumerationProducerKind::Parameter);
        required.insert(EnumerationProducerKind::Coverage);
        match technique {
            "GOLISH-ENUM-DIR" => {
                required.insert(EnumerationProducerKind::Content);
            }
            "GOLISH-ENUM-JS" => {
                required.insert(EnumerationProducerKind::Browser);
            }
            "GOLISH-ENUM-JSAPI" => {
                required.insert(EnumerationProducerKind::Browser);
                required.insert(EnumerationProducerKind::JsApi);
            }
            "GOLISH-ENUM-PARAM" => {
                required.insert(EnumerationProducerKind::Browser);
                required.insert(EnumerationProducerKind::JsApi);
                required.insert(EnumerationProducerKind::Parameter);
            }
            unsupported => {
                return Err(format!("unsupported Enumeration technique '{unsupported}'"))
            }
        }
    }
    Ok(required)
}

#[allow(dead_code)] // consumed by the Task 9 Candidate stage_run wiring
pub(super) fn candidate_artifact_receipt_output(
    work_item_id: uuid::Uuid,
    worker_run_id: uuid::Uuid,
    artifact_id: uuid::Uuid,
    artifact_hash: &str,
) -> Result<NewStageWorkerOutput, &'static str> {
    let receipt = CandidateAnalysisArtifactOutputReceipt::new(artifact_id, artifact_hash.into())
        .map_err(|_| "candidate_artifact_receipt_invalid")?;
    let canonical_output = canonicalize_json(&receipt.canonical_output());
    Ok(NewStageWorkerOutput {
        work_item_id,
        worker_run_id,
        output_schema: "candidate_analysis_artifact_receipt.v1".into(),
        disposition: StageWorkerOutputDisposition::ArtifactRecorded,
        canonical_output: canonical_output.clone(),
        fact_refs: Vec::new(),
        evidence_ids: Vec::new(),
        checked_empty_units: Vec::new(),
        blocker_code: None,
        output_sha256: sha256_json(&canonical_output),
    })
}

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

const VULN_GENERAL_BASELINE_TECHNIQUES: &[&str] = &[
    "WSTG-ATHN-02",
    "WSTG-SESS-02",
    "WSTG-CONF-05",
    "WSTG-CRYP-03",
    "WSTG-INFO",
];
const VULN_GENERAL_DAST_TECHNIQUES: &[&str] = &["WSTG-INPV-05", "WSTG-INPV-01", "WSTG-INPV-12"];
const VULN_ANONYMOUS_TECHNIQUE: &str = "WSTG-ATHN-04";
const VULN_NDAY_TECHNIQUE: &str = "GOLISH-NDAY";
pub(super) const MAX_VULN_AUTOMATIC_ATTEMPTS: u32 = 3;
pub(super) const MAX_VULN_ANONYMOUS_AUTOMATIC_ATTEMPTS: u32 = 2;
const VULN_BUDGET_RECOVERY_ATTEMPT: u32 = MAX_VULN_AUTOMATIC_ATTEMPTS + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum VulnShardShape {
    Primary,
    Narrowed,
    BudgetRecovery,
}

impl VulnShardShape {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Narrowed => "narrowed",
            Self::BudgetRecovery => "budget_recovery",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VulnWorklistShard {
    pub target_id: uuid::Uuid,
    pub target_url: String,
    pub tool_name: &'static str,
    pub techniques: Vec<String>,
    pub shape: VulnShardShape,
    pub recovery_attempt: u32,
}

impl VulnWorklistShard {
    fn capability_family(&self) -> &'static str {
        match self.tool_name {
            "vuln_nuclei_general"
                if self.techniques.iter().all(|technique| {
                    VULN_GENERAL_DAST_TECHNIQUES.contains(&technique.as_str())
                }) =>
            {
                "nuclei_general_dast"
            }
            "vuln_nuclei_general" => "nuclei_general_baseline",
            "vuln_nuclei_fingerprint_targeted" => "nuclei_fingerprint_targeted",
            "vuln_probe_anonymous_access" => "anonymous_access",
            _ => "invalid",
        }
    }

    pub(super) fn stable_key(&self) -> String {
        let digest = sha256_json(&json!({
            "capability": self.capability_family(),
            "recovery_attempt": self.recovery_attempt,
            "shape": self.shape.as_str(),
            "target_id": self.target_id,
            "target_url": self.target_url,
            "techniques": self.techniques,
            "tool": self.tool_name,
        }));
        format!(
            "vuln-worklist:{}:{}",
            self.shape.as_str(),
            digest.trim_start_matches("sha256:")
        )
    }

    pub(super) fn subject_refs(&self) -> Vec<Value> {
        vec![json!({
            "kind": "target",
            "target_id": self.target_id,
        })]
    }

    pub(super) fn objective(&self) -> String {
        json!({
            "assignment_schema": "vuln_formulaic_shard.v1",
            "capability": self.capability_family(),
            "instructions": "Execute exactly this server-owned shard. Do not page the worklist, broaden the subject, dispatch workers, or retry the wrapper inside this Worker. Call the named wrapper at most once, preserve partial/error truth, then return the typed Stage Worker result.",
            "recovery_attempt": self.recovery_attempt,
            "shape": self.shape.as_str(),
            "target_id": self.target_id,
            "target_url": self.target_url,
            "techniques": self.techniques,
            "tool": self.tool_name,
        })
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VulnShardGroupKey {
    target_id: uuid::Uuid,
    target_url: String,
    tool_name: &'static str,
    capability_family: &'static str,
    shape: VulnShardShape,
    recovery_attempt: u32,
    narrowed_technique: Option<String>,
}

fn exact_http_origin(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/")
        && url.port_or_known_default().is_some()
}

fn vuln_capability_for_technique(technique: &str) -> Option<(&'static str, &'static str)> {
    if VULN_GENERAL_DAST_TECHNIQUES.contains(&technique) {
        Some(("vuln_nuclei_general", "nuclei_general_dast"))
    } else if VULN_GENERAL_BASELINE_TECHNIQUES.contains(&technique) {
        Some(("vuln_nuclei_general", "nuclei_general_baseline"))
    } else if technique == VULN_NDAY_TECHNIQUE {
        Some((
            "vuln_nuclei_fingerprint_targeted",
            "nuclei_fingerprint_targeted",
        ))
    } else if technique == VULN_ANONYMOUS_TECHNIQUE {
        Some(("vuln_probe_anonymous_access", "anonymous_access"))
    } else {
        None
    }
}

pub(super) fn validate_vuln_shard_assignment(
    tool_name: &str,
    capability_family: &str,
    target_url: &str,
    techniques: &[String],
) -> bool {
    !techniques.is_empty()
        && exact_http_origin(target_url)
        && techniques.iter().all(|technique| {
            vuln_capability_for_technique(technique).is_some_and(
                |(expected_tool, expected_family)| {
                    expected_tool == tool_name && expected_family == capability_family
                },
            )
        })
}

/// Convert the operation-scoped Vuln coverage matrix into deterministic exact
/// origin shards. Pending cells share one capability call; any prior
/// partial/error is narrowed to one technique so a spent broad budget is never
/// retried with the same shape.
pub(super) fn build_vuln_worklist_shards(
    snapshot: &Value,
) -> Result<Vec<VulnWorklistShard>, &'static str> {
    if snapshot.get("stage").and_then(Value::as_str) != Some("vuln_triage") {
        return Err("vuln_worklist_stage_mismatch");
    }
    let assets = snapshot
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("vuln_worklist_assets_missing")?;
    let mut grouped = BTreeMap::<VulnShardGroupKey, BTreeSet<String>>::new();
    for asset in assets {
        let target_id = asset
            .get("target_id")
            .and_then(Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .filter(|value| !value.is_nil())
            .ok_or("vuln_worklist_target_id_invalid")?;
        let target_url = asset
            .get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && exact_http_origin(value))
            .ok_or("vuln_worklist_exact_origin_invalid")?
            .to_string();
        let coverage = asset
            .get("coverage")
            .and_then(Value::as_array)
            .ok_or("vuln_worklist_coverage_missing")?;
        for cell in coverage {
            let technique = cell
                .get("technique")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or("vuln_worklist_technique_missing")?;
            let (tool_name, capability_family) = vuln_capability_for_technique(technique)
                .ok_or("vuln_worklist_technique_unknown")?;
            let state = cell
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let (shape, recovery_attempt) = match state {
                "pending" => (VulnShardShape::Primary, 1),
                "partial" | "error" => {
                    let prior_attempt = cell
                        .pointer("/details/attempt_ordinal")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .filter(|value| *value > 0)
                        .unwrap_or(1);
                    let retry_disabled = cell
                        .pointer("/details/automatic_retry_allowed")
                        .and_then(Value::as_bool)
                        == Some(false);
                    let legacy_budget_recovery = retry_disabled
                        && prior_attempt == MAX_VULN_AUTOMATIC_ATTEMPTS
                        && matches!(
                            tool_name,
                            "vuln_nuclei_general" | "vuln_nuclei_fingerprint_targeted"
                        )
                        && cell
                            .pointer("/details/failure_owner")
                            .and_then(Value::as_str)
                            == Some("scanner_runtime")
                        && cell
                            .pointer("/details/failure_class")
                            .and_then(Value::as_str)
                            == Some("scan_budget_exhausted");
                    if legacy_budget_recovery {
                        (VulnShardShape::BudgetRecovery, VULN_BUDGET_RECOVERY_ATTEMPT)
                    } else {
                        if retry_disabled || prior_attempt >= MAX_VULN_AUTOMATIC_ATTEMPTS {
                            continue;
                        }
                        (VulnShardShape::Narrowed, prior_attempt.saturating_add(1))
                    }
                }
                "found" | "checked_empty" | "blocked" | "not_applicable" => continue,
                _ => return Err("vuln_worklist_state_invalid"),
            };
            let key = VulnShardGroupKey {
                target_id,
                target_url: target_url.clone(),
                tool_name,
                capability_family,
                shape,
                recovery_attempt,
                narrowed_technique: (shape != VulnShardShape::Primary)
                    .then(|| technique.to_string()),
            };
            grouped
                .entry(key)
                .or_default()
                .insert(technique.to_string());
        }
    }
    Ok(grouped
        .into_iter()
        .map(|(key, techniques)| VulnWorklistShard {
            target_id: key.target_id,
            target_url: key.target_url,
            tool_name: key.tool_name,
            techniques: techniques.into_iter().collect(),
            shape: key.shape,
            recovery_attempt: key.recovery_attempt,
        })
        .collect())
}

pub(super) fn exact_investigation_asset_primary(item: &StageWorkItemView) -> bool {
    let Some([marker]) = item.input_refs.as_array().map(Vec::as_slice) else {
        return false;
    };
    let Some(marker) = marker
        .as_object()
        .filter(|marker| matches!(marker.len(), 5 | 6))
    else {
        return false;
    };
    let Some(asset_lane_id) = marker
        .get("asset_lane_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return false;
    };
    let Some(target_id) = marker
        .get("target_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return false;
    };
    let Some(asset_context_sha256) = marker.get("asset_context_sha256").and_then(Value::as_str)
    else {
        return false;
    };
    let Some(evolution_epoch) = marker.get("evolution_epoch").and_then(Value::as_i64) else {
        return false;
    };
    let schedule_round = match marker.get("schedule_round") {
        Some(value) => match value.as_i64().filter(|value| *value >= 0) {
            Some(value) if marker.len() == 6 => Some(value),
            _ => return false,
        },
        None if marker.len() == 5 => None,
        None => return false,
    };
    if (InvestigationAssetLaneIdentity {
        asset_lane_id,
        target_id,
        asset_context_sha256: asset_context_sha256.to_string(),
    })
    .validate()
    .is_err()
    {
        return false;
    }
    let expected_id = Uuid::new_v5(
        &asset_lane_id,
        match schedule_round {
            Some(round) => {
                format!("investigation-asset-primary-work-item-v2:{evolution_epoch}:{round}")
            }
            None => format!("investigation-asset-primary-work-item-v1:{evolution_epoch}"),
        }
        .as_bytes(),
    );
    let expected_key = match schedule_round {
        Some(round) => format!("asset:{asset_lane_id}:primary:{evolution_epoch}:round:{round}"),
        None => format!("asset:{asset_lane_id}:primary:{evolution_epoch}"),
    };
    marker.get("kind").and_then(Value::as_str) == Some("investigation_asset_lane")
        && item.id == expected_id
        && item.stable_key == expected_key
        && item.work_item_kind == "investigation_asset_primary"
        && item.created_by == "server_phase_transition"
        && item.output_schema == "stage_unit_aggregate.v1"
        && item.conflict_key.is_none()
        && !item.required_for_barrier
        && item.input_manifest_hash == asset_context_sha256
}

pub(super) fn stage_team_leader_binding_for_claim(
    plan: &StageTeamPlanView,
    item: &StageWorkItemView,
) -> Option<StageTeamLeaderBinding> {
    let coordination_mode = plan
        .dynamic_request_policy
        .get("coordination_mode")
        .and_then(Value::as_str);
    let planning_only = match coordination_mode {
        Some(COMPANY_CONTROLLER_COORDINATION_MODE) => false,
        Some(INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE)
            if plan.stage_kind == StageKind::Investigation.as_str()
                && plan.leader_role == "investigation" =>
        {
            true
        }
        _ => return None,
    };
    let exact_primary_key = item.stable_key == "leader:primary";
    let exact_asset_primary_key = planning_only && exact_investigation_asset_primary(item);
    let exact_synthesis_recovery_key = planning_only
        && item
            .stable_key
            .strip_prefix("leader:synthesis-recovery:")
            .and_then(|value| Uuid::parse_str(value).ok())
            .is_some();
    let exact_finalizer_conflict = (exact_primary_key
        && item.conflict_key.as_deref() == Some("stage_unit_finalizer"))
        || ((exact_synthesis_recovery_key || exact_asset_primary_key)
            && item.conflict_key.is_none());
    let completed_investigation_primary_replay = planning_only
        && (exact_primary_key || exact_asset_primary_key)
        && plan.requests_closed_at.is_some()
        && item.status == RuntimeStageWorkItemStatus::Completed;
    let parked_investigation_primary_continuation = planning_only
        && (exact_primary_key || exact_asset_primary_key)
        && plan.requests_closed_at.is_none()
        && item.status == RuntimeStageWorkItemStatus::WaitingDependency;
    (((plan.status == RuntimeStageTeamPlanStatus::Active)
        || (completed_investigation_primary_replay
            && plan.status == RuntimeStageTeamPlanStatus::Finalizing))
        && (item.status == RuntimeStageWorkItemStatus::Running
            || parked_investigation_primary_continuation
            || completed_investigation_primary_replay)
        && item.stage_team_plan_id == plan.id
        && item.stage_run_unit_id == plan.stage_run_unit_id
        && item.organization_id == plan.organization_id
        && (exact_primary_key || exact_synthesis_recovery_key || exact_asset_primary_key)
        && item.role == plan.leader_role
        && plan.aggregator_role.as_deref() == Some(item.role.as_str())
        && item.is_aggregator
        && !item.required_for_barrier
        && exact_finalizer_conflict)
        .then_some(StageTeamLeaderBinding {
            stage_team_plan_id: plan.id,
            leader_work_item_id: item.id,
            expected_dispatch_epoch: plan.dispatch_epoch,
            expected_plan_row_version: plan.row_version,
            expected_work_item_row_version: item.row_version,
            controller_action_compiler: plan
                .dynamic_request_policy
                .get("controller_action_compiler")
                .and_then(Value::as_str)
                .map(str::to_string),
            compiled_actions: Vec::new(),
            planning_only,
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
    #[serde(default)]
    proposal_signals: Vec<Value>,
    #[serde(default)]
    action_intents: Vec<Value>,
    #[serde(default)]
    residuals: Vec<Value>,
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
    let cognitive_payload_bytes = serde_json::to_vec(&json!({
        "action_intents": &report.action_intents,
        "proposal_signals": &report.proposal_signals,
        "residuals": &report.residuals,
    }))
    .map_or(usize::MAX, |value| value.len());
    report.evidence_ids.sort_unstable();
    report.evidence_ids.dedup();
    if report.summary.trim().is_empty()
        || report.summary.chars().count() > MAX_TEAM_OUTPUT_SUMMARY_CHARS
        || report.fact_refs.len() > MAX_TEAM_OUTPUT_VALUES
        || report.evidence_ids.len() > MAX_TEAM_OUTPUT_VALUES
        || report.checked_empty_units.len() > MAX_TEAM_OUTPUT_VALUES
        || report.proposal_signals.len() > 32
        || report.action_intents.len() > 32
        || report.residuals.len() > 32
        || cognitive_payload_bytes > 64 * 1024
        || report
            .proposal_signals
            .iter()
            .chain(report.action_intents.iter())
            .chain(report.residuals.iter())
            .any(|value| !value.is_object())
        || report.evidence_ids.iter().any(|id| *id <= 0)
    {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "stage child output exceeded bounds or contained invalid evidence ids",
        ));
    }
    let investigation_cognitive_output = item.output_schema == "investigation_cognitive_output.v1";
    if investigation_cognitive_output
        && report.business_disposition == "found"
        && (!report.fact_refs.is_empty()
            || !report.evidence_ids.is_empty()
            || !report.checked_empty_units.is_empty()
            || report.blocker_code.is_some())
    {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "successful Investigation cognition must be advisory-only: use found with empty fact_refs, evidence_ids, checked_empty_units and null blocker_code; inherited subject refs stay in frozen context, while proposal_signals/action_intents/residuals carry advisory output",
        ));
    }
    if !investigation_cognitive_output
        && (!report.proposal_signals.is_empty()
            || !report.action_intents.is_empty()
            || !report.residuals.is_empty())
    {
        return Err(stage_child_output_violation(
            "STAGE_TEAM_WORKER_OUTPUT_INVALID",
            "non-Investigation worker output cannot carry cognitive proposal fields",
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
        && report.evidence_ids.is_empty()
        && !investigation_cognitive_output)
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
    let mut canonical_output = canonicalize_json(&json!({
        "discarded_invalid_fact_refs": discarded_fact_ref_count,
        "schema_version": 1,
        "stable_work_key": item.stable_key,
        "summary": report.summary,
    }));
    if investigation_cognitive_output {
        canonical_output["proposal_signals"] = Value::Array(report.proposal_signals);
        canonical_output["action_intents"] = Value::Array(report.action_intents);
        canonical_output["residuals"] = Value::Array(report.residuals);
        canonical_output = canonicalize_json(&canonical_output);
    }
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

/// Turn the exact wrapper's already-landed ledger result into the immutable
/// advisory child output. The wrapper/technique_outcome rows remain Gate
/// authority; this record only proves that the durable shard reached its one
/// allowed producer boundary.
pub(super) fn server_vuln_child_output_from_wrapper(
    item: &StageWorkItemView,
    worker_run_id: uuid::Uuid,
    wrapper_result: &Value,
) -> Result<NewStageWorkerOutput, StageChildOutputViolation> {
    let mut evidence_ids = wrapper_result
        .pointer("/guarded_evidence/evidence_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    if evidence_ids.is_empty() {
        return Err(stage_child_output_violation(
            "VULN_FORMULAIC_LEDGER_LANDING_MISSING",
            "server-owned Vuln wrapper returned without authoritative evidence ids",
        ));
    }
    let canonical_output = canonicalize_json(&json!({
        "automatic_retry_allowed": wrapper_result.get("automatic_retry_allowed"),
        "completion_state": wrapper_result.get("completion_state"),
        "exact_origin": wrapper_result.get("exact_origin"),
        "schema_version": 1,
        "stable_work_key": item.stable_key,
        "techniques": wrapper_result.get("techniques"),
        "wrapper_tool": wrapper_result.get("wrapper_tool"),
    }));
    let hash_material = canonicalize_json(&json!({
        "blocker_code": Value::Null,
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "found",
        "evidence_ids": evidence_ids,
        "fact_refs": [],
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker_run_id,
    }));
    Ok(NewStageWorkerOutput {
        work_item_id: item.id,
        worker_run_id,
        output_schema: item.output_schema.clone(),
        disposition: StageWorkerOutputDisposition::Found,
        canonical_output,
        fact_refs: Vec::new(),
        evidence_ids,
        checked_empty_units: Vec::new(),
        blocker_code: None,
        output_sha256: sha256_json(&hash_material),
    })
}

/// Persist the trusted Enumeration producer/verifier receipt in the immutable
/// WorkerOutput. Later waves obtain the exact authority/hash tuple from this
/// named sibling output, never from a latest/current repository query.
pub(super) fn server_enumeration_receipt_output(
    item: &StageWorkItemView,
    worker_run_id: uuid::Uuid,
    producer: EnumerationProducerKind,
    exact_origin: &str,
    receipt: &EnumerationLaneClosureReceiptV2,
    evidence_ids: &[i64],
) -> Result<NewStageWorkerOutput, StageChildOutputViolation> {
    let expected_lane = match producer {
        EnumerationProducerKind::Browser => Some(EnumerationLaneKindV2::Browser),
        EnumerationProducerKind::JsApi => Some(EnumerationLaneKindV2::JsApi),
        EnumerationProducerKind::Parameter => Some(EnumerationLaneKindV2::Parameter),
        EnumerationProducerKind::Resolution => Some(EnumerationLaneKindV2::Resolution),
        EnumerationProducerKind::Coverage => Some(EnumerationLaneKindV2::Coverage),
        EnumerationProducerKind::Preflight | EnumerationProducerKind::Content => None,
    };
    if expected_lane != Some(receipt.lane) || !receipt.is_terminal() {
        return Err(stage_child_output_violation(
            "ENUMERATION_PRODUCER_RECEIPT_INVALID",
            "server-owned Enumeration worker returned a vacuous or mismatched producer receipt",
        ));
    }
    let mut submitted_evidence_ids = evidence_ids
        .iter()
        .copied()
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    submitted_evidence_ids.sort_unstable();
    submitted_evidence_ids.dedup();
    let evidence_ids = receipt.evidence_audit_ids.clone();
    if evidence_ids.is_empty()
        || evidence_ids.iter().any(|id| *id <= 0)
        || evidence_ids.windows(2).any(|pair| pair[0] >= pair[1])
        || submitted_evidence_ids != evidence_ids
    {
        return Err(stage_child_output_violation(
            "ENUMERATION_FORMULAIC_LEDGER_LANDING_DRIFT",
            "Enumeration WorkerOutput evidence must equal the receipt's canonical exact manifest",
        ));
    }
    let canonical_output = canonicalize_json(&json!({
        "exact_origin": exact_origin,
        "producer": producer,
        "lane_closure_receipt_v2": receipt,
        "schema_version": 2,
        "stable_work_key": item.stable_key,
    }));
    let hash_material = canonicalize_json(&json!({
        "blocker_code": Value::Null,
        "canonical_output": canonical_output,
        "checked_empty_units": [],
        "disposition": "found",
        "evidence_ids": evidence_ids,
        "fact_refs": [],
        "output_schema": item.output_schema,
        "work_item_id": item.id,
        "worker_run_id": worker_run_id,
    }));
    Ok(NewStageWorkerOutput {
        work_item_id: item.id,
        worker_run_id,
        output_schema: item.output_schema.clone(),
        disposition: StageWorkerOutputDisposition::Found,
        canonical_output,
        fact_refs: Vec::new(),
        evidence_ids,
        checked_empty_units: Vec::new(),
        blocker_code: None,
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
    if spec.kind == StageKind::Investigation {
        return format!(
            "Run one bounded cognition-only Investigation WorkItem. Organization: {organization_name} \
             (organization_id: {organization_id}). Durable work_item_id: {work_item_id}; role: {role}; \
             stable key: {stable_key}; frozen input: {input}. Analyze only this assignment. You may \
             use exact-scope read tools and, when genuinely needed, one bounded nested cognitive \
             specialist; you may not perform HTTP/browser/CLI/credential/pentest I/O, write a Finding, \
             mutate a canonical hypothesis, change scope, or submit a stage deliverable. Finish with \
             exactly one JSON object and no prose using investigation_cognitive_output.v1: \
             {{\"business_disposition\":\"found|checked_empty|blocked\",\"summary\":\"...\",\
             \"fact_refs\":[],\"evidence_ids\":[],\"checked_empty_units\":[],\"blocker_code\":null,\
             \"proposal_signals\":[],\"action_intents\":[],\"residuals\":[]}}. Proposal/action/residual \
             entries are advisory typed objects for the host reducer, never proof or execution authority. \
             A successful cognition result MUST use business_disposition=found with fact_refs=[], \
             evidence_ids=[], checked_empty_units=[], and blocker_code=null. Put knowledge gaps or \
             checked-empty advisory observations in residuals instead. Inherited evidence subject_refs \
             in frozen input are sealed authority selectors for the supplied context; they are not expected \
             to appear in list_recent_evidence, whose exact-worker scope contains only rows newly produced \
             by this WorkItem. Never classify inherited evidence as missing merely because that view is \
             empty, and never copy inherited ids into evidence_ids. blocked is reserved for a real \
             provider/runtime blocker and requires a stable blocker_code.",
            work_item_id = item.id,
            role = item.role,
            stable_key = item.stable_key,
            input = item.input_refs,
        );
    }
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
         ids must come from evidence actually booked by this WorkItem. Before returning, refresh with \
         list_recent_evidence and copy only values from its evidence_id field; never copy a generic id or \
         any audit/action, event, or tool-call id into evidence_ids. A checked_empty business disposition \
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
    let final_action = if spec.kind == StageKind::VulnTriage {
        "The server-owned formulaic worklist executor has observed zero unfinished cells. Do not run any scanner, wrapper, worklist paging, retry, or child dispatch in this turn. Refresh current DB/evidence truth only as needed, then call submit_stage_deliverable exactly once."
    } else {
        "Close any exact remaining deterministic gaps using stage-allowed tools, then call submit_stage_deliverable exactly once."
    };
    Ok(format!(
        "Continue as the same Company Controller for stage {stage}. Organization: \
         {organization_name} (organization_id: {organization_id}). This is your final submission \
         turn and the request epoch is closed; do not dispatch more SubAgents. Reconcile the \
         immutable child-output manifest below with CURRENT database/evidence-ledger truth. Child \
         prose is not gate authority. {final_action} This Company \
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

    let server_owned_vuln_worklist = spec.kind == StageKind::VulnTriage;
    let server_owned_enumeration_worklist = spec.kind == StageKind::Enumeration;
    let investigation_task_orchestrator = spec.kind == StageKind::Investigation;
    if investigation_task_orchestrator {
        let configured_roles = policy
            .allowed_roles
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let cognitive_catalog = INVESTIGATION_CONFIGURED_COGNITIVE_ROLES
            .into_iter()
            .collect::<BTreeSet<_>>();
        if policy.aggregator_kind != "investigation_primary"
            || policy.aggregator_role != "investigation"
            || policy.allowed_roles.len() != INVESTIGATION_CONFIGURED_COGNITIVE_ROLES.len()
            || configured_roles != cognitive_catalog
        {
            return Err("invalid_investigation_task_orchestrator_policy");
        }
    }
    let coordination_mode = if investigation_task_orchestrator {
        INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE
    } else {
        COMPANY_CONTROLLER_COORDINATION_MODE
    };
    let server_owned_formulaic_worklist =
        server_owned_vuln_worklist || server_owned_enumeration_worklist;
    let child_budget = if server_owned_formulaic_worklist {
        json!({"max_wrapper_calls": 1})
    } else {
        json!({})
    };
    let organization_scope_implicit = !server_owned_formulaic_worklist;

    let leader_manifest = canonicalize_json(&json!({
        "coordination_mode": coordination_mode,
        "role": policy.aggregator_role,
        "stage": spec.kind.as_str(),
    }));
    let leader_work_item = StageWorkItemSeed {
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
    };
    // Investigation starts with a closed, empty governance plan. The current
    // company/asset queue owns runnable admission; only the exact Asset
    // Primary schedule may open an epoch. The Primary then generates dynamic
    // cognition requests from the current asset rather than a host roster.
    // Seeding a generic organization Primary here would
    // reintroduce the retired org-wide topology before an asset was claimed.
    let work_items = if investigation_task_orchestrator {
        Vec::new()
    } else {
        vec![leader_work_item]
    };

    let mut allowed_roles = policy.allowed_roles.clone();
    if investigation_task_orchestrator {
        allowed_roles.extend(
            INVESTIGATION_DYNAMIC_COGNITIVE_ROLES
                .into_iter()
                .map(str::to_string),
        );
    }
    if server_owned_enumeration_worklist {
        allowed_roles.extend(
            [
                EnumerationProducerKind::Preflight,
                EnumerationProducerKind::Content,
                EnumerationProducerKind::Browser,
                EnumerationProducerKind::JsApi,
                EnumerationProducerKind::Parameter,
                EnumerationProducerKind::Resolution,
                EnumerationProducerKind::Coverage,
            ]
            .into_iter()
            .map(|producer| producer.role().to_string()),
        );
    }
    allowed_roles.sort();
    allowed_roles.dedup();
    let mut allowed_dynamic_request_kinds = policy.allowed_dynamic_request_kinds.clone();
    if server_owned_enumeration_worklist {
        allowed_dynamic_request_kinds.push("formulaic_enumeration".to_string());
    }
    allowed_dynamic_request_kinds.sort();
    allowed_dynamic_request_kinds.dedup();
    let child_output_schema = if investigation_task_orchestrator {
        "investigation_cognitive_output.v1"
    } else {
        "stage_worker_output.v1"
    };
    let plan_material = json!({
        "aggregator_kind": policy.aggregator_kind,
        "aggregator_role": policy.aggregator_role,
        "allowed_roles": allowed_roles,
        "child_budget": child_budget,
        "child_output_schema": child_output_schema,
        "coordination_mode": coordination_mode,
        "global_provider_cap": policy.global_provider_cap,
        "allowed_dynamic_request_kinds": allowed_dynamic_request_kinds,
        "max_company_units_active": policy.max_company_units_active,
        "max_dynamic_subject_refs": policy.max_dynamic_subject_refs,
        "max_dynamic_requests": policy.max_dynamic_requests,
        "max_workers": policy.max_workers,
        "organization_scope_implicit": organization_scope_implicit,
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

    // Preserve the historical total exactly in frozen plan material so an
    // active Company Controller operation can replay its seed after upgrade.
    // Runtime admission, claim, retry and coordination do not enforce this
    // compatibility count for `coordination_mode=company_controller`.
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
    let mut dynamic_request_policy = json!({
        "allowed_request_kinds": allowed_dynamic_request_kinds,
        "canonical_subject_refs_only": true,
        "child_budget": child_budget,
        "child_output_schema": child_output_schema,
        "coordination_mode": coordination_mode,
        "global_provider_cap": policy.global_provider_cap,
        "max_company_units_active": policy.max_company_units_active,
        // Same-Turn Gate repair and operator-triggered successor Turns
        // are separate bounded fuels. The latter aligns with the
        // producer contract that may need three attempts before an
        // exact cell can terminalize.
        // EAS can legitimately discover a new exact Web origin while repairing
        // liveness. That newly-authoritative denominator needs one additional
        // bounded repair turn for web fingerprinting; a single repair would
        // terminalize the Controller before it can close the derived cell.
        "max_controller_gate_repairs": 2,
        "max_controller_turn_resumes": MAX_STAGE_TEAM_CONTROLLER_TURN_RESUMES,
        "max_requests": policy.max_dynamic_requests,
        "max_repair_generations": MAX_STAGE_TEAM_REPAIR_GENERATIONS,
        "max_subject_refs": policy.max_dynamic_subject_refs,
        "organization_scope_implicit": organization_scope_implicit,
    });
    if server_owned_vuln_worklist {
        dynamic_request_policy["attempt_policy"] = json!({"max_attempts": 1});
        dynamic_request_policy["formulaic_worklist_executor"] = json!("vuln_v1");
    } else if spec.kind == StageKind::Enumeration {
        dynamic_request_policy["attempt_policy"] = json!({"max_attempts": 2});
        dynamic_request_policy["formulaic_worklist_executor"] = json!("enumeration_v2");
        dynamic_request_policy["enumeration_caps"] = json!({
            "global_browser_jobs": ENUMERATION_GLOBAL_BROWSER_JOB_CAP,
            "global_host_jobs": ENUMERATION_GLOBAL_HOST_JOB_CAP,
            "global_provider_cap": ENUMERATION_GLOBAL_PROVIDER_CAP,
            "max_company_units_active": ENUMERATION_MAX_COMPANY_UNITS_ACTIVE,
            "max_dynamic_requests_per_company": ENUMERATION_MAX_DYNAMIC_REQUESTS_PER_COMPANY,
            "max_workers_per_company": ENUMERATION_MAX_WORKERS_PER_COMPANY,
        });
    }

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
            dynamic_request_policy,
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

    #[test]
    fn candidate_artifact_receipt_uses_only_generic_non_epistemic_output_fields() {
        let output = candidate_artifact_receipt_output(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            &format!("sha256:{}", "a".repeat(64)),
        )
        .expect("valid receipt");
        assert_eq!(
            output.disposition,
            StageWorkerOutputDisposition::ArtifactRecorded
        );
        assert!(output.fact_refs.is_empty());
        assert!(output.evidence_ids.is_empty());
        assert!(output.checked_empty_units.is_empty());
        assert_eq!(
            output
                .canonical_output
                .as_object()
                .expect("receipt object")
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["artifact_hash", "artifact_id", "schema"]
        );
    }

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
    fn investigation_primary_binding_is_planning_only() {
        let (mut plan, mut item) = controller_claim_views();
        plan.stage_kind = StageKind::Investigation.as_str().to_string();
        plan.leader_role = "investigation".to_string();
        plan.aggregator_role = Some("investigation".to_string());
        plan.dynamic_request_policy["coordination_mode"] =
            json!(INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE);
        plan.dynamic_request_policy
            .as_object_mut()
            .expect("policy object")
            .remove("controller_action_compiler");
        item.role = "investigation".to_string();

        let binding = stage_team_leader_binding_for_claim(&plan, &item)
            .expect("exact Investigation Primary receives planning authority");
        assert!(binding.planning_only);
        assert!(binding.controller_action_compiler.is_none());
        assert!(binding.compiled_actions.is_empty());

        item.status = RuntimeStageWorkItemStatus::WaitingDependency;
        let parked = stage_team_leader_binding_for_claim(&plan, &item)
            .expect("parked Investigation Primary retains planning-only continuation authority");
        assert!(parked.planning_only);
        item.status = RuntimeStageWorkItemStatus::Running;

        plan.requests_closed_at = Some(chrono::Utc::now());
        plan.status = RuntimeStageTeamPlanStatus::Finalizing;
        item.status = RuntimeStageWorkItemStatus::Completed;
        let completed = stage_team_leader_binding_for_claim(&plan, &item).expect(
            "completed sealed Investigation Primary retains replay-only planning authority",
        );
        assert!(completed.planning_only);

        plan.requests_closed_at = None;
        assert!(stage_team_leader_binding_for_claim(&plan, &item).is_none());
        plan.requests_closed_at = Some(chrono::Utc::now());
        plan.status = RuntimeStageTeamPlanStatus::Active;
        item.status = RuntimeStageWorkItemStatus::Running;

        item.stable_key = format!("leader:synthesis-recovery:{}", Uuid::from_u128(42));
        item.conflict_key = None;
        let recovery = stage_team_leader_binding_for_claim(&plan, &item)
            .expect("sealed Investigation synthesis recovery retains planning authority");
        assert!(recovery.planning_only);
        assert_eq!(recovery.leader_work_item_id, item.id);

        item.conflict_key = Some("stage_unit_finalizer".to_string());
        assert!(stage_team_leader_binding_for_claim(&plan, &item).is_none());

        let verification_task_id = Uuid::from_u128(77);
        plan.requests_closed_at = None;
        plan.status = RuntimeStageTeamPlanStatus::Active;
        item.status = RuntimeStageWorkItemStatus::Running;
        item.stable_key = format!("task:{verification_task_id}:primary");
        item.work_item_kind = "investigation_primary".to_string();
        item.conflict_key = None;
        item.output_schema = "stage_unit_aggregate.v1".to_string();
        item.created_by = "server_phase_transition".to_string();
        item.input_refs = json!([{
            "id": verification_task_id,
            "kind": "verification_task",
            "subject_fingerprint_sha256": format!("sha256:{}", "a".repeat(64)),
        }]);
        assert!(
            stage_team_leader_binding_for_claim(&plan, &item).is_none(),
            "legacy per-VerificationTask Primary must not regain runnable authority"
        );

        item.stable_key = "leader:primary".to_string();
        item.conflict_key = Some("stage_unit_finalizer".to_string());
        plan.stage_kind = StageKind::TargetIntel.as_str().to_string();
        assert!(stage_team_leader_binding_for_claim(&plan, &item).is_none());
    }

    #[test]
    fn investigation_policy_seeds_closed_governance_without_an_org_primary() {
        let raw_spec: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/investigation/spec.json"
        )))
        .expect("Investigation stage spec JSON");
        assert_eq!(
            raw_spec["team_scheduler"]["coordination_mode"],
            INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE
        );

        let spec =
            load_embedded_stage_spec(StageKind::Investigation).expect("Investigation stage spec");
        let mut base = base_seed();
        base.stage_kind = StageKind::Investigation.as_str().to_string();
        base.specialist = "investigation".to_string();
        base.work_item_key = StageKind::Investigation.as_str().to_string();
        base.agent_path_prefix = "main>stage_run:investigation".to_string();

        let seeded = build_stage_team_seed(&spec, base)
            .expect("valid Investigation TaskOrchestrator policy")
            .expect("Investigation seeds its durable governance envelope");

        assert!(seeded.work_items.is_empty());
        assert_eq!(
            seeded.plan.dynamic_request_policy["coordination_mode"],
            INVESTIGATION_TASK_ORCHESTRATOR_COORDINATION_MODE
        );
        assert!(!seeded
            .plan
            .allowed_roles
            .contains(&"company_stage_controller".to_string()));
        assert_eq!(
            seeded
                .plan
                .allowed_roles
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            [
                "adviser",
                "browser",
                "coder",
                "enricher",
                "investigation",
                "installer",
                "memorist",
                "pentester",
                "researcher",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
        );
        assert!(seeded
            .plan
            .dynamic_request_policy
            .get("controller_action_compiler")
            .is_none());
        assert!(seeded
            .plan
            .dynamic_request_policy
            .get("formulaic_worklist_executor")
            .is_none());
        assert_eq!(
            seeded.plan.dynamic_request_policy["child_output_schema"],
            "investigation_cognitive_output.v1"
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["allowed_request_kinds"],
            json!(["analysis_task", "cognitive_support", "verification_task"])
        );
    }

    #[test]
    fn exact_asset_primary_binds_to_one_lane_without_a_seeded_roster() {
        let spec =
            load_embedded_stage_spec(StageKind::Investigation).expect("Investigation stage spec");
        let mut base = base_seed();
        base.stage_kind = StageKind::Investigation.as_str().to_string();
        base.specialist = "investigation".to_string();
        let seeded = build_stage_team_seed(&spec, base).unwrap().unwrap();
        assert!(seeded.work_items.is_empty());
        assert!(seeded.plan.allowed_roles.iter().any(|role| role == "coder"));
        assert!(seeded
            .plan
            .allowed_roles
            .iter()
            .any(|role| role == "memorist"));
        let plan_id = Uuid::from_u128(10);
        let unit_id = Uuid::from_u128(11);
        let organization_id = Uuid::from_u128(12);
        let asset_lane_id = Uuid::from_u128(13);
        let target_id = Uuid::from_u128(14);
        let evolution_epoch = 2_i64;
        let asset_hash = format!("sha256:{}", "a".repeat(64));
        let plan = StageTeamPlanView {
            id: plan_id,
            operation_id: seeded.base.operation_id,
            stage_execution_id: seeded.base.stage_execution_id,
            stage_run_unit_id: unit_id,
            scope_snapshot_id: Uuid::from_u128(15),
            organization_id,
            stage_kind: StageKind::Investigation.as_str().to_string(),
            unit_generation: 1,
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
            dispatch_epoch: 3,
            requests_closed_at: None,
            final_submitter_kind: seeded.plan.final_submitter_kind,
            final_submitter_worker_run_id: None,
            created_from_stage_spec_hash: seeded.plan.created_from_stage_spec_hash,
            status: RuntimeStageTeamPlanStatus::Active,
            row_version: 4,
        };
        let primary_marker = json!([{
            "kind": "investigation_asset_lane",
            "asset_lane_id": asset_lane_id,
            "target_id": target_id,
            "asset_context_sha256": asset_hash,
            "evolution_epoch": evolution_epoch,
        }]);
        let primary = StageWorkItemView {
            id: Uuid::new_v5(
                &asset_lane_id,
                format!("investigation-asset-primary-work-item-v1:{evolution_epoch}").as_bytes(),
            ),
            stage_team_plan_id: plan_id,
            stage_run_unit_id: unit_id,
            organization_id,
            stable_key: format!("asset:{asset_lane_id}:primary:{evolution_epoch}"),
            work_item_kind: "investigation_asset_primary".to_string(),
            role: "investigation".to_string(),
            input_refs: primary_marker,
            input_manifest_hash: asset_hash.clone(),
            priority: 0,
            required_for_barrier: false,
            is_aggregator: true,
            conflict_key: None,
            attempt_policy: json!({"max_attempts": 3}),
            budget: json!({}),
            output_schema: "stage_unit_aggregate.v1".to_string(),
            created_by: "server_phase_transition".to_string(),
            status: RuntimeStageWorkItemStatus::Running,
            row_version: 1,
        };
        assert!(stage_team_leader_binding_for_claim(&plan, &primary)
            .is_some_and(|binding| binding.planning_only));

        let schedule_round = 4_i64;
        let mut v2_primary = primary;
        v2_primary.input_refs[0]["schedule_round"] = json!(schedule_round);
        v2_primary.id = Uuid::new_v5(
            &asset_lane_id,
            format!("investigation-asset-primary-work-item-v2:{evolution_epoch}:{schedule_round}")
                .as_bytes(),
        );
        v2_primary.stable_key =
            format!("asset:{asset_lane_id}:primary:{evolution_epoch}:round:{schedule_round}");
        assert!(stage_team_leader_binding_for_claim(&plan, &v2_primary)
            .is_some_and(|binding| binding.planning_only));
    }

    #[test]
    fn investigation_policy_rejects_roles_outside_the_cognitive_catalog() {
        let mut spec =
            load_embedded_stage_spec(StageKind::Investigation).expect("Investigation stage spec");
        spec.team_scheduler
            .as_mut()
            .expect("Investigation TeamPlan policy")
            .allowed_roles
            .push("fixed_consult_lane".to_string());

        assert!(matches!(
            build_stage_team_seed(&spec, base_seed()),
            Err("invalid_investigation_task_orchestrator_policy")
        ));
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
            .expect("compatibility request count");
        let reserved_repair_runs =
            MAX_STAGE_TEAM_REPAIR_GENERATIONS * MAX_REPAIR_WORKER_RUNS_PER_GENERATION;
        assert_eq!(
            usize::try_from(first.plan.max_workers_total).expect("compatibility worker total"),
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
    fn company_controller_vuln_plan_is_server_worklist_owned_and_requires_subjects() {
        let spec = load_embedded_stage_spec(StageKind::VulnTriage).expect("vuln spec");
        let mut base = base_seed();
        base.stage_kind = StageKind::VulnTriage.as_str().to_string();
        base.specialist = "vuln_scanner".to_string();
        let seeded = build_stage_team_seed(&spec, base)
            .expect("valid policy")
            .expect("team enabled");

        assert_eq!(
            seeded.plan.dynamic_request_policy["formulaic_worklist_executor"],
            "vuln_v1"
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["organization_scope_implicit"],
            false
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["attempt_policy"],
            json!({"max_attempts": 1})
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["child_budget"],
            json!({"max_wrapper_calls": 1})
        );
    }

    fn vuln_snapshot_with_cells(target_id: Uuid, origin: &str, cells: &[(&str, &str)]) -> Value {
        json!({
            "stage": "vuln_triage",
            "organization_id": Uuid::new_v4(),
            "session_id": Uuid::new_v4(),
            "summary": {
                "total_assets": 1,
                "seed_assets": 1,
                "new_assets": 0,
                "done_assets": 0,
                "pending_assets": 1,
                "blocked_assets": 0
            },
            "assets": [{
                "target_id": target_id,
                "value": origin,
                "target_type": "url",
                "coverage": cells.iter().map(|(technique, state)| json!({
                    "technique": technique,
                    "state": state
                })).collect::<Vec<_>>()
            }]
        })
    }

    #[test]
    fn vuln_worklist_shard_groups_pending_cells_by_exact_origin_and_capability() {
        let first_target = Uuid::new_v4();
        let second_target = Uuid::new_v4();
        let mut snapshot = vuln_snapshot_with_cells(
            first_target,
            "https://one.example:443",
            &[
                ("WSTG-ATHN-02", "pending"),
                ("WSTG-SESS-02", "pending"),
                ("WSTG-CONF-05", "pending"),
                ("WSTG-CRYP-03", "pending"),
                ("WSTG-INFO", "pending"),
                ("GOLISH-NDAY", "not_applicable"),
            ],
        );
        snapshot["assets"].as_array_mut().expect("assets").push(
            vuln_snapshot_with_cells(
                second_target,
                "https://two.example:443",
                &[("WSTG-ATHN-02", "pending")],
            )["assets"][0]
                .clone(),
        );

        let shards = build_vuln_worklist_shards(&snapshot).expect("valid worklist snapshot");

        assert_eq!(shards.len(), 2);
        let first = shards
            .iter()
            .find(|shard| shard.target_id == first_target)
            .expect("first target shard");
        let second = shards
            .iter()
            .find(|shard| shard.target_id == second_target)
            .expect("second target shard");
        assert_eq!(first.target_url, "https://one.example:443");
        assert_eq!(first.tool_name, "vuln_nuclei_general");
        assert_eq!(first.shape, VulnShardShape::Primary);
        assert_eq!(
            first.techniques,
            [
                "WSTG-ATHN-02",
                "WSTG-CONF-05",
                "WSTG-CRYP-03",
                "WSTG-INFO",
                "WSTG-SESS-02",
            ]
        );
        assert_ne!(first.stable_key(), second.stable_key());
    }

    #[test]
    fn vuln_worklist_shard_narrows_partial_or_error_cells_to_one_technique() {
        let target_id = Uuid::new_v4();
        let snapshot = vuln_snapshot_with_cells(
            target_id,
            "https://partial.example:443",
            &[
                ("WSTG-ATHN-02", "partial"),
                ("WSTG-SESS-02", "error"),
                ("WSTG-CONF-05", "checked_empty"),
            ],
        );

        let shards = build_vuln_worklist_shards(&snapshot).expect("valid worklist snapshot");

        assert_eq!(shards.len(), 2);
        assert!(shards
            .iter()
            .all(|shard| shard.shape == VulnShardShape::Narrowed));
        assert!(shards.iter().all(|shard| shard.techniques.len() == 1));
        assert_eq!(
            shards
                .iter()
                .map(|shard| shard.techniques[0].as_str())
                .collect::<Vec<_>>(),
            vec!["WSTG-ATHN-02", "WSTG-SESS-02"]
        );
    }

    #[test]
    fn vuln_worklist_shard_rejects_non_exact_or_unknown_work() {
        for snapshot in [
            vuln_snapshot_with_cells(
                Uuid::nil(),
                "https://example.test:443",
                &[("WSTG-ATHN-02", "pending")],
            ),
            vuln_snapshot_with_cells(
                Uuid::new_v4(),
                "example.test",
                &[("WSTG-ATHN-02", "pending")],
            ),
            vuln_snapshot_with_cells(
                Uuid::new_v4(),
                "https://example.test:443",
                &[("WSTG-UNKNOWN", "pending")],
            ),
        ] {
            assert!(build_vuln_worklist_shards(&snapshot).is_err());
        }
        assert!(!validate_vuln_shard_assignment(
            "vuln_nuclei_general",
            "nuclei_general_baseline",
            "https://example.test:443/path",
            &["WSTG-ATHN-02".to_string()],
        ));
        assert!(!validate_vuln_shard_assignment(
            "vuln_nuclei_general",
            "nuclei_general_dast",
            "https://example.test:443",
            &["WSTG-ATHN-02".to_string()],
        ));
    }

    #[test]
    fn vuln_worklist_shard_stops_after_backend_retry_fuel_is_exhausted() {
        let mut snapshot = vuln_snapshot_with_cells(
            Uuid::new_v4(),
            "https://exhausted.example:443",
            &[("WSTG-ATHN-02", "partial"), ("WSTG-SESS-02", "error")],
        );
        snapshot["assets"][0]["coverage"][0]["details"] = json!({
            "attempt_ordinal": 2,
            "automatic_retry_allowed": false
        });
        snapshot["assets"][0]["coverage"][1]["details"] = json!({
            "attempt_ordinal": MAX_VULN_AUTOMATIC_ATTEMPTS
        });

        let shards = build_vuln_worklist_shards(&snapshot).expect("valid exhausted worklist");

        assert!(shards.is_empty());
    }

    #[test]
    fn vuln_worklist_shard_reopens_one_legacy_scan_budget_exhaustion_at_max_budget() {
        let mut snapshot = vuln_snapshot_with_cells(
            Uuid::new_v4(),
            "https://budget-recovery.example:443",
            &[("WSTG-CONF-05", "partial")],
        );
        snapshot["assets"][0]["coverage"][0]["details"] = json!({
            "attempt_ordinal": MAX_VULN_AUTOMATIC_ATTEMPTS,
            "automatic_retry_allowed": false,
            "failure_owner": "scanner_runtime",
            "failure_class": "scan_budget_exhausted"
        });

        let shards = build_vuln_worklist_shards(&snapshot).expect("valid recovery worklist");

        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0].shape.as_str(), "budget_recovery");
        assert_eq!(shards[0].recovery_attempt, 4);
        assert_eq!(shards[0].techniques, ["WSTG-CONF-05"]);
    }

    #[test]
    fn vuln_worklist_shard_never_reopens_budget_recovery_or_other_runtime_failures() {
        for (attempt_ordinal, failure_class) in [
            (MAX_VULN_AUTOMATIC_ATTEMPTS, "runner_failure"),
            (MAX_VULN_AUTOMATIC_ATTEMPTS, "operator_cancelled"),
            (4, "scan_budget_exhausted"),
        ] {
            let mut snapshot = vuln_snapshot_with_cells(
                Uuid::new_v4(),
                "https://still-exhausted.example:443",
                &[("WSTG-CONF-05", "partial")],
            );
            snapshot["assets"][0]["coverage"][0]["details"] = json!({
                "attempt_ordinal": attempt_ordinal,
                "automatic_retry_allowed": false,
                "failure_owner": "scanner_runtime",
                "failure_class": failure_class
            });

            let shards = build_vuln_worklist_shards(&snapshot).expect("valid exhausted worklist");

            assert!(shards.is_empty(), "unexpected retry for {failure_class}");
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
                "allowed_request_kinds": ["semantic_frontier_task"],
                "canonical_subject_refs_only": true,
                "child_budget": {},
                "child_output_schema": "stage_worker_output.v1",
                "coordination_mode": "company_controller",
                "global_provider_cap": 7,
                "max_company_units_active": 3,
                "max_controller_gate_repairs": 2,
                "max_controller_turn_resumes": 2,
                "max_repair_generations": MAX_STAGE_TEAM_REPAIR_GENERATIONS,
                "max_requests": 32,
                "max_subject_refs": 32,
                "organization_scope_implicit": true,
            })
        );
    }

    #[test]
    fn company_controller_allows_a_second_gate_repair_for_a_derived_denominator() {
        let seeded = build_stage_team_seed(&company_controller_spec(1, 1), base_seed())
            .expect("valid controller policy")
            .expect("team enabled");

        assert_eq!(
            seeded.plan.dynamic_request_policy["max_controller_gate_repairs"],
            json!(2)
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
    fn stage_child_objective_requires_authoritative_ledger_evidence_ids() {
        let spec = load_embedded_stage_spec(StageKind::Enumeration).expect("Enumeration spec");
        let item = stage_child_item();
        let objective = stage_child_objective(&spec, "Example Corp", item.organization_id, &item);

        assert!(objective.contains("list_recent_evidence"));
        assert!(objective.contains("evidence_id"));
        assert!(objective.contains("audit/action"));
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
    fn investigation_child_rejects_authority_bearing_evidence_even_with_advisory_signals() {
        let mut item = stage_child_item();
        item.output_schema = "investigation_cognitive_output.v1".to_string();
        let violation = stage_child_completion_from_result(
            &item,
            Uuid::new_v4(),
            &json!({
                "response": r#"{"business_disposition":"found","summary":"Evidence suggests an authorization boundary hypothesis","fact_refs":[],"evidence_ids":[41],"checked_empty_units":[],"blocker_code":null,"proposal_signals":[{"kind":"authorization_boundary","evidence_ids":[41]}],"action_intents":[{"kind":"read_only_recheck"}],"residuals":[]}"#
            }),
            true,
        )
        .expect_err("Investigation cognitive output cannot re-emit inherited evidence authority");

        assert_eq!(violation.failure_code, "STAGE_TEAM_WORKER_OUTPUT_INVALID");
        assert!(violation.detail.contains("advisory-only"));
    }

    #[test]
    fn investigation_cognitive_worker_may_report_advisory_found_without_claiming_evidence() {
        let mut item = stage_child_item();
        item.output_schema = "investigation_cognitive_output.v1".to_string();
        let output = stage_child_completion_from_result(
            &item,
            Uuid::new_v4(),
            &json!({
                "response": r#"{"business_disposition":"found","summary":"A bounded reasoning lead for Primary review","fact_refs":[],"evidence_ids":[],"checked_empty_units":[],"blocker_code":null,"proposal_signals":[{"kind":"advisory_lead"}],"action_intents":[],"residuals":[]}"#
            }),
            true,
        )
        .expect("advisory-only Investigation cognitive output");

        assert_eq!(output.disposition, StageWorkerOutputDisposition::Found);
        assert!(output.fact_refs.is_empty());
        assert!(output.evidence_ids.is_empty());
        assert_eq!(
            output.canonical_output["proposal_signals"][0]["kind"],
            "advisory_lead"
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

    fn enumeration_shard() -> EnumerationWorklistShard {
        EnumerationWorklistShard {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            target_id: Uuid::new_v4(),
            exact_origin: "https://example.test:443/".to_string(),
            producer: EnumerationProducerKind::Preflight,
            unresolved_cluster_id: Some("cluster:1".to_string()),
            generation: Uuid::new_v4(),
            attempt: 1,
            dependency_lane_receipts_v2: Vec::new(),
            producer_evidence_audit_ids: Vec::new(),
        }
    }

    #[test]
    fn enumeration_plan_uses_the_server_owned_formulaic_worklist() {
        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let mut base = base_seed();
        base.stage_kind = StageKind::Enumeration.as_str().to_string();
        base.specialist = "enumerator".to_string();
        let seeded = build_stage_team_seed(&spec, base).unwrap().unwrap();
        assert!(seeded
            .plan
            .dynamic_request_policy
            .get("controller_action_compiler")
            .is_none());
        assert_eq!(
            seeded.plan.dynamic_request_policy["formulaic_worklist_executor"],
            "enumeration_v2"
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["organization_scope_implicit"],
            false
        );
        assert_eq!(
            seeded.plan.dynamic_request_policy["allowed_request_kinds"],
            json!(["enumeration_resolution", "formulaic_enumeration"])
        );
        assert_eq!(
            seeded.plan.allowed_roles,
            vec![
                "browser_runtime".to_string(),
                "company_stage_controller".to_string(),
                "content_mapper".to_string(),
                "coverage_reviewer".to_string(),
                "js_api_analyzer".to_string(),
                "parameter_analyzer".to_string(),
                "resolution_analyst".to_string()
            ]
        );
        assert_eq!(seeded.work_items.len(), 1);
        assert_eq!(seeded.work_items[0].role, "company_stage_controller");
    }

    #[test]
    fn enumeration_compiled_objective_carries_its_exact_immutable_work_key() {
        let shard = enumeration_shard();
        let objective = shard.typed_objective();

        assert_eq!(
            objective.get("stable_work_key").and_then(Value::as_str),
            Some(shard.stable_key().as_str())
        );
    }

    #[test]
    fn enumeration_shard_uses_the_authorized_canonical_target_subject() {
        let shard = enumeration_shard();

        assert_eq!(
            shard.subject_refs(),
            vec![json!({"kind":"target","target_id":shard.target_id})]
        );
        assert_eq!(shard.typed_objective()["exact_origin"], shard.exact_origin);
    }

    #[test]
    fn enumeration_browser_and_js_shards_use_single_target_formulaic_arguments() {
        let mut shard = enumeration_shard();
        shard.unresolved_cluster_id = None;
        shard.producer = EnumerationProducerKind::Browser;
        assert_eq!(
            shard.formulaic_args(),
            Some(json!({
                "target_id": shard.target_id,
                "target_url": shard.exact_origin,
                "crawl_mode": "standard",
                "ai": false,
                "ai_assist": false,
            }))
        );

        shard.producer = EnumerationProducerKind::JsApi;
        assert_eq!(
            shard.formulaic_args(),
            Some(json!({
                "target_id": shard.target_id,
                "target_url": shard.exact_origin,
                "ai": false,
            }))
        );
    }

    #[test]
    fn enumeration_stable_identity_ignores_the_receipt_replay_observation_bit() {
        let mut shard = enumeration_shard();
        shard.unresolved_cluster_id = None;
        shard.producer = EnumerationProducerKind::JsApi;
        shard.dependency_lane_receipts_v2 = vec![EnumerationLaneClosureReceiptV2 {
            receipt_id: Uuid::new_v4(),
            lane: EnumerationLaneKindV2::Browser,
            execution_authority_id: Uuid::new_v4(),
            artifact_sha256: format!("sha256:{}", "a".repeat(64)),
            receipt_set_sha256: format!("sha256:{}", "b".repeat(64)),
            closure_graph_sha256: format!("sha256:{}", "c".repeat(64)),
            dependency_receipt_ids: Vec::new(),
            evidence_audit_ids: vec![41],
            script_denominator_id: Some(Uuid::new_v4()),
            candidate_denominator_ids: vec![Uuid::new_v4()],
            parameter_denominator_ids: Vec::new(),
            resolution_occurrence_id: None,
            resolution_terminal_receipt_id: None,
            resolution_terminal_receipt_input_id: None,
            terminal_disposition: "found".to_string(),
            entity_set_sha256: format!("sha256:{}", "d".repeat(64)),
            denominator_set_sha256: format!("sha256:{}", "e".repeat(64)),
            script_count: 1,
            candidate_count: 1,
            occurrence_count: 1,
            parameter_assessment_count: 0,
            parameter_fact_count: 0,
            unresolved_count: 0,
            group_count: 1,
            occurrence_link_count: 1,
            api_link_count: 1,
            missing: 0,
            replayed: false,
        }];
        let mut replay = shard.clone();
        replay.dependency_lane_receipts_v2[0].replayed = true;

        assert_eq!(shard.stable_key(), replay.stable_key());
        assert_eq!(shard.typed_objective(), replay.typed_objective());
        assert_eq!(
            replay.typed_objective()["dependency_lane_receipts_v2"][0]["replayed"],
            false
        );
    }

    #[test]
    fn enumeration_shards_partition_exact_origin_and_producer_without_overlap() {
        let shards = enumeration_shards_for_origin(&enumeration_shard(), true);
        let keys = shards
            .iter()
            .map(|shard| shard.stable_key())
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), shards.len());
        assert!(shards
            .iter()
            .all(|shard| shard.exact_origin == "https://example.test:443/"));
    }

    #[test]
    fn enumeration_browser_and_dir_wave_can_run_concurrently() {
        assert_eq!(EnumerationProducerKind::Content.wave(), 1);
        assert_eq!(EnumerationProducerKind::Browser.wave(), 1);
        let terminal = [EnumerationProducerKind::Preflight].into_iter().collect();
        assert!(enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::Content,
            &terminal,
            false
        ));
        assert!(enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::Browser,
            &terminal,
            false
        ));
    }

    #[test]
    fn enumeration_jsapi_waits_for_browser_manifest_receipt() {
        let mut terminal = [EnumerationProducerKind::Preflight].into_iter().collect();
        assert!(!enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::JsApi,
            &terminal,
            false
        ));
        terminal.insert(EnumerationProducerKind::Browser);
        assert!(enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::JsApi,
            &terminal,
            false
        ));
    }

    #[test]
    fn enumeration_parameter_waits_for_runtime_and_static_occurrences() {
        let browser_only = [EnumerationProducerKind::Browser].into_iter().collect();
        assert!(!enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::Parameter,
            &browser_only,
            false
        ));
        let both = [
            EnumerationProducerKind::Browser,
            EnumerationProducerKind::JsApi,
        ]
        .into_iter()
        .collect();
        assert!(enumeration_wave_dependencies_satisfied(
            EnumerationProducerKind::Parameter,
            &both,
            false
        ));
    }

    #[test]
    fn enumeration_parameter_and_coverage_are_in_the_runtime_denominator() {
        let required = enumeration_required_producers_for_techniques([
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-JSAPI",
            "GOLISH-ENUM-PARAM",
        ])
        .unwrap();
        assert_eq!(
            required,
            [
                EnumerationProducerKind::Preflight,
                EnumerationProducerKind::Content,
                EnumerationProducerKind::Browser,
                EnumerationProducerKind::JsApi,
                EnumerationProducerKind::Parameter,
                EnumerationProducerKind::Coverage,
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn enumeration_terminal_projection_does_not_shrink_frozen_producer_denominator() {
        let from_pending =
            enumeration_required_producers_for_techniques(["GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM"])
                .unwrap();
        let from_terminal =
            enumeration_required_producers_for_techniques(["GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM"])
                .unwrap();
        assert_eq!(from_pending, from_terminal);
        assert!(from_terminal.contains(&EnumerationProducerKind::Parameter));
        assert!(from_terminal.contains(&EnumerationProducerKind::Coverage));
    }

    #[test]
    fn enumeration_resolution_worker_only_spawns_for_unresolved_cluster() {
        assert!(!enumeration_shards_for_origin(&enumeration_shard(), false)
            .iter()
            .any(|shard| shard.producer == EnumerationProducerKind::Resolution));
        assert!(enumeration_shards_for_origin(&enumeration_shard(), true)
            .iter()
            .any(|shard| shard.producer == EnumerationProducerKind::Resolution));
    }

    #[test]
    fn enumeration_rolling_window_never_exceeds_company_or_global_caps() {
        assert_eq!(ENUMERATION_MAX_COMPANY_UNITS_ACTIVE, 2);
        assert_eq!(ENUMERATION_MAX_WORKERS_PER_COMPANY, 3);
        assert_eq!(ENUMERATION_GLOBAL_HOST_JOB_CAP, 6);
        assert_eq!(ENUMERATION_GLOBAL_PROVIDER_CAP, 4);
    }

    #[test]
    fn enumeration_browser_jobs_never_exceed_global_two() {
        assert_eq!(ENUMERATION_GLOBAL_BROWSER_JOB_CAP, 2);
    }

    #[test]
    fn enumeration_successor_preserves_authority_but_uses_a_distinct_stable_key() {
        let shard = enumeration_shard();
        let successor = shard.successor();
        assert_ne!(successor.stable_key(), shard.stable_key());
        assert!(successor.stable_key().ends_with(":successor:2"));
        assert_eq!(successor.attempt, shard.attempt + 1);
        assert_eq!(successor.generation, shard.generation);
        assert_eq!(successor.operation_id, shard.operation_id);
        assert_eq!(successor.target_id, shard.target_id);
        assert_eq!(successor.exact_origin, shard.exact_origin);
    }

    #[test]
    fn enumeration_controller_is_only_final_submitter() {
        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let mut base = base_seed();
        base.stage_kind = StageKind::Enumeration.as_str().to_string();
        let seeded = build_stage_team_seed(&spec, base).unwrap().unwrap();
        assert_eq!(seeded.plan.final_submitter_kind, "worker");
        assert_eq!(
            seeded.plan.aggregator_role.as_deref(),
            Some("company_stage_controller")
        );
    }

    #[test]
    fn enumeration_role_tool_masks_are_host_enforced() {
        assert_eq!(
            EnumerationProducerKind::Resolution.role(),
            "resolution_analyst"
        );
        assert_eq!(EnumerationProducerKind::Browser.role(), "browser_runtime");
        assert_eq!(
            EnumerationProducerKind::Coverage.role(),
            "coverage_reviewer"
        );
    }

    #[test]
    fn enumeration_deterministic_lanes_never_dispatch_provider() {
        for producer in [
            EnumerationProducerKind::Preflight,
            EnumerationProducerKind::Content,
            EnumerationProducerKind::Browser,
            EnumerationProducerKind::JsApi,
            EnumerationProducerKind::Parameter,
            EnumerationProducerKind::Coverage,
        ] {
            assert_eq!(producer.execution_kind(), "host_deterministic");
        }
        assert_eq!(
            EnumerationProducerKind::Resolution.execution_kind(),
            "llm_subagent"
        );
    }

    #[test]
    fn free_text_objective_cannot_expand_typed_shard_scope() {
        let shard = enumeration_shard();
        let objective = shard.typed_objective();
        assert_eq!(objective["target_id"], json!(shard.target_id));
        assert_eq!(objective["exact_origin"], shard.exact_origin);
        assert!(objective.get("free_text_subject").is_none());
    }
}
