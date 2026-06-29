//! `execute_stage_run` — the `stage_run` tool handler.
//!
//! 在 chat 的 task 模式里把当前 harness 阶段的多 org / 子公司扇出做完（in-stage、
//! sub_agent per org）。曾经的后端 engagement fleet（overview 一把梭）已移除，本工具
//! 是 chat 内多 org 扇出的现行路径。
//!
//! `stage_run` brings the CLI `--stage-run --include-subsidiaries` behaviour
//! into chat: for the CURRENT harness stage, fan the stage's **specialist**
//! (intel→`recon`, config-driven via `StageSpec::specialist`) out across every
//! in-scope organization, one specialist run per org, each isolated and gated on
//! its own — then aggregate "EVERY org must pass" and report gaps so the main
//! agent can re-run only the failed orgs (gate-closure loop, design
//! `docs/design/2026-06-13-stage-run-fanout-design.md` D2/D6/D9/D11).
//!
//! Architecture (why a loop handler, not a registry tool): dispatching a
//! sub-agent needs the agentic-loop context (`execute_sub_agent`'s
//! `SubAgentExecutorContext`), which a registry `Tool` cannot assemble. So
//! `stage_run` is special-cased in the loop's tool router (like `sub_agent_*`)
//! and reuses [`super::sub_agent_call::execute_sub_agent_call`] per org. The
//! MVP runs orgs **serially** (parallel `run_stage` on one bridge is unsafe —
//! shared harness side-channels / conversation history / cancel flag); K-
//! concurrency via `JoinSet` is a follow-up that keeps the per-org isolation.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::{json, Value};

use golish_agent_kit::db_traits::{OrgScopeUnit, StageAssetWaveView};
use golish_agent_kit::harness::org_gate::{
    completion_is_fresh_for_stage, decide_org_verdict, fanout_completion_scope_ids,
    stage_pass_token, STAGE_COMPLETION_TTL_SECS, STAGE_RUN_PASS_TOKEN_KIND,
};
use golish_agent_kit::harness::{
    allowed_tool_names, evaluate_org_stage_gate, load_embedded_stage_spec, stage_methodology_md,
    HarnessRecoveryActions, OrgVerdict, StageDeliverable, StageKind,
};
use golish_agent_kit::task_orchestrator::agent_run_checkpoint::{
    agent_run_from_state_blob, state_blob_with_agent_run, state_blob_without_agent_run,
    AgentRunCheckpoint, AgentRunStatus, RuntimeCorrectionCheckpoint, ToolCheckpoint,
    ToolCheckpointState,
};
use golish_agent_kit::task_orchestrator::stage_refiner::{
    refine_gate_block, RefinerContext, RepairDirective,
};
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_sub_agents::{
    submit_coverage_gap_repair_mode_from_reasons, SubAgentContext, SubmitRepairMode,
};

use super::super::super::{AgenticLoopContext, ToolExecutionResult};
use super::sub_agent_call::execute_sub_agent_call;

/// One per-org unit the fan-out runs the stage specialist against.
#[derive(Debug, Clone, PartialEq)]
struct OrgUnit {
    id: String,
    name: String,
    ownership_percent: Option<f64>,
}

#[derive(Debug, Clone)]
struct QueuedStageAssetBatch {
    org_id: String,
    org_name: String,
    wave_index: i32,
    asset_count: usize,
}

const STAGE_RUN_WORKERS_KEY: &str = "stage_run_workers";
const MAX_STAGE_ASSET_WAVE_ASSETS: i64 = 200;

