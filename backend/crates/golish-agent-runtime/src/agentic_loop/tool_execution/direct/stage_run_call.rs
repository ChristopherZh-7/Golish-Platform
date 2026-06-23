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

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::{json, Value};

use golish_agent_kit::harness::org_gate::{
    completion_is_fresh, decide_org_verdict, stage_pass_token, STAGE_COMPLETION_TTL_SECS,
    STAGE_RUN_PASS_TOKEN_KIND,
};
use golish_agent_kit::harness::{
    allowed_tool_names, evaluate_org_stage_gate, load_embedded_stage_spec, stage_methodology_md,
    OrgVerdict, StageDeliverable, StageKind,
};
use golish_core::events::{AiEvent, HarnessTraceKind};
use golish_sub_agents::SubAgentContext;

use super::super::super::{AgenticLoopContext, ToolExecutionResult};
use super::sub_agent_call::execute_sub_agent_call;

/// One per-org unit the fan-out runs the stage specialist against.
struct OrgUnit {
    id: String,
    name: String,
    ownership_percent: Option<f64>,
}

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
             timeout: poll with check_job, never re-run the same command.",
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
) -> Option<chrono::DateTime<chrono::Utc>> {
    let tracker = ctx.events.db_tracker?;
    let org_id = uuid::Uuid::parse_str(&unit.id).ok()?;
    let passed_at = tracker
        .recent_org_stage_completion(org_id, stage.as_str())
        .await?;
    completion_is_fresh(passed_at, chrono::Utc::now(), STAGE_COMPLETION_TTL_SECS)
        .then_some(passed_at)
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
        OrgVerdict::Block { reasons } => {
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

    // 2. Per-org units (from the engagement org tree the agent built in scoping).
    let mut units = parse_org_units(tool_args);
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

    // 2b. Engagement-org isolation (设计 2026-06-15-engagement-org-isolation):
    // confine the fan-out to the scoping-confirmed root org's subtree (root +
    // subsidiaries). Drop any requested org outside it — a sibling engagement's
    // org left in the same workspace (the "测 example 串成平安" bug) must never
    // get a specialist dispatched against it. Enforced only when a root org is
    // bound AND its subtree is readable; otherwise fail-open to legacy behavior
    // (test doubles / no-DB return an empty subtree → no confinement).
    if let Some(root) = ctx.harness_org_id {
        if let Some(repo) = ctx.events.db_tracker.and_then(|t| t.repo()) {
            let allowed: std::collections::HashSet<uuid::Uuid> = repo
                .org_subtree_ids(root)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect();
            if !allowed.is_empty() {
                let before = units.len();
                let mut rejected: Vec<String> = Vec::new();
                units.retain(|u| match uuid::Uuid::parse_str(&u.id) {
                    Ok(id) if allowed.contains(&id) => true,
                    _ => {
                        rejected.push(u.name.clone());
                        false
                    }
                });
                if !rejected.is_empty() {
                    tracing::warn!(
                        target: "harness::stage_run",
                        root_org = %root,
                        rejected = ?rejected,
                        "stage_run dropped {}/{} org(s) outside the engagement org subtree",
                        rejected.len(),
                        before
                    );
                }
                if units.is_empty() {
                    return Ok(ToolExecutionResult {
                        value: json!({
                            "error": format!(
                                "None of the requested organizations are within this engagement's \
                                 scope (root org {root} plus its subsidiaries). Rejected: {rejected:?}. \
                                 Pass only orgs from THIS engagement's scoping-confirmed org tree."
                            ),
                            "passed": false
                        }),
                        success: false,
                    });
                }
            }
        }
    }

    // 3. Serial fan-out: dispatch the specialist sub-agent once per org. Serial
    //    (not parallel) because sibling runs share this bridge's harness side-
    //    channels + conversation history; K-concurrency is a safe follow-up.
    let sub_agent_tool = format!("sub_agent_{specialist}");
    let mut gaps: Vec<Value> = Vec::new();
    let mut passed_count = 0usize;

    // Seed EVERY org as a queued row up-front so the UI's covered/total denominator
    // reflects the FULL fan-out immediately. Without this, serial execution emits
    // one row at a time, so the count visibly grows from "0/1" instead of showing
    // "0/N" — exactly the "怎么就记录了这么点" the user saw. Each org flips to
    // running → passed/blocked as the serial loop reaches it (merged by org id).
    for unit in &units {
        let org_request_id = format!("{tool_id}::org::{}", unit.id);
        emit_org_progress(
            ctx,
            stage,
            unit,
            &org_request_id,
            "queued",
            None,
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

        // Resume-skip: if this org already passed THIS stage within the TTL
        // window, count it covered and DON'T re-dispatch the specialist — the
        // fix for "为什么还带着已完成的 org 重新跑 / 很多操作重复做". Fail-open
        // (no db_tracker / unparseable id / stale row → run; see helper).
        if let Some(passed_at) = resume_skip_passed_at(ctx, stage, unit).await {
            passed_count += 1;
            emit_org_progress(
                ctx,
                stage,
                unit,
                &org_request_id,
                "passed",
                Some(format!(
                    "已完成于 {} · 跳过重跑（{}d 内已通过本阶段）",
                    passed_at.format("%Y-%m-%d %H:%M UTC"),
                    STAGE_COMPLETION_TTL_SECS / 86_400
                )),
                0,
                &stage_label,
                &role_label,
                &coverage_axis,
            );
            continue;
        }

        // Phase 2 闸1·A-lite: run this org's dispatch→gate inside a bounded retry
        // loop. A BLOCK re-dispatches the SAME specialist with the gate's reasons as
        // feedback — the already-collected evidence stays in the ledger and the gate
        // reads it cumulatively, so a fresh re-run only needs to close the named
        // gaps. Only a PASS counts + writes the ledger; exhausting the attempts
        // records a gap for the main agent's gap-closure loop. The no-DB fallback
        // path uses max_attempts=1 so eval/headless never retries.
        let mut attempt = 0usize;
        let mut feedback: Option<String> = None;
        loop {
            attempt += 1;
            emit_org_progress(
                ctx,
                stage,
                unit,
                &org_request_id,
                "running",
                Some(if attempt == 1 {
                    format!("dispatching {role_label}")
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
                match &feedback {
                    Some(fb) => format!("{base}\n\n{fb}"),
                    None => base,
                }
            };
            let sub_args = json!({ "task": objective });
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
            let (verdict, from_gate) = match (
                ctx.events.db_tracker.and_then(|t| t.repo()),
                org_deliverable.as_ref(),
            ) {
                (Some(repo), Some(deliv)) => {
                    let org_uuid = uuid::Uuid::parse_str(&unit.id).ok();
                    let session = ctx.events.session_id.unwrap_or("");
                    let gate = evaluate_org_stage_gate(repo, org_uuid, session, stage, deliv).await;
                    (decide_org_verdict(&gate), true)
                }
                _ => {
                    let v = if sub_ok {
                        OrgVerdict::Pass
                    } else {
                        OrgVerdict::Block {
                            reasons: vec!["sub-agent did not complete".to_string()],
                        }
                    };
                    (v, false)
                }
            };

            // Only the real DB gate earns retries; the fallback path is terminal.
            let max_attempts = if from_gate { MAX_ORG_GATE_ATTEMPTS } else { 1 };
            match next_org_action(&verdict, attempt, max_attempts) {
                OrgAttemptOutcome::Passed => {
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
                        None,
                        0,
                        &stage_label,
                        &role_label,
                        &coverage_axis,
                    );
                    break;
                }
                OrgAttemptOutcome::Retry { feedback: fb } => {
                    feedback = Some(fb);
                    continue;
                }
                OrgAttemptOutcome::Exhausted { reasons } => {
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
    let passed = gaps.is_empty();

    // Phase 1.5 阶段过门令牌：仅当本阶段**全 in-scope org**（不只本次 `units`——D11 只重跑
    // 缺口 org 的场景也要看累积账本是否齐）都已 fresh PASS 时，对账本回读值算一个确定性 hash
    // 令牌随返回带回主 agent；收尾 gate 拿同一张账本重算比对（B-recompute）。无 repo / 核不到
    // 全集 / 某 org 缺失或过期 → 不发令牌（收尾 gate 会 fail-closed 引导重跑）。
    let pass_token: Option<String> = if passed {
        match ctx.events.db_tracker.and_then(|t| t.repo()) {
            Some(repo) => {
                let org_ids = repo.in_scope_org_ids(None).await.unwrap_or_default();
                if org_ids.is_empty() {
                    None
                } else {
                    let now = chrono::Utc::now();
                    let fresh: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> = repo
                        .org_stage_completions_get(stage.as_str(), &org_ids)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(_, at)| completion_is_fresh(*at, now, STAGE_COMPLETION_TTL_SECS))
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
        if passed {
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

    Ok(ToolExecutionResult {
        value: json!({
            "passed": passed,
            "stage": stage.as_str(),
            "specialist": specialist,
            "total_orgs": units.len(),
            "passed_orgs": passed_count,
            "gaps": gaps,
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
                      organization tree you built during scoping. Returns { passed, gaps[] }: if \
                      not passed, call stage_run again with `orgs` set to ONLY the blocked org(s) \
                      to close the gap."
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
    fn stage_label_and_role_label_title_case() {
        assert_eq!(stage_label_for(StageKind::TargetIntel), "Target Intel");
        assert_eq!(role_label_for("recon"), "Recon");
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
        // Tool boundary is listed so the specialist stays in-stage + poll guidance.
        assert!(obj.contains("recon/dns"));
        assert!(obj.contains("check_job"));
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
        };
        assert_eq!(
            next_org_action(&v, 1, 1),
            OrgAttemptOutcome::Exhausted {
                reasons: vec!["sub-agent did not complete".to_string()]
            }
        );
    }
}
