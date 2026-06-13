//! `execute_stage_run` — the `stage_run` tool handler.
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

use golish_agent_kit::harness::{load_embedded_stage_spec, StageKind};
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
fn build_org_objective(stage: StageKind, unit: &OrgUnit) -> String {
    format!(
        "Run the {} stage for this engagement. Organization: {} (organization_id: {}). \
         Collect for THIS organization only — discover its own assets and register them as \
         in-scope targets bound to this organization_id, then submit the stage deliverable.",
        stage.as_str(),
        unit.name,
        unit.id,
    )
}

/// Emit a [`HarnessTraceKind::StageRunOrgProgress`] for one org row.
#[allow(clippy::too_many_arguments)]
fn emit_org_progress(
    ctx: &AgenticLoopContext<'_>,
    stage: StageKind,
    unit: &OrgUnit,
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
    let units = parse_org_units(tool_args);
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

    for unit in &units {
        emit_org_progress(
            ctx,
            stage,
            unit,
            "running",
            Some(format!("dispatching {role_label}")),
            0,
            &stage_label,
            &role_label,
            &coverage_axis,
        );

        let objective = build_org_objective(stage, unit);
        let sub_args = json!({ "task": objective });
        let result =
            execute_sub_agent_call(&sub_agent_tool, &sub_args, ctx, model, context, tool_id).await;

        let ok = matches!(&result, Ok(r) if r.success);
        if ok {
            passed_count += 1;
            emit_org_progress(
                ctx,
                stage,
                unit,
                "passed",
                None,
                0,
                &stage_label,
                &role_label,
                &coverage_axis,
            );
        } else {
            let detail = match &result {
                Ok(r) => r
                    .value
                    .get("response")
                    .or_else(|| r.value.get("error"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().take(300).collect::<String>())
                    .unwrap_or_default(),
                Err(e) => e.to_string(),
            };
            emit_org_progress(
                ctx,
                stage,
                unit,
                "blocked",
                None,
                0,
                &stage_label,
                &role_label,
                &coverage_axis,
            );
            gaps.push(json!({ "org_id": unit.id, "org_name": unit.name, "detail": detail }));
        }
    }

    // 4. Aggregate: engagement passes only when EVERY org passed (design §2).
    let passed = gaps.is_empty();
    let summary = format!(
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

    Ok(ToolExecutionResult {
        value: json!({
            "passed": passed,
            "stage": stage.as_str(),
            "specialist": specialist,
            "total_orgs": units.len(),
            "passed_orgs": passed_count,
            "gaps": gaps,
            "summary": summary,
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
        let obj = build_org_objective(StageKind::TargetIntel, &unit);
        assert!(obj.contains("organization_id: abc"));
        assert!(obj.contains("THIS organization only"));
        assert!(obj.contains("target_intel"));
    }

    #[test]
    fn tool_definition_requires_orgs() {
        let def = stage_run_tool_definition();
        assert_eq!(def.name, "stage_run");
        let required = def.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "orgs"));
    }
}