/// Parse the `orgs` argument into per-org units. The main agent passes the
/// in-scope organization tree it built during scoping (each `{id, name,
/// ownership_percent?}`); the per-org gate enforces DB truth downstream, so a
/// fabricated org simply fails its own gate.
fn parse_org_units(args: &Value) -> Vec<OrgUnit> {
    args.get("orgs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let id = o.get("id").and_then(|v| v.as_str())?.trim().to_string();
                    if id.is_empty() {
                        return None;
                    }
                    let name = o
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    let ownership_percent = o.get("ownership_percent").and_then(|v| v.as_f64());
                    Some(OrgUnit {
                        id,
                        name,
                        ownership_percent,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn org_unit_from_scope_unit(unit: OrgScopeUnit) -> OrgUnit {
    OrgUnit {
        id: unit.id.to_string(),
        name: if unit.name.trim().is_empty() {
            unit.id.to_string()
        } else {
            unit.name
        },
        ownership_percent: None,
    }
}

fn merge_with_authoritative_subtree(
    requested: Vec<OrgUnit>,
    authoritative: Vec<OrgUnit>,
) -> (Vec<OrgUnit>, Vec<String>, Vec<String>) {
    let requested_by_id: HashMap<String, OrgUnit> = requested
        .iter()
        .cloned()
        .map(|unit| (unit.id.clone(), unit))
        .collect();
    let authoritative_ids: HashSet<String> =
        authoritative.iter().map(|unit| unit.id.clone()).collect();
    let rejected = requested
        .iter()
        .filter(|unit| !authoritative_ids.contains(&unit.id))
        .map(|unit| unit.name.clone())
        .collect::<Vec<_>>();

    let mut added = Vec::new();
    let mut merged = Vec::with_capacity(authoritative.len());
    for mut unit in authoritative {
        match requested_by_id.get(&unit.id) {
            Some(requested) => {
                if unit.ownership_percent.is_none() {
                    unit.ownership_percent = requested.ownership_percent;
                }
            }
            None => added.push(unit.name.clone()),
        }
        merged.push(unit);
    }
    (merged, added, rejected)
}

/// Title-case a stage id for display: `target_intel` → `Target Intel`.
fn stage_label_for(stage: StageKind) -> String {
    stage
        .as_str()
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Display label for the specialist slug: `recon` → `Recon`.
fn role_label_for(specialist: &str) -> String {
    let mut c = specialist.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect::<String>(),
        None => specialist.to_string(),
    }
}

/// Build the per-org objective for the specialist (pins the org id + scope to
/// THIS org only, so the specialist registers assets against the right org).
///
/// The objective also front-loads the stage's COVERAGE CONTRACT (`expected_techniques`)
/// and tool boundary (`allowed_tool_types`) so the specialist fills the coverage
/// matrix and stays in-stage BEFORE submitting — instead of learning the gate's
/// requirements only after the deliverable is rejected (the observed retry loop:
/// "intel coverage incomplete: never attempted (asset × technique)").
fn build_org_objective(
    stage: StageKind,
    unit: &OrgUnit,
    expected_techniques: &[String],
    allowed_tool_types: &[String],
) -> String {
    let mut obj = format!(
        "Run the {} stage for this engagement. Organization: {} (organization_id: {}). \
         Collect for THIS organization only — discover its own assets and register them as \
         in-scope targets bound to this organization_id, then submit the stage deliverable.",
        stage.as_str(),
        unit.name,
        unit.id,
    );
    if !allowed_tool_types.is_empty() {
        obj.push_str(&format!(
            " TOOLS: in the {} stage you may only use these tool types: [{}].",
            stage.as_str(),
            allowed_tool_types.join(", "),
        ));
        // Q3 ①+ · resolve the type-selectors into the CONCRETE tool names this
        // stage permits, so a weak model does not have to translate `recon/dns`
        // → `dig` itself (and wrongly translate `nmap` into a tool it can only
        // get BLOCKED on). Consistent with the dispatch guard (both resolve via
        // stage_allows), so every name advertised here is one that will run.
        let tool_names = allowed_tool_names(allowed_tool_types);
        if !tool_names.is_empty() {
            obj.push_str(&format!(
                " Concretely, the only tools you may run here are: [{}] — invoke them via \
                 pentest_run (or run_pty_cmd). Any tool NOT in that list is out-of-stage and will \
                 be BLOCKED; do not call it.",
                tool_names.join(", "),
            ));
        }
        obj.push_str(
            " If a tool is blocked by the stage boundary, switch technique or submit your \
             deliverable — do NOT retry the blocked tool. Long scans may be backgrounded on a soft \
             timeout: do NOT re-run the same command. submit_stage_deliverable waits for \
             attributed background jobs to finish before grading; if it reports jobs still \
             running, wait for their completion notes and resubmit. Only inspect/kill a \
             background job if it is clearly hung.",
        );
    }
    if !expected_techniques.is_empty() {
        obj.push_str(&format!(
            " COVERAGE CONTRACT (this stage is GATED on it): the expected techniques are [{}]. \
             For EVERY in-scope asset you discover/confirm, add ONE coverage cell per technique to \
             your StageDeliverable with a terminal status: found (cite real evidence_refs from the \
             tool run) | checked_empty (cite the probe evidence proving you ran it) | \
             blocked/not_applicable (give a note). A MISSING (asset × technique) cell counts as \
             not_attempted and FAILS the gate. Tag each corroborating claim/finding with the SAME \
             technique id and the SAME asset as its subject. Always cite the REAL evidence ids your \
             tools returned — never placeholder ids like 1, 2, 3.",
            expected_techniques.join(", "),
        ));
        obj.push_str(&format!(
            " PRE-SUBMIT SELF-CHECK (mandatory): before calling submit_stage_deliverable, call \
             check_stage_asset_coverage with stage=\"{}\" and organization_id=\"{}\". If \
             ready_to_submit=false, do NOT submit yet; use gap_examples, cell_summary, and \
             next_action to close missing cells, wait for attributed background jobs, or record \
             honest blocked/not_applicable terminal coverage. Only call submit_stage_deliverable \
             after ready_to_submit=true. next_wave_pending is visible expansion backlog for a \
             later global delta pass and does not block the current batch.",
            stage.as_str(),
            unit.id,
        ));
    }
    if stage == StageKind::ExternalAttackSurface {
        obj.push_str(
            " EAS SCAN STRATEGY: this is coverage-driven, not a fixed pipeline. Start by \
             understanding the current asset/coverage state with check_stage_asset_coverage \
             and query_target_data, then choose the smallest useful batch that closes real gaps. \
             Use httpx early when liveness/HTTP evidence is missing, but do not treat it as \
             a mechanical prerequisite for every later action when fresh DB truth already exists. \
             Do not run broad `nmap -sV -iL` against every raw in-scope domain/IP. Confirm open \
             ports with naabu/masscan/nmap port-scan output or existing target port data, then \
             run service fingerprinting only on confirmed open host:port groups. Normalize URL \
             assets before nmap; never feed `https://...` URL strings to nmap target lists. If an \
             asset has no open ports, cannot resolve, or is URL-only for PORT/SERVICE, close the \
             applicable cells with honest checked_empty/blocked/not_applicable terminal coverage \
             and a concrete note instead of launching a speculative service sweep. If a scan is \
             backgrounded, use wait_for_background_jobs as an incremental visible wait/check loop: \
             when any job completes, inspect its output and newly landed evidence before deciding \
             whether the remaining jobs should continue, be narrowed, or be killed. If it returns \
             idle_timeout or check_job shows no useful progress, kill_job the stuck/broad job \
             before submitting or narrowing the batch.",
        );
    }
    // The recon "how-to" playbook belongs to the WORKER that actually collects
    // (this specialist sub-agent), not the orchestrator. Append the stage
    // methodology here — recommended tool sequence / efficiency red lines /
    // coverage contract — so the worker gets it; the main agent no longer carries
    // it for specialist stages (see task_orchestrator subtask_phases::execute).
    if let Some(md) = stage_methodology_md(stage)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        obj.push_str(&format!(
            "\n\n## HOW TO RUN {} (methodology — follow this)\n\n{}",
            stage.as_str(),
            md,
        ));
    }
    obj
}

/// Emit a [`HarnessTraceKind::StageRunOrgProgress`] for one org row.
///
/// `agent_request_id` is the org's specialist sub-agent's `parent_request_id`
/// (its `sub_agent_*` events share it), letting the UI drill from the org row
/// into that org's own conversation + tool calls.
#[allow(clippy::too_many_arguments)]
fn emit_org_progress(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
    agent_request_id: &str,
    status: &str,
    activity: Option<String>,
    evidence_count: u32,
    stage_label: &str,
    role_label: &str,
    coverage_axis: &[String],
) {
    let event = AiEvent::HarnessTrace {
        operation_id: ctx.events.session_id.unwrap_or("").to_string(),
        stage: stage.as_str().to_string(),
        agent_path: "main".to_string(),
        trace: HarnessTraceKind::StageRunOrgProgress {
            org_id: unit.id.clone(),
            org_name: unit.name.clone(),
            agent_request_id: Some(agent_request_id.to_string()),
            ownership_percent: unit.ownership_percent,
            status: status.to_string(),
            coverage: Vec::new(),
            evidence_count,
            activity,
            stage_label: stage_label.to_string(),
            role_label: role_label.to_string(),
            coverage_axis: coverage_axis.to_vec(),
        },
    };
    let _ = ctx.events.event_tx.send(event);
}

/// Resume-skip lookup: returns the prior `passed_at` iff this org already passed
/// `stage` within the TTL window, so the caller can skip re-dispatching the
/// specialist. Fail-open: no `db_tracker` (pure-eval contexts), unparseable org
/// id, no ledger row, or a stale row → `None` (run normally).
async fn resume_skip_passed_at(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
    not_before: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let tracker = ctx.events.db_tracker?;
    let org_id = uuid::Uuid::parse_str(&unit.id).ok()?;
    let passed_at = tracker
        .recent_org_stage_completion(org_id, stage.as_str())
        .await?;
    resume_skip_is_allowed(passed_at, chrono::Utc::now(), not_before).then_some(passed_at)
}

fn resume_skip_is_allowed(
    passed_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    not_before: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    completion_is_fresh_for_stage(passed_at, now, STAGE_COMPLETION_TTL_SECS, not_before)
}

fn resume_skip_covers_current_wave(
    passed_at: chrono::DateTime<chrono::Utc>,
    current_wave: Option<&StageAssetWaveView>,
    legacy_wave_items_covered_by_pass: bool,
) -> bool {
    current_wave
        .map(|wave| passed_at >= wave.started_at || legacy_wave_items_covered_by_pass)
        .unwrap_or(true)
}

fn active_stage_skip_floor_from_state(
    state: &golish_agent_kit::db_traits::OperationStateView,
    stage: StageKind,
) -> Option<chrono::DateTime<chrono::Utc>> {
    (StageKind::try_parse(&state.current_stage) == Some(stage)).then_some(state.stage_started_at)
}

async fn active_stage_skip_floor(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = stage_run_operation_id(ctx)?;
    let state = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()?;
    active_stage_skip_floor_from_state(&state, stage)
}

fn parse_sub_agent_session_id(response: &str) -> Option<uuid::Uuid> {
    let marker = "[sub_agent_session_id:";
    let start = response.rfind(marker)? + marker.len();
    let id = response[start..].split(']').next()?.trim();
    uuid::Uuid::parse_str(id).ok()
}

fn stage_run_worker_chain_from_blob(
    blob: &Value,
    stage: StageKind,
    org_id: &str,
    specialist: &str,
) -> Option<uuid::Uuid> {
    let entry = blob
        .get(STAGE_RUN_WORKERS_KEY)?
        .get(stage.as_str())?
        .get(org_id)?;
    let stored_specialist = entry.get("specialist").and_then(|v| v.as_str())?;
    if stored_specialist != specialist {
        return None;
    }
    entry
        .get("chain_id")
        .and_then(|v| v.as_str())
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
}

fn ensure_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().unwrap()
}

fn upsert_stage_run_worker_blob(
    mut blob: Value,
    stage: StageKind,
    unit: &OrgUnit,
    specialist: &str,
    org_request_id: &str,
    chain_id: uuid::Uuid,
) -> Value {
    let root = ensure_object(&mut blob);
    let workers = root
        .entry(STAGE_RUN_WORKERS_KEY.to_string())
        .or_insert_with(|| json!({}));
    let stage_map = ensure_object(
        ensure_object(workers)
            .entry(stage.as_str().to_string())
            .or_insert_with(|| json!({})),
    );
    stage_map.insert(
        unit.id.clone(),
        json!({
            "chain_id": chain_id.to_string(),
            "specialist": specialist,
            "org_name": unit.name.clone(),
            "tool_call_id": org_request_id,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        }),
    );
    blob
}

fn stage_run_operation_id(ctx: &AgenticLoopContext<'_>) -> Option<uuid::Uuid> {
    ctx.harness_operation_id.or_else(|| {
        ctx.events
            .db_tracker
            .map(|tracker| tracker.task_id().unwrap_or_else(|| tracker.session_uuid()))
    })
}

fn stage_asset_wave_instruction(stage: StageKind, wave: &StageAssetWaveView) -> String {
    let preview_limit = 80usize;
    let mut assets = wave
        .asset_values
        .iter()
        .take(preview_limit)
        .map(|asset| format!("- {asset}"))
        .collect::<Vec<_>>();
    if wave.asset_values.len() > preview_limit {
        assets.push(format!(
            "- ... {} more assets in this wave",
            wave.asset_values.len() - preview_limit
        ));
    }
    let asset_list = if assets.is_empty() {
        "- (empty wave)".to_string()
    } else {
        assets.join("\n")
    };
    format!(
        "## CURRENT ASSET WAVE\n\n\
         This {} run is on durable asset wave #{} ({} asset(s), hash {}). \
         Close coverage only for the assets in this batch. Assets discovered while \
         this batch runs are intentionally held as expansion backlog for a later \
         global delta pass and must not be counted as current coverage gaps.\n\n{}",
        stage.as_str(),
        wave.wave_index + 1,
        wave.asset_values.len(),
        wave.asset_hash,
        asset_list
    )
}

async fn prepare_stage_asset_wave(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
    started_at: chrono::DateTime<chrono::Utc>,
) -> Option<StageAssetWaveView> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = stage_run_operation_id(ctx)?;
    let organization_id = uuid::Uuid::parse_str(&unit.id).ok()?;
    match repo
        .stage_asset_wave_current_or_create_initial(
            operation_id,
            organization_id,
            stage.as_str(),
            started_at,
            MAX_STAGE_ASSET_WAVE_ASSETS,
        )
        .await
    {
        Ok(wave) => wave,
        Err(error) => {
            tracing::warn!(
                target: "harness::stage_run",
                stage = %stage.as_str(),
                org_id = %unit.id,
                error = %error,
                "stage_run could not prepare durable asset wave; falling back to stage-start cutoff"
            );
            None
        }
    }
}

async fn current_running_stage_asset_wave(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
) -> Option<StageAssetWaveView> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = stage_run_operation_id(ctx)?;
    let organization_id = uuid::Uuid::parse_str(&unit.id).ok()?;
    match repo
        .stage_asset_wave_current_running(operation_id, organization_id, stage.as_str())
        .await
    {
        Ok(wave) => wave,
        Err(error) => {
            tracing::warn!(
                target: "harness::stage_run",
                stage = %stage.as_str(),
                org_id = %unit.id,
                error = %error,
                "stage_run could not read current durable asset wave"
            );
            None
        }
    }
}

async fn stage_asset_wave_items_covered_by_pass(
    ctx: &AgenticLoopContext<'_>,
    wave: &StageAssetWaveView,
    passed_at: chrono::DateTime<chrono::Utc>,
) -> bool {
    let Some(tracker) = ctx.events.db_tracker else {
        return false;
    };
    let Some(repo) = tracker.repo() else {
        return false;
    };
    match repo
        .stage_asset_wave_all_items_created_at_or_before(wave.id, passed_at)
        .await
    {
        Ok(covered) => covered,
        Err(error) => {
            tracing::warn!(
                target: "harness::stage_run",
                wave_id = %wave.id,
                wave_index = wave.wave_index,
                error = %error,
                "stage_run could not compare asset wave items against org pass time"
            );
            false
        }
    }
}

async fn complete_stage_asset_wave(
    ctx: &AgenticLoopContext<'_>,
    wave: &StageAssetWaveView,
) -> std::result::Result<(), String> {
    let Some(tracker) = ctx.events.db_tracker else {
        return Ok(());
    };
    let Some(repo) = tracker.repo() else {
        return Ok(());
    };
    if let Err(error) = repo.stage_asset_wave_complete(wave.id).await {
        tracing::warn!(
            target: "harness::stage_run",
            wave_id = %wave.id,
            wave_index = wave.wave_index,
            error = %error,
            "stage_run failed to mark asset wave completed"
        );
        return Err(format!(
            "asset wave #{} passed gate but could not be marked completed: {error}",
            wave.wave_index + 1
        ));
    }
    Ok(())
}

async fn queue_global_delta_asset_batches(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    units: &[OrgUnit],
) -> std::result::Result<Vec<QueuedStageAssetBatch>, String> {
    let Some(tracker) = ctx.events.db_tracker else {
        return Ok(Vec::new());
    };
    let Some(repo) = tracker.repo() else {
        return Ok(Vec::new());
    };
    let Some(operation_id) = stage_run_operation_id(ctx) else {
        return Ok(Vec::new());
    };

    let mut queued = Vec::new();
    for unit in units {
        let organization_id = match uuid::Uuid::parse_str(&unit.id) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let next = repo
            .stage_asset_wave_create_next(
                operation_id,
                organization_id,
                stage.as_str(),
                None,
                MAX_STAGE_ASSET_WAVE_ASSETS,
            )
            .await
            .map_err(|error| {
                format!(
                    "global delta expansion queue failed for {} ({}): {error}",
                    unit.name, unit.id
                )
            })?;
        if let Some(next) = next {
            queued.push(QueuedStageAssetBatch {
                org_id: unit.id.clone(),
                org_name: unit.name.clone(),
                wave_index: next.wave_index,
                asset_count: next.asset_values.len(),
            });
        }
    }
    Ok(queued)
}

fn stage_run_agent_path(stage: StageKind, unit: &OrgUnit, specialist: &str) -> String {
    format!(
        "main>stage_run:{}>org:{}>{}",
        stage.as_str(),
        unit.id,
        specialist
    )
}

fn repair_kind_label(directive: &RepairDirective) -> String {
    serde_json::to_string(&directive.repair_kind)
        .unwrap_or_else(|_| "\"generic\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn emit_stage_refiner_decision(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    agent_path: &str,
    directive: &RepairDirective,
) {
    let operation_id = ctx
        .harness_operation_id
        .map(|id| id.to_string())
        .or_else(|| ctx.events.session_id.map(str::to_string));
    let Some(operation_id) = operation_id else {
        return;
    };
    let _ = ctx.events.event_tx.send(AiEvent::HarnessTrace {
        operation_id,
        stage: stage.as_str().to_string(),
        agent_path: agent_path.to_string(),
        trace: HarnessTraceKind::StageRefinerDecision {
            repair_kind: repair_kind_label(directive),
            root_cause: directive.root_cause.clone(),
            action_count: directive.actions.len().min(u32::MAX as usize) as u32,
            gap_count: directive
                .submit_guidance
                .required_coverage_cells
                .len()
                .min(u32::MAX as usize) as u32,
            llm_escalated: directive.llm_escalated,
            directive_hash: directive.gate_reason_hash.clone(),
        },
    });
}

async fn load_stage_run_worker_chain(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
    specialist: &str,
) -> Option<uuid::Uuid> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = stage_run_operation_id(ctx)?;
    let state = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()?;
    stage_run_worker_chain_from_blob(&state.state_blob, stage, &unit.id, specialist)
}

async fn load_stage_run_agent_checkpoint(
    ctx: &AgenticLoopContext<'_>,
    agent_path: &str,
) -> Option<AgentRunCheckpoint> {
    let tracker = ctx.events.db_tracker?;
    let repo = tracker.repo()?;
    let operation_id = stage_run_operation_id(ctx)?;
    let state = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()?;
    let checkpoint = agent_run_from_state_blob(&state.state_blob)?;
    (checkpoint.agent_path == agent_path).then_some(checkpoint)
}

fn pending_stage_run_retry_from_checkpoint(
    checkpoint: &AgentRunCheckpoint,
    max_attempts: usize,
) -> Option<(usize, String)> {
    if checkpoint.status != AgentRunStatus::GateBlocked {
        return None;
    }
    let completed_attempt = checkpoint.llm_turn_index? as usize;
    if completed_attempt == 0 || completed_attempt >= max_attempts {
        return None;
    }
    let feedback = checkpoint.pending_gate_correction.clone()?;
    Some((completed_attempt, feedback))
}

async fn persist_stage_run_agent_checkpoint(
    ctx: &AgenticLoopContext<'_>,
    checkpoint: AgentRunCheckpoint,
) {
    let Some(tracker) = ctx.events.db_tracker else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let Some(operation_id) = stage_run_operation_id(ctx) else {
        return;
    };
    let current = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
        .unwrap_or_default();
    let next = state_blob_with_agent_run(current, &checkpoint);
    if let Err(e) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::stage_run",
            agent_path = %checkpoint.agent_path,
            error = %e,
            "stage_run failed to persist agent-run checkpoint"
        );
    }
}

async fn clear_stage_run_agent_checkpoint(ctx: &AgenticLoopContext<'_>, agent_path: &str) {
    let Some(tracker) = ctx.events.db_tracker else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let Some(operation_id) = stage_run_operation_id(ctx) else {
        return;
    };
    let current = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
        .unwrap_or_default();
    let should_clear = agent_run_from_state_blob(&current)
        .map(|checkpoint| checkpoint.agent_path == agent_path)
        .unwrap_or(false);
    if !should_clear {
        return;
    }
    let next = state_blob_without_agent_run(current);
    if let Err(e) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::stage_run",
            agent_path = %agent_path,
            error = %e,
            "stage_run failed to clear agent-run checkpoint"
        );
    }
}

struct StageRunCheckpointInput<'a> {
    operation_id: Option<uuid::Uuid>,
    stage: StageKind,
    agent_path: &'a str,
    attempt: usize,
    org_request_id: &'a str,
    sub_agent_tool: &'a str,
    chain_id: Option<uuid::Uuid>,
    status: AgentRunStatus,
    pending_gate_correction: Option<String>,
    correction_kind: Option<&'a str>,
    submit_repair_mode: Option<SubmitRepairMode>,
    repair_directive: Option<RepairDirective>,
}

fn build_stage_run_agent_checkpoint(input: StageRunCheckpointInput<'_>) -> AgentRunCheckpoint {
    AgentRunCheckpoint {
        schema_v: 1,
        operation_id: input.operation_id,
        stage: Some(input.stage.as_str().to_string()),
        stage_attempt_id: None,
        agent_path: input.agent_path.to_string(),
        status: input.status,
        llm_turn_index: Some(input.attempt as u64),
        message_chain_ref: input.chain_id.map(|id| id.to_string()),
        pending_gate_correction: input.pending_gate_correction.clone(),
        pending_submit_only: false,
        submit_repair_mode: input
            .submit_repair_mode
            .as_ref()
            .and_then(|mode| serde_json::to_value(mode).ok()),
        repair_directive: input
            .repair_directive
            .as_ref()
            .and_then(|directive| serde_json::to_value(directive).ok()),
        runtime_corrections: input
            .pending_gate_correction
            .map(|message| {
                vec![RuntimeCorrectionCheckpoint {
                    source: if input.repair_directive.is_some() {
                        "stage_refiner".to_string()
                    } else {
                        "rule".to_string()
                    },
                    kind: input
                        .correction_kind
                        .unwrap_or("per_org_gate_block")
                        .to_string(),
                    message,
                    job_ids: Vec::new(),
                    evidence_ids: Vec::new(),
                    submit_allowed: matches!(
                        input.submit_repair_mode.as_ref().map(|mode| mode.kind),
                        Some(golish_sub_agents::SubmitRepairKind::EvidenceRefs)
                    ),
                }]
            })
            .unwrap_or_default(),
        background_job_ids: Vec::new(),
        evidence_watermark: None,
        last_tool: Some(ToolCheckpoint {
            tool_call_id: input.org_request_id.to_string(),
            tool_name: input.sub_agent_tool.to_string(),
            state: ToolCheckpointState::Completed,
            result_ref: input.chain_id.map(|id| format!("message_chain:{id}")),
        }),
        updated_at: chrono::Utc::now(),
    }
}

async fn persist_stage_run_worker_chain(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
    specialist: &str,
    org_request_id: &str,
    chain_id: uuid::Uuid,
) {
    let Some(tracker) = ctx.events.db_tracker else {
        return;
    };
    let Some(repo) = tracker.repo() else {
        return;
    };
    let Some(operation_id) = stage_run_operation_id(ctx) else {
        return;
    };
    let current = golish_agent_kit::db_shim::operation_state::get(repo, operation_id)
        .await
        .ok()
        .flatten()
        .map(|state| state.state_blob)
        .unwrap_or_default();
    let next =
        upsert_stage_run_worker_blob(current, stage, unit, specialist, org_request_id, chain_id);
    if let Err(e) =
        golish_agent_kit::db_shim::operation_state::write_state_blob(repo, operation_id, next).await
    {
        tracing::warn!(
            target: "harness::stage_run",
            stage = %stage.as_str(),
            org_id = %unit.id,
            specialist = %specialist,
            chain_id = %chain_id,
            error = %e,
            "stage_run failed to persist worker resume chain"
        );
    }
}

/// Phase 2 闸1·A-lite: max gate attempts per org before giving up to `gaps`
/// (1 initial dispatch + 2 feedback retries). Exceeding it still records a gap so
/// the main agent's gap-closure loop can take over.
const MAX_ORG_GATE_ATTEMPTS: usize = 3;

/// Next action for an org after one attempt's gate verdict (pure control-flow,
/// unit-tested). `max_attempts == 1` (the no-DB fallback path) makes a BLOCK
/// terminal so eval/headless runs never retry.
#[derive(Debug, PartialEq, Eq)]
enum OrgAttemptOutcome {
    /// Gate passed — count the org + write the completion ledger.
    Passed,
    /// Gate blocked with attempts left — re-dispatch the specialist with `feedback`.
    Retry { feedback: String },
    /// Gate blocked with no attempts left — record a gap carrying `reasons`.
    Exhausted { reasons: Vec<String> },
}

/// Decide what to do after a per-org attempt produced `verdict`. `attempt` is the
/// 1-based number of the attempt that just ran; `max_attempts` is the cap.
fn next_org_action(verdict: &OrgVerdict, attempt: usize, max_attempts: usize) -> OrgAttemptOutcome {
    match verdict {
        OrgVerdict::Pass => OrgAttemptOutcome::Passed,
        OrgVerdict::Block { reasons, .. } => {
            if attempt < max_attempts {
                OrgAttemptOutcome::Retry {
                    feedback: gate_retry_feedback(attempt + 1, max_attempts, reasons),
                }
            } else {
                OrgAttemptOutcome::Exhausted {
                    reasons: reasons.clone(),
                }
            }
        }
    }
}

fn fallback_org_verdict(repo_available: bool, sub_ok: bool) -> (OrgVerdict, bool) {
    if repo_available {
        return (
            OrgVerdict::Block {
                reasons: vec![
                    "sub-agent completed without a StageDeliverable accepted by the per-org gate. \
                     It may have received needs_fix from submit_stage_deliverable (for example, \
                     pending background jobs or missing evidence). Close that feedback and submit \
                     again before this organization can pass."
                        .to_string(),
                ],
                recovery_actions: HarnessRecoveryActions::default(),
            },
            true,
        );
    }

    let verdict = if sub_ok {
        OrgVerdict::Pass
    } else {
        OrgVerdict::Block {
            reasons: vec!["sub-agent did not complete".to_string()],
            recovery_actions: HarnessRecoveryActions::default(),
        }
    };
    (verdict, false)
}

/// Build the feedback block appended to the specialist's objective on a retry,
/// naming the gate's BLOCK reasons so it closes exactly those gaps. `attempt` is
/// the (1-based) NEXT attempt number being launched.
fn gate_retry_feedback(attempt: usize, max_attempts: usize, reasons: &[String]) -> String {
    let reasons_block = if reasons.is_empty() {
        "the per-org stage gate did not pass (no specific reasons returned)".to_string()
    } else {
        reasons
            .iter()
            .map(|r| format!("- {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## GATE FEEDBACK — retry {attempt}/{max_attempts}\n\n\
         Your previous deliverable for THIS organization did NOT pass the per-org \
         stage gate. The evidence you already collected is saved in the ledger — do \
         NOT redo it; focus only on closing these specific gaps, then submit the \
         StageDeliverable again:\n\n{reasons_block}"
    )
}

fn stage_run_gate_repair_directive(
    stage: StageKind,
    org_id: Option<uuid::Uuid>,
    agent_path: String,
    reasons: Vec<String>,
    recovery_actions: &HarnessRecoveryActions,
) -> RepairDirective {
    refine_gate_block(RefinerContext {
        stage,
        org_id,
        agent_path,
        reasons,
        coverage_gap_actions: recovery_actions.coverage_gap_actions.clone(),
        available_evidence_ids: Vec::new(),
        running_background_jobs: Vec::new(),
    })
}

/// Handle the `stage_run` tool call: per-org serial specialist fan-out.
pub(super) async fn execute_stage_run<M>(
    tool_args: &Value,
    ctx: &AgenticLoopContext<'_>,
    model: &M,
    context: &SubAgentContext,
    tool_id: &str,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    // 1. Resolve the active stage + its specialist/coverage config (Task 5).
    let Some(stage) = ctx.harness_stage else {
        return Ok(ToolExecutionResult {
            value: json!({
                "error": "stage_run can only run inside an active harness stage. \
                          It fans the current stage's specialist out per organization."
            }),
            success: false,
        });
    };
    let spec = match load_embedded_stage_spec(stage) {
        Ok(s) => s,
        Err(e) => {
            return Ok(ToolExecutionResult {
                value: json!({ "error": format!("could not load stage spec for {}: {e}", stage.as_str()) }),
                success: false,
            });
        }
    };
    let Some(specialist) = spec.specialist.clone().filter(|s| !s.is_empty()) else {
        return Ok(ToolExecutionResult {
            value: json!({
                "error": format!(
                    "stage '{}' has no `specialist` configured, so stage_run does not apply here. \
                     Run this stage directly instead.",
                    stage.as_str()
                )
            }),
            success: false,
        });
    };
    let stage_label = stage_label_for(stage);
    let role_label = role_label_for(&specialist);
    let coverage_axis = spec.coverage_axis.clone();

    // 2. Per-org units. The model still passes `orgs` as a convenient hint, but
    // once scoping has bound an engagement root the DB org subtree is the
    // authoritative fan-out set. Continuation/repair turns are especially prone
    // to reconstructing an incomplete org list from memory; `stage_run` must not
    // let that silently skip a subsidiary and all of its assets.
    let mut units = parse_org_units(tool_args);
    let requested_org_count = units.len();
    let mut scope_source = "tool_args".to_string();
    let mut auto_added_orgs: Vec<String> = Vec::new();
    let mut rejected_orgs: Vec<String> = Vec::new();

    // 2b. Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
    // confine AND complete the fan-out to the scoping-confirmed root org's
    // subtree (root + subsidiaries). Drop requested orgs outside it and add any
    // DB subtree orgs the model omitted.
    if let Some(root) = ctx.harness_org_id {
        if let Some(repo) = ctx.events.db_tracker.and_then(|t| t.repo()) {
            match repo.org_subtree_units(root).await {
                Ok(authoritative) if !authoritative.is_empty() => {
                    let before = units.len();
                    let authoritative = authoritative
                        .into_iter()
                        .map(org_unit_from_scope_unit)
                        .collect();
                    let (merged, added, rejected) =
                        merge_with_authoritative_subtree(units, authoritative);
                    units = merged;
                    auto_added_orgs = added;
                    rejected_orgs = rejected;
                    scope_source = "engagement_org_subtree".to_string();
                    if !rejected_orgs.is_empty() {
                        tracing::warn!(
                            target: "harness::stage_run",
                            root_org = %root,
                            rejected = ?rejected_orgs,
                            "stage_run dropped {}/{} requested org(s) outside the engagement org subtree",
                            rejected_orgs.len(),
                            before
                        );
                    }
                    if !auto_added_orgs.is_empty() {
                        tracing::info!(
                            target: "harness::stage_run",
                            root_org = %root,
                            requested_orgs = before,
                            total_orgs = units.len(),
                            auto_added = ?auto_added_orgs,
                            "stage_run filled missing requested org(s) from the engagement org subtree"
                        );
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "harness::stage_run",
                        root_org = %root,
                        error = %error,
                        "stage_run could not read engagement org subtree; falling back to requested orgs"
                    );
                }
            }
        }
    }
    if units.is_empty() {
        return Ok(ToolExecutionResult {
            value: json!({
                "error": "stage_run needs the in-scope organizations. Pass `orgs` as a non-empty \
                          array of { id, name, ownership_percent? } using the organization tree you \
                          built during scoping (manage_organizations).",
                "passed": false
            }),
            success: false,
        });
    }

    // 3. Serial fan-out: dispatch the specialist sub-agent once per org. Serial
    //    (not parallel) because sibling runs share this bridge's harness side-
    //    channels + conversation history; K-concurrency is a safe follow-up.
    let sub_agent_tool = format!("sub_agent_{specialist}");
    let mut gaps: Vec<Value> = Vec::new();
    let mut passed_count = 0usize;
    let resume_skip_not_before = active_stage_skip_floor(ctx, stage).await;
    if let Some(floor) = resume_skip_not_before {
        tracing::info!(
            target: "harness::stage_run",
            stage = %stage.as_str(),
            stage_started_at = %floor,
            "stage_run constrained resume-skip to completions from the current active stage"
        );
    }

    let mut resume_skips: HashMap<String, chrono::DateTime<chrono::Utc>> = HashMap::new();
    for unit in &units {
        if let Some(passed_at) =
            resume_skip_passed_at(ctx, stage, unit, resume_skip_not_before).await
        {
            resume_skips.insert(unit.id.clone(), passed_at);
        }
    }

    // Seed EVERY org up-front so the UI's covered/total denominator reflects the
    // FULL fan-out immediately. Resume-skipped rows are seeded as `passed`, not
    // `queued`: continuation/repair often passes only blocked orgs, then runtime
    // auto-fills the authoritative org subtree. Already-passed siblings must not
    // briefly look like queued work just because they appear after the current
    // blocked org in the serial loop.
    for unit in &units {
        let org_request_id = format!("{tool_id}::org::{}", unit.id);
        let passed_at = resume_skips.get(&unit.id);
        emit_org_progress(
            ctx,
            stage,
            unit,
            &org_request_id,
            if passed_at.is_some() {
                "passed"
            } else {
                "queued"
            },
            passed_at.map(|passed_at| {
                format!(
                    "已完成于 {} · 跳过重跑（{}d 内已通过本阶段）",
                    passed_at.format("%Y-%m-%d %H:%M UTC"),
                    STAGE_COMPLETION_TTL_SECS / 86_400
                )
            }),
            0,
            &stage_label,
            &role_label,
            &coverage_axis,
        );
    }

    for unit in &units {
        // Distinct per-org `parent_request_id` so each org's specialist sub-agent
        // is tracked independently in the UI (the frontend keys sub-agents by
        // `parent_request_id`; reusing the stage_run `tool_id` for every org would
        // collapse them into one). The UI links the org row to this id via the
        // `agent_request_id` field on the StageRunOrgProgress event.
        let org_request_id = format!("{tool_id}::org::{}", unit.id);
        let agent_path = stage_run_agent_path(stage, unit, &specialist);
        let passed_at = resume_skips.get(&unit.id).copied();
        let mut current_wave = if spec.asset_wave_barrier {
            match (passed_at, resume_skip_not_before) {
                (Some(_), _) => current_running_stage_asset_wave(ctx, stage, unit).await,
                (None, Some(started_at)) => {
                    prepare_stage_asset_wave(ctx, stage, unit, started_at).await
                }
                (None, None) => None,
            }
        } else {
            None
        };

        // Resume-skip: if this org already passed THIS stage within the TTL
        // window, count it covered and DON'T re-dispatch the specialist — the
        // fix for "为什么还带着已完成的 org 重新跑 / 很多操作重复做". In a
        // new operation/current active stage, old completions are not enough:
        // the current gate still needs evidence/source rows for this stage, so
        // only completions written after this stage started may skip.
        if let Some(passed_at) = passed_at {
            let legacy_wave_items_covered = match current_wave.as_ref() {
                Some(wave) if passed_at < wave.started_at => {
                    stage_asset_wave_items_covered_by_pass(ctx, wave, passed_at).await
                }
                _ => false,
            };
            if resume_skip_covers_current_wave(
                passed_at,
                current_wave.as_ref(),
                legacy_wave_items_covered,
            ) {
                if let Some(wave) = current_wave.take() {
                    if let Err(reason) = complete_stage_asset_wave(ctx, &wave).await {
                        emit_org_progress(
                            ctx,
                            stage,
                            unit,
                            &org_request_id,
                            "blocked",
                            Some(reason.clone()),
                            0,
                            &stage_label,
                            &role_label,
                            &coverage_axis,
                        );
                        gaps.push(json!({
                            "org_id": unit.id,
                            "org_name": unit.name,
                            "detail": reason
                        }));
                        continue;
                    }
                }
                clear_stage_run_agent_checkpoint(ctx, &agent_path).await;
                passed_count += 1;
                continue;
            }
        }

        // Phase 2 闸1·A-lite: run this org's dispatch→gate inside a bounded retry
        // loop. A BLOCK re-dispatches the SAME specialist with the gate's reasons as
        // feedback — the already-collected evidence stays in the ledger and the gate
        // reads it cumulatively, so a fresh re-run only needs to close the named
        // gaps. Only a PASS counts + writes the ledger; exhausting the attempts
        // records a gap for the main agent's gap-closure loop. The no-DB fallback
        // path uses max_attempts=1 so eval/headless never retries.
        let restored_retry = load_stage_run_agent_checkpoint(ctx, &agent_path)
            .await
            .and_then(|checkpoint| {
                pending_stage_run_retry_from_checkpoint(&checkpoint, MAX_ORG_GATE_ATTEMPTS)
            });
        let mut attempt = restored_retry
            .as_ref()
            .map(|(completed_attempt, _)| *completed_attempt)
            .unwrap_or(0);
        let mut feedback: Option<String> = restored_retry.map(|(_, feedback)| feedback);
        let mut resume_chain_id = load_stage_run_worker_chain(ctx, stage, unit, &specialist).await;
        loop {
            attempt += 1;
            emit_org_progress(
                ctx,
                stage,
                unit,
                &org_request_id,
                "running",
                Some(if attempt == 1 {
                    match resume_chain_id {
                        Some(chain_id) => {
                            format!("resuming {role_label} worker ({chain_id})")
                        }
                        None => format!("dispatching {role_label}"),
                    }
                } else {
                    format!("retry {attempt}/{MAX_ORG_GATE_ATTEMPTS}: closing gate gaps")
                }),
                0,
                &stage_label,
                &role_label,
                &coverage_axis,
            );

            let objective = {
                let base = build_org_objective(
                    stage,
                    unit,
                    &spec.expected_techniques,
                    &spec.allowed_tool_types,
                );
                let base = match current_wave.as_ref() {
                    Some(wave) => {
                        format!("{base}\n\n{}", stage_asset_wave_instruction(stage, wave))
                    }
                    None => base,
                };
                match &feedback {
                    Some(fb) => format!("{base}\n\n{fb}"),
                    None => base,
                }
            };
            let mut sub_args = json!({ "task": objective });
            if let Some(chain_id) = resume_chain_id {
                sub_args["resume"] = json!(chain_id.to_string());
            }
            let result = execute_sub_agent_call(
                &sub_agent_tool,
                &sub_args,
                ctx,
                model,
                context,
                &org_request_id,
            )
            .await;

            let sub_ok = matches!(&result, Ok(r) if r.success);
            let carried_submit_repair_mode = load_stage_run_agent_checkpoint(ctx, &agent_path)
                .await
                .and_then(|checkpoint| {
                    serde_json::from_value::<SubmitRepairMode>(checkpoint.submit_repair_mode?).ok()
                });
            if let Ok(result) = &result {
                if let Some(chain_id) = result
                    .value
                    .get("response")
                    .and_then(|v| v.as_str())
                    .and_then(parse_sub_agent_session_id)
                {
                    resume_chain_id = Some(chain_id);
                    persist_stage_run_worker_chain(
                        ctx,
                        stage,
                        unit,
                        &specialist,
                        &org_request_id,
                        chain_id,
                    )
                    .await;
                }
            }
            persist_stage_run_agent_checkpoint(
                ctx,
                build_stage_run_agent_checkpoint(StageRunCheckpointInput {
                    operation_id: stage_run_operation_id(ctx),
                    stage,
                    agent_path: &agent_path,
                    attempt,
                    org_request_id: &org_request_id,
                    sub_agent_tool: &sub_agent_tool,
                    chain_id: resume_chain_id,
                    status: AgentRunStatus::ToolCompleted,
                    pending_gate_correction: None,
                    correction_kind: None,
                    submit_repair_mode: carried_submit_repair_mode,
                    repair_directive: None,
                }),
            )
            .await;

            // Take THIS org's own deliverable: serial execution means the
            // side-channel slot currently holds this org's last submit.
            let org_deliverable: Option<StageDeliverable> =
                match ctx.harness_deliverable_sink.as_ref() {
                    Some(sink) => sink
                        .read()
                        .await
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<StageDeliverable>(s).ok()),
                    None => None,
                };
            // Clear after taking so the next attempt/org cannot reuse this residue.
            if let Some(sink) = ctx.harness_deliverable_sink.as_ref() {
                *sink.write().await = None;
            }

            // Authoritative verdict + whether it came from the real DB gate. Without
            // a repo (pure-eval/headless) or a parseable deliverable, fall back to
            // the sub-agent success flag so regression/eval paths keep working.
            let repo = ctx.events.db_tracker.and_then(|t| t.repo());
            let (verdict, from_gate) = match (repo, org_deliverable.as_ref()) {
                (Some(repo), Some(deliv)) => {
                    let org_uuid = uuid::Uuid::parse_str(&unit.id).ok();
                    let session = ctx.events.session_id.unwrap_or("");
                    let wave_cutoff = current_wave
                        .as_ref()
                        .map(|wave| wave.started_at)
                        .or(resume_skip_not_before);
                    let wave_assets = current_wave
                        .as_ref()
                        .map(|wave| wave.asset_values.as_slice());
                    let gate = evaluate_org_stage_gate(
                        repo,
                        org_uuid,
                        session,
                        stage,
                        deliv,
                        wave_cutoff,
                        wave_assets,
                    )
                    .await;
                    (decide_org_verdict(&gate), true)
                }
                (repo, None) => fallback_org_verdict(repo.is_some(), sub_ok),
                (None, Some(_)) => fallback_org_verdict(false, sub_ok),
            };

            // Only the real DB gate earns retries; the fallback path is terminal.
            let max_attempts = if from_gate { MAX_ORG_GATE_ATTEMPTS } else { 1 };
            match next_org_action(&verdict, attempt, max_attempts) {
                OrgAttemptOutcome::Passed => {
                    clear_stage_run_agent_checkpoint(ctx, &agent_path).await;
                    let mut passed_note = None;
                    if let Some(wave) = current_wave.take() {
                        if let Err(reason) = complete_stage_asset_wave(ctx, &wave).await {
                            emit_org_progress(
                                ctx,
                                stage,
                                unit,
                                &org_request_id,
                                "blocked",
                                Some(reason.clone()),
                                0,
                                &stage_label,
                                &role_label,
                                &coverage_axis,
                            );
                            gaps.push(json!({
                                "org_id": unit.id,
                                "org_name": unit.name,
                                "detail": reason
                            }));
                            break;
                        }
                        passed_note = Some(format!(
                            "asset batch #{} passed; newly discovered assets remain backlog for the global expansion pass",
                            wave.wave_index + 1
                        ));
                    }
                    passed_count += 1;
                    // Record this org's pass into the resume ledger so a later run
                    // can skip it (upsert latest). Fail-open: no db_tracker /
                    // unparseable id → just don't record (re-runs won't skip).
                    if let (Some(tracker), Ok(org_id)) =
                        (ctx.events.db_tracker, uuid::Uuid::parse_str(&unit.id))
                    {
                        tracker
                            .record_org_stage_completion(
                                org_id,
                                stage.as_str(),
                                Some(&org_request_id),
                            )
                            .await;
                    }
                    emit_org_progress(
                        ctx,
                        stage,
                        unit,
                        &org_request_id,
                        "passed",
                        passed_note,
                        0,
                        &stage_label,
                        &role_label,
                        &coverage_axis,
                    );
                    break;
                }
                OrgAttemptOutcome::Retry { feedback: fb } => {
                    let repair_directive = match &verdict {
                        OrgVerdict::Block {
                            reasons,
                            recovery_actions,
                        } => Some(stage_run_gate_repair_directive(
                            stage,
                            uuid::Uuid::parse_str(&unit.id).ok(),
                            agent_path.clone(),
                            reasons.clone(),
                            recovery_actions,
                        )),
                        OrgVerdict::Pass => None,
                    };
                    let submit_repair_mode = repair_directive
                        .as_ref()
                        .and_then(RepairDirective::to_submit_repair_mode)
                        .or_else(|| match &verdict {
                            OrgVerdict::Block { reasons, .. } => {
                                submit_coverage_gap_repair_mode_from_reasons(reasons)
                            }
                            OrgVerdict::Pass => None,
                        });
                    let next_feedback = repair_directive
                        .as_ref()
                        .map(|directive| format!("{fb}\n\n{}", directive.model_instruction()))
                        .unwrap_or(fb);
                    if let Some(directive) = repair_directive.as_ref() {
                        emit_stage_refiner_decision(ctx, stage, &agent_path, directive);
                    }
                    persist_stage_run_agent_checkpoint(
                        ctx,
                        build_stage_run_agent_checkpoint(StageRunCheckpointInput {
                            operation_id: stage_run_operation_id(ctx),
                            stage,
                            agent_path: &agent_path,
                            attempt,
                            org_request_id: &org_request_id,
                            sub_agent_tool: &sub_agent_tool,
                            chain_id: resume_chain_id,
                            status: AgentRunStatus::GateBlocked,
                            pending_gate_correction: Some(next_feedback.clone()),
                            correction_kind: Some("per_org_gate_retry"),
                            submit_repair_mode,
                            repair_directive,
                        }),
                    )
                    .await;
                    feedback = Some(next_feedback);
                    continue;
                }
                OrgAttemptOutcome::Exhausted { reasons } => {
                    clear_stage_run_agent_checkpoint(ctx, &agent_path).await;
                    // Prefer the gate's own reasons; fall back to the sub-agent's
                    // response/error when the block came from the success-flag path.
                    let detail = if reasons.is_empty() {
                        match &result {
                            Ok(r) => r
                                .value
                                .get("response")
                                .or_else(|| r.value.get("error"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.chars().take(300).collect::<String>())
                                .unwrap_or_default(),
                            Err(e) => e.to_string(),
                        }
                    } else {
                        reasons.join("; ").chars().take(300).collect::<String>()
                    };
                    emit_org_progress(
                        ctx,
                        stage,
                        unit,
                        &org_request_id,
                        "blocked",
                        None,
                        0,
                        &stage_label,
                        &role_label,
                        &coverage_axis,
                    );
                    gaps.push(
                        json!({ "org_id": unit.id, "org_name": unit.name, "detail": detail }),
                    );
                    break;
                }
            }
        }
    }

    // 4. Aggregate: engagement passes only when EVERY org passed (design §2).
    // For wave-aware stages, newly discovered assets are queued only after all
    // org seed batches close, so delta work is global rather than per-org
    // recursion. Queued delta batches intentionally withhold the close token.
    let mut expansion_batches = Vec::new();
    if gaps.is_empty() && spec.asset_wave_barrier {
        match queue_global_delta_asset_batches(ctx, stage, &units).await {
            Ok(queued) => expansion_batches = queued,
            Err(reason) => {
                gaps.push(json!({
                    "org_id": null,
                    "org_name": "global_delta_expansion",
                    "detail": reason
                }));
            }
        }
    }
    let passed = gaps.is_empty() && expansion_batches.is_empty();

    // Phase 1.5 阶段过门令牌：仅当本阶段**全 in-scope org**（不只本次 `units`——D11 只重跑
    // 缺口 org 的场景也要看累积账本是否齐）都已 fresh PASS 时，对账本回读值算一个确定性 hash
    // 令牌随返回带回主 agent；收尾 gate 拿同一张账本重算比对（B-recompute）。无 repo / 核不到
    // 全集 / 某 org 缺失或过期 → 不发令牌（收尾 gate 会 fail-closed 引导重跑）。
    let pass_token: Option<String> = if passed {
        match ctx.events.db_tracker.and_then(|t| t.repo()) {
            Some(repo) => {
                let engagement_subtree_ids = if let Some(root) = ctx.harness_org_id {
                    match repo.org_subtree_ids(root).await {
                        Ok(ids) if !ids.is_empty() => Some(ids),
                        Ok(_) => {
                            tracing::warn!(
                                target: "harness::stage_run",
                                root_org = %root,
                                "stage_run pass-token could not resolve engagement org subtree"
                            );
                            Some(vec![])
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: "harness::stage_run",
                                root_org = %root,
                                error = %error,
                                "stage_run pass-token org-subtree lookup failed"
                            );
                            Some(vec![])
                        }
                    }
                } else {
                    None
                };
                let legacy_org_ids = if ctx.harness_org_id.is_none() {
                    repo.in_scope_org_ids(None).await.unwrap_or_default()
                } else {
                    vec![]
                };
                let org_ids = fanout_completion_scope_ids(
                    ctx.harness_org_id,
                    engagement_subtree_ids,
                    legacy_org_ids,
                );
                if org_ids.is_empty() {
                    None
                } else {
                    let now = chrono::Utc::now();
                    let fresh: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = repo
                        .org_stage_completions_get(stage.as_str(), &org_ids)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(_, at)| {
                            completion_is_fresh_for_stage(
                                *at,
                                now,
                                STAGE_COMPLETION_TTL_SECS,
                                resume_skip_not_before,
                            )
                        })
                        .collect();
                    let have: std::collections::HashSet<uuid::Uuid> =
                        fresh.iter().map(|(o, _)| *o).collect();
                    if org_ids.iter().all(|o| have.contains(o)) {
                        Some(stage_pass_token(stage, &fresh))
                    } else {
                        None
                    }
                }
            }
            None => None,
        }
    } else {
        None
    };

    let mut summary = format!(
        "stage_run {}: {}/{} orgs passed{}",
        stage.as_str(),
        passed_count,
        units.len(),
        if gaps.is_empty() {
            String::new()
        } else {
            format!(
                " — {} blocked. Re-run stage_run with `orgs` set to only the blocked org(s) to close the gap.",
                gaps.len()
            )
        }
    );
    if let Some(token) = pass_token.as_deref() {
        summary.push_str(&format!(
            " — every in-scope org passed this stage's per-org gate. To CLOSE this stage, submit your StageDeliverable with a claim {{\"kind\":\"{}\",\"subject\":\"{}\",\"summary\":\"{}\"}}; the stage gate re-derives this pass_token from the DB ledger and BLOCKS without it.",
            STAGE_RUN_PASS_TOKEN_KIND,
            stage.as_str(),
            token
        ));
    }
    if !expansion_batches.is_empty() {
        let asset_count: usize = expansion_batches
            .iter()
            .map(|batch| batch.asset_count)
            .sum();
        summary.push_str(&format!(
            " — seed batches passed; queued global delta expansion for {} newly discovered asset(s) across {} org(s). Re-run stage_run to process this delta batch before closing the stage.",
            asset_count,
            expansion_batches.len()
        ));
    }
    if !auto_added_orgs.is_empty() {
        summary.push_str(&format!(
            " — auto-filled {} missing org(s) from the engagement tree",
            auto_added_orgs.len()
        ));
    }

    Ok(ToolExecutionResult {
        value: json!({
            "passed": passed,
            "stage": stage.as_str(),
            "specialist": specialist,
            "scope_source": scope_source,
            "requested_orgs": requested_org_count,
            "total_orgs": units.len(),
            "passed_orgs": passed_count,
            "auto_added_orgs": auto_added_orgs,
            "rejected_orgs": rejected_orgs,
            "gaps": gaps,
            "expansion_batches": expansion_batches.iter().map(|batch| json!({
                "org_id": batch.org_id.as_str(),
                "org_name": batch.org_name.as_str(),
                "wave_index": batch.wave_index,
                "asset_count": batch.asset_count,
            })).collect::<Vec<_>>(),
            "summary": summary,
            "pass_token": pass_token,
        }),
        success: true,
    })
}

/// The `stage_run` tool definition surfaced to the task-mode primary agent.
///
/// Not a registry tool (it is routed in the agentic loop), so its definition is
/// injected by `selection_apply` when `ToolSelection::include_stage_run` is set.
pub fn stage_run_tool_definition() -> rig::completion::ToolDefinition {
    rig::completion::ToolDefinition {
        name: "stage_run".to_string(),
        description: "Run the CURRENT engagement stage across every in-scope organization in \
                      parallel-of-effort: this fans the stage's specialist (e.g. Recon for \
                      target_intel) out, one run per org, each isolated and gated on its own \
                      evidence. Call this once you are inside a stage that has a per-org \
                      specialist, instead of dispatching sub_agent_* per org yourself. Pass the \
                      organization tree you built during scoping; when an engagement root is \
                      bound, the runtime expands this to the full DB-backed root subtree so \
                      continuation turns cannot omit subsidiaries. Returns { passed, gaps[] }: if \
                      not passed, call stage_run again; the runtime still checks the full bound \
                      engagement tree and resumes/skips already-passed orgs."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "orgs": {
                    "type": "array",
                    "description": "In-scope organizations to run the stage specialist against \
                                    (parent + subsidiaries). Each { id: organization_id uuid, \
                                    name, ownership_percent? }.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "ownership_percent": { "type": "number" }
                        },
                        "required": ["id", "name"]
                    }
                },
                "concurrency": {
                    "type": "integer",
                    "description": "Reserved for future K-parallel fan-out; currently runs serially."
                }
            },
            "required": ["orgs"]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::harness::org_gate::completion_is_fresh;
    use golish_agent_kit::harness::CoverageGapAction;

    #[test]
    fn parse_org_units_reads_id_name_ownership() {
        let args = json!({
            "orgs": [
                { "id": "11111111-1111-1111-1111-111111111111", "name": "平安科技", "ownership_percent": 100 },
                { "id": "22222222-2222-2222-2222-222222222222", "name": "子公司" }
            ]
        });
        let units = parse_org_units(&args);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].name, "平安科技");
        assert_eq!(units[0].ownership_percent, Some(100.0));
        assert_eq!(units[1].name, "子公司");
        assert_eq!(units[1].ownership_percent, None);
    }

    #[test]
    fn parse_org_units_skips_blank_ids_and_missing_orgs() {
        assert!(parse_org_units(&json!({})).is_empty());
        let args = json!({ "orgs": [ { "id": "  ", "name": "x" }, { "name": "no id" } ] });
        assert!(parse_org_units(&args).is_empty());
    }

    #[test]
    fn authoritative_subtree_fills_missing_requested_orgs() {
        let requested = vec![
            OrgUnit {
                id: "root".to_string(),
                name: "Root from model".to_string(),
                ownership_percent: None,
            },
            OrgUnit {
                id: "child-a".to_string(),
                name: "Child A from model".to_string(),
                ownership_percent: Some(100.0),
            },
            OrgUnit {
                id: "outside".to_string(),
                name: "Outside Org".to_string(),
                ownership_percent: None,
            },
        ];
        let authoritative = vec![
            OrgUnit {
                id: "root".to_string(),
                name: "Root".to_string(),
                ownership_percent: None,
            },
            OrgUnit {
                id: "child-a".to_string(),
                name: "Child A".to_string(),
                ownership_percent: None,
            },
            OrgUnit {
                id: "child-b".to_string(),
                name: "Child B".to_string(),
                ownership_percent: None,
            },
        ];

        let (merged, added, rejected) = merge_with_authoritative_subtree(requested, authoritative);

        assert_eq!(
            merged
                .iter()
                .map(|unit| unit.id.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "child-a", "child-b"]
        );
        assert_eq!(merged[1].ownership_percent, Some(100.0));
        assert_eq!(added, vec!["Child B"]);
        assert_eq!(rejected, vec!["Outside Org"]);
    }

    #[test]
    fn stage_label_and_role_label_title_case() {
        assert_eq!(stage_label_for(StageKind::TargetIntel), "Target Intel");
        assert_eq!(role_label_for("recon"), "Recon");
    }

    #[test]
    fn stage_asset_wave_instruction_pins_current_batch() {
        let wave = StageAssetWaveView {
            id: uuid::Uuid::from_u128(1),
            operation_id: uuid::Uuid::from_u128(2),
            organization_id: uuid::Uuid::from_u128(3),
            stage_kind: "external_attack_surface".to_string(),
            wave_index: 1,
            started_at: chrono::Utc::now(),
            asset_hash: "abc123".to_string(),
            asset_values: vec!["a.example.com".to_string(), "1.2.3.4".to_string()],
        };

        let instruction = stage_asset_wave_instruction(StageKind::ExternalAttackSurface, &wave);

        assert!(instruction.contains("wave #2"));
        assert!(instruction.contains("a.example.com"));
        assert!(instruction.contains("1.2.3.4"));
        assert!(instruction.contains("expansion backlog"));
        assert!(instruction.contains("global delta pass"));
    }

    #[test]
    fn build_org_objective_pins_org_id_and_scope() {
        let unit = OrgUnit {
            id: "abc".to_string(),
            name: "ACME".to_string(),
            ownership_percent: None,
        };
        // No techniques / tools → bare objective (back-compat shape, no contract).
        let obj = build_org_objective(StageKind::TargetIntel, &unit, &[], &[]);
        assert!(obj.contains("organization_id: abc"));
        assert!(obj.contains("THIS organization only"));
        assert!(obj.contains("target_intel"));
        assert!(!obj.contains("COVERAGE CONTRACT"));
    }

    #[test]
    fn build_org_objective_front_loads_coverage_contract_and_tools() {
        let unit = OrgUnit {
            id: "abc".to_string(),
            name: "ACME".to_string(),
            ownership_percent: None,
        };
        let techniques = vec![
            "GOLISH-INTEL-DNS".to_string(),
            "GOLISH-INTEL-WHOIS".to_string(),
        ];
        let tools = vec!["recon/dns".to_string(), "recon/subdomain".to_string()];
        let obj = build_org_objective(StageKind::TargetIntel, &unit, &techniques, &tools);
        // Coverage contract names the expected techniques + the gate consequence.
        assert!(obj.contains("COVERAGE CONTRACT"));
        assert!(obj.contains("GOLISH-INTEL-DNS"));
        assert!(obj.contains("GOLISH-INTEL-WHOIS"));
        assert!(obj.contains("FAILS the gate"));
        assert!(obj.contains("PRE-SUBMIT SELF-CHECK"));
        assert!(obj.contains("check_stage_asset_coverage"));
        assert!(obj.contains("stage=\"target_intel\""));
        assert!(obj.contains("organization_id=\"abc\""));
        assert!(obj.contains("ready_to_submit=false"));
        assert!(obj.contains("gap_examples"));
        assert!(obj.contains("next_action"));
        assert!(obj.contains("Only call submit_stage_deliverable after ready_to_submit=true"));
        // Tool boundary is listed so the specialist stays in-stage + background guidance.
        assert!(obj.contains("recon/dns"));
        assert!(obj.contains("submit_stage_deliverable waits"));
        assert!(obj.contains("completion notes"));
        assert!(obj.contains("do NOT re-run"));
    }

    #[test]
    fn build_eas_objective_blocks_broad_service_sweeps() {
        let unit = OrgUnit {
            id: "abc".to_string(),
            name: "ACME".to_string(),
            ownership_percent: None,
        };
        let obj = build_org_objective(
            StageKind::ExternalAttackSurface,
            &unit,
            &["GOLISH-EAS-SERVICE-FINGERPRINT".to_string()],
            &["nmap".to_string()],
        );

        assert!(obj.contains("EAS SCAN STRATEGY"));
        assert!(obj.contains("do not run broad `nmap -sV -iL`"));
        assert!(obj.contains("confirmed open host:port groups"));
        assert!(obj.contains("visible wait/check loop"));
        assert!(obj.contains("stdout/stderr is"));
        assert!(obj.contains("kill_job"));
    }

    #[test]
    fn tool_definition_requires_orgs() {
        let def = stage_run_tool_definition();
        assert_eq!(def.name, "stage_run");
        let required = def.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "orgs"));
    }

    #[test]
    fn completion_freshness_respects_ttl() {
        let now = chrono::Utc::now();
        let ttl = STAGE_COMPLETION_TTL_SECS;
        // Just now → fresh.
        assert!(completion_is_fresh(now, now, ttl));
        // 1 day ago under a 7d TTL → fresh (resume-skip applies).
        assert!(completion_is_fresh(
            now - chrono::Duration::days(1),
            now,
            ttl
        ));
        // Exactly at the TTL boundary → still fresh (<=).
        assert!(completion_is_fresh(
            now - chrono::Duration::seconds(ttl),
            now,
            ttl
        ));
        // 8 days ago under a 7d TTL → stale (re-test).
        assert!(!completion_is_fresh(
            now - chrono::Duration::days(8),
            now,
            ttl
        ));
        // Future timestamp (clock skew) → treated as fresh, never re-runs early.
        assert!(completion_is_fresh(
            now + chrono::Duration::hours(1),
            now,
            ttl
        ));
    }

    #[test]
    fn resume_skip_floor_blocks_prior_continuity_completion() {
        let now = chrono::Utc::now();
        let floor = now - chrono::Duration::minutes(10);

        assert!(
            !resume_skip_is_allowed(now - chrono::Duration::hours(1), now, Some(floor)),
            "a new active stage must not skip from old stage completions"
        );
        assert!(resume_skip_is_allowed(
            now - chrono::Duration::minutes(5),
            now,
            Some(floor)
        ));
        assert!(resume_skip_is_allowed(
            now - chrono::Duration::hours(1),
            now,
            None
        ));
    }

    #[test]
    fn resume_skip_covers_current_or_legacy_backfilled_wave() {
        let wave_started_at = chrono::Utc::now();
        let wave = StageAssetWaveView {
            id: uuid::Uuid::from_u128(1),
            operation_id: uuid::Uuid::from_u128(2),
            organization_id: uuid::Uuid::from_u128(3),
            stage_kind: "external_attack_surface".to_string(),
            wave_index: 0,
            started_at: wave_started_at,
            asset_hash: "abc123".to_string(),
            asset_values: vec!["a.example.com".to_string()],
        };

        assert!(resume_skip_covers_current_wave(
            wave_started_at + chrono::Duration::minutes(1),
            Some(&wave),
            false
        ));
        assert!(
            !resume_skip_covers_current_wave(
                wave_started_at - chrono::Duration::minutes(1),
                Some(&wave),
                false
            ),
            "a completion before the current wave must not suppress new work"
        );
        assert!(resume_skip_covers_current_wave(
            wave_started_at - chrono::Duration::minutes(1),
            Some(&wave),
            true
        ));
        assert!(resume_skip_covers_current_wave(
            wave_started_at,
            None,
            false
        ));
    }

    #[test]
    fn active_stage_skip_floor_uses_current_operation_stage() {
        let floor = chrono::Utc::now();
        let state = golish_agent_kit::db_traits::OperationStateView {
            operation_id: uuid::Uuid::new_v4(),
            profile: "assessment".to_string(),
            current_stage: "target_intel".to_string(),
            engagement_org_id: None,
            state_blob: json!({}),
            stage_started_at: floor,
        };

        assert_eq!(
            active_stage_skip_floor_from_state(&state, StageKind::TargetIntel),
            Some(floor)
        );
        assert_eq!(
            active_stage_skip_floor_from_state(&state, StageKind::ExternalAttackSurface),
            None
        );
    }

    #[test]
    fn parses_sub_agent_session_id_from_response_tail() {
        let id = uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let response = format!("done\n\n[sub_agent_session_id: {id}]");

        assert_eq!(parse_sub_agent_session_id(&response), Some(id));
        assert_eq!(parse_sub_agent_session_id("done"), None);
    }

    #[test]
    fn stage_run_worker_blob_round_trips_chain_and_preserves_graph_flow() {
        let chain_id = uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();
        let unit = OrgUnit {
            id: "11111111-1111-1111-1111-111111111111".to_string(),
            name: "ACME".to_string(),
            ownership_percent: None,
        };
        let existing = json!({
            "graph_flow": { "next_node": "external_attack_surface" }
        });

        let blob = upsert_stage_run_worker_blob(
            existing,
            StageKind::ExternalAttackSurface,
            &unit,
            "recon",
            "stage_run_1::org::11111111-1111-1111-1111-111111111111",
            chain_id,
        );

        assert_eq!(blob["graph_flow"]["next_node"], "external_attack_surface");
        assert_eq!(
            stage_run_worker_chain_from_blob(
                &blob,
                StageKind::ExternalAttackSurface,
                &unit.id,
                "recon"
            ),
            Some(chain_id)
        );
        assert_eq!(
            stage_run_worker_chain_from_blob(
                &blob,
                StageKind::ExternalAttackSurface,
                &unit.id,
                "crawler"
            ),
            None,
            "a different specialist must not resume another worker's chain"
        );
    }

    #[test]
    fn stage_run_agent_path_is_stable_per_stage_org_and_specialist() {
        let unit = OrgUnit {
            id: "11111111-1111-1111-1111-111111111111".to_string(),
            name: "ACME".to_string(),
            ownership_percent: None,
        };

        assert_eq!(
            stage_run_agent_path(StageKind::ExternalAttackSurface, &unit, "prober"),
            "main>stage_run:external_attack_surface>org:11111111-1111-1111-1111-111111111111>prober"
        );
    }

    #[test]
    fn pending_retry_restores_completed_attempt_and_feedback_from_checkpoint() {
        let checkpoint = AgentRunCheckpoint {
            schema_v: 1,
            operation_id: None,
            stage: Some(StageKind::ExternalAttackSurface.as_str().to_string()),
            stage_attempt_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            status: AgentRunStatus::GateBlocked,
            llm_turn_index: Some(1),
            message_chain_ref: Some("22222222-2222-2222-2222-222222222222".to_string()),
            pending_gate_correction: Some("retry 2/3: close liveness gap".to_string()),
            pending_submit_only: false,
            submit_repair_mode: None,
            repair_directive: None,
            runtime_corrections: Vec::new(),
            background_job_ids: Vec::new(),
            evidence_watermark: None,
            last_tool: None,
            updated_at: chrono::Utc::now(),
        };

        assert_eq!(
            pending_stage_run_retry_from_checkpoint(&checkpoint, 3),
            Some((1, "retry 2/3: close liveness gap".to_string()))
        );
    }

    #[test]
    fn pending_retry_ignores_non_gate_blocked_or_exhausted_checkpoint() {
        let mut checkpoint = AgentRunCheckpoint {
            schema_v: 1,
            operation_id: None,
            stage: Some(StageKind::ExternalAttackSurface.as_str().to_string()),
            stage_attempt_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            status: AgentRunStatus::ToolCompleted,
            llm_turn_index: Some(1),
            message_chain_ref: None,
            pending_gate_correction: Some("retry".to_string()),
            pending_submit_only: false,
            submit_repair_mode: None,
            repair_directive: None,
            runtime_corrections: Vec::new(),
            background_job_ids: Vec::new(),
            evidence_watermark: None,
            last_tool: None,
            updated_at: chrono::Utc::now(),
        };
        assert_eq!(
            pending_stage_run_retry_from_checkpoint(&checkpoint, 3),
            None
        );

        checkpoint.status = AgentRunStatus::GateBlocked;
        checkpoint.llm_turn_index = Some(3);
        assert_eq!(
            pending_stage_run_retry_from_checkpoint(&checkpoint, 3),
            None
        );
    }

    #[test]
    fn stage_run_agent_checkpoint_records_pending_gate_feedback() {
        let checkpoint = build_stage_run_agent_checkpoint(StageRunCheckpointInput {
            operation_id: None,
            stage: StageKind::ExternalAttackSurface,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober",
            attempt: 1,
            org_request_id: "stage_run_1::org::abc",
            sub_agent_tool: "sub_agent_prober",
            chain_id: Some(uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()),
            status: AgentRunStatus::GateBlocked,
            pending_gate_correction: Some("retry 2/3: close port gap".to_string()),
            correction_kind: Some("per_org_gate_retry"),
            submit_repair_mode: None,
            repair_directive: None,
        });

        assert_eq!(checkpoint.status, AgentRunStatus::GateBlocked);
        assert_eq!(checkpoint.llm_turn_index, Some(1));
        assert_eq!(
            checkpoint.pending_gate_correction.as_deref(),
            Some("retry 2/3: close port gap")
        );
        assert_eq!(checkpoint.runtime_corrections[0].kind, "per_org_gate_retry");
        assert_eq!(
            checkpoint.last_tool.as_ref().unwrap().result_ref.as_deref(),
            Some("message_chain:22222222-2222-2222-2222-222222222222")
        );
    }

    #[test]
    fn stage_run_agent_checkpoint_carries_coverage_repair_mode() {
        let mode = submit_coverage_gap_repair_mode_from_reasons(&[
            "coverage cell missing for 1.2.3.4 x GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
        ])
        .expect("coverage feedback should map to repair mode");
        let checkpoint = build_stage_run_agent_checkpoint(StageRunCheckpointInput {
            operation_id: None,
            stage: StageKind::ExternalAttackSurface,
            agent_path: "main>stage_run:external_attack_surface>org:abc>prober",
            attempt: 1,
            org_request_id: "stage_run_1::org::abc",
            sub_agent_tool: "sub_agent_prober",
            chain_id: None,
            status: AgentRunStatus::GateBlocked,
            pending_gate_correction: Some("retry 2/3: close coverage gap".to_string()),
            correction_kind: Some("per_org_gate_retry"),
            submit_repair_mode: Some(mode),
            repair_directive: None,
        });

        let restored: SubmitRepairMode =
            serde_json::from_value(checkpoint.submit_repair_mode.unwrap()).unwrap();
        assert_eq!(
            restored.kind,
            golish_sub_agents::SubmitRepairKind::CoverageGap
        );
        assert!(restored.block_result("pentest_run").is_none());
        assert!(restored
            .model_instruction()
            .contains("Targeted gap-closure"));
    }

    #[test]
    fn next_org_action_pass_is_passed() {
        assert_eq!(
            next_org_action(&OrgVerdict::Pass, 1, 3),
            OrgAttemptOutcome::Passed
        );
        assert_eq!(
            next_org_action(&OrgVerdict::Pass, 3, 3),
            OrgAttemptOutcome::Passed
        );
    }

    #[test]
    fn next_org_action_block_with_attempts_left_retries_with_named_reasons() {
        let v = OrgVerdict::Block {
            reasons: vec!["missing GOLISH-INTEL-DNS on a.com".to_string()],
            recovery_actions: HarnessRecoveryActions::default(),
        };
        match next_org_action(&v, 1, 3) {
            OrgAttemptOutcome::Retry { feedback } => {
                assert!(
                    feedback.contains("missing GOLISH-INTEL-DNS on a.com"),
                    "feedback names the gap: {feedback}"
                );
                assert!(
                    feedback.contains("retry 2/3"),
                    "feedback names attempt: {feedback}"
                );
            }
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn next_org_action_block_on_last_attempt_is_exhausted() {
        let v = OrgVerdict::Block {
            reasons: vec!["coverage incomplete".to_string()],
            recovery_actions: HarnessRecoveryActions::default(),
        };
        assert_eq!(
            next_org_action(&v, 3, 3),
            OrgAttemptOutcome::Exhausted {
                reasons: vec!["coverage incomplete".to_string()]
            }
        );
    }

    #[test]
    fn next_org_action_no_db_fallback_does_not_retry() {
        // max_attempts == 1 (no-repo fallback path): a BLOCK is terminal, never retried.
        let v = OrgVerdict::Block {
            reasons: vec!["sub-agent did not complete".to_string()],
            recovery_actions: HarnessRecoveryActions::default(),
        };
        assert_eq!(
            next_org_action(&v, 1, 1),
            OrgAttemptOutcome::Exhausted {
                reasons: vec!["sub-agent did not complete".to_string()]
            }
        );
    }

    #[test]
    fn live_stage_run_blocks_missing_deliverable_even_if_sub_agent_completed() {
        let (verdict, from_gate) = fallback_org_verdict(true, true);

        assert!(
            from_gate,
            "a live DB-backed stage must treat missing deliverable as a gate BLOCK so it retries"
        );
        match verdict {
            OrgVerdict::Block { reasons, .. } => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("without a StageDeliverable")),
                    "reason should explain that no accepted deliverable was captured: {reasons:?}"
                );
            }
            OrgVerdict::Pass => {
                panic!("missing live deliverable must not pass via sub_ok fallback")
            }
        }
    }

    #[test]
    fn stage_run_gate_repair_directive_uses_structured_gap_actions() {
        let recovery_actions = HarnessRecoveryActions {
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "pinganstock.com".to_string(),
                technique: "GOLISH-EAS-LIVENESS".to_string(),
                reason: "liveness cell never reached a terminal state".to_string(),
                suggested_tools: vec!["httpx".to_string()],
            }],
            ..Default::default()
        };
        let directive = stage_run_gate_repair_directive(
            StageKind::ExternalAttackSurface,
            None,
            "main>stage_run:external_attack_surface>org:abc>prober".to_string(),
            vec!["coverage incomplete".to_string()],
            &recovery_actions,
        );

        assert_eq!(directive.actions.len(), 1);
        assert_eq!(
            directive.actions[0].asset.as_deref(),
            Some("pinganstock.com")
        );
        assert_eq!(directive.actions[0].tool.as_deref(), Some("httpx"));
        assert_eq!(
            directive.submit_guidance.required_coverage_cells[0].technique,
            "GOLISH-EAS-LIVENESS"
        );
        let mode = directive
            .to_submit_repair_mode()
            .expect("coverage directive should become submit repair mode");
        assert_eq!(mode.coverage_gap_actions.len(), 1);
        assert_eq!(
            mode.coverage_gap_actions[0].asset,
            "pinganstock.com".to_string()
        );
    }

    #[test]
    fn no_db_fallback_still_uses_sub_agent_success() {
        let (verdict, from_gate) = fallback_org_verdict(false, true);

        assert!(!from_gate);
        assert!(matches!(verdict, OrgVerdict::Pass));
    }
}
