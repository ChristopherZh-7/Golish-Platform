//! Runtime strategy supervision for in-run agent drift.
//!
//! The deterministic gate remains the only PASS/BLOCK authority and
//! StageRefiner remains the post-gate repair owner. RuntimeSupervisor only
//! shapes the next action while the agent is still running.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::harness::{allowed_tool_names, load_embedded_stage_spec, StageKind};

const HARD_SUPERVISOR_MARKER: &str = "--- EXECUTION SUPERVISOR (HARD) ---";

const COMMON_STAGE_TOOLS: &[&str] = &[
    "query_target_data",
    "check_stage_asset_coverage",
    "wait_for_background_jobs",
    "check_job",
    "kill_job",
    "submit_stage_deliverable",
];

const BROAD_DISCOVERY_TOOLS: &[&str] = &[
    "list_in_scope_targets",
    "list_attack_surface_seeds",
    "manage_targets",
    "manage_organizations",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    Continue,
    StrategyPivot,
    WaitForBackground,
    ConsolidateAndSubmit,
    StopAndReport,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_hint: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyDirective {
    pub schema_v: u32,
    pub strategy_kind: StrategyKind,
    pub trigger: String,
    pub root_cause: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<StrategyAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_tools: Vec<String>,
    pub submit_after_actions: bool,
    pub directive_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_model_preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSupervisorContext {
    pub stage: Option<StageKind>,
    pub agent_path: String,
    pub agent_role: String,
    pub task: String,
    pub trigger: String,
    pub repeated_tool: String,
    pub repeat_count: usize,
    pub recent_calls: String,
    pub last_tool_name: String,
    pub last_tool_result: String,
    pub visible_tools: Vec<String>,
    pub active_repair_directive: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelDirective {
    #[serde(default)]
    strategy_kind: Option<StrategyKind>,
    #[serde(default)]
    root_cause: Option<String>,
    #[serde(default)]
    actions: Vec<ModelAction>,
    #[serde(default)]
    submit_after_actions: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ModelAction {
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    command_hint: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

pub fn runtime_supervisor_system_prompt() -> &'static str {
    r#"You are Golish RuntimeSupervisor, a stage-aware execution supervisor for an autonomous penetration-testing agent.

You are invoked only after the runtime detects repeated failed/stalled tool results. Your job is to choose the next narrow strategy without violating the current stage boundary.

Rules:
- Return STRICT JSON only. No markdown, no prose outside JSON.
- Do not invent evidence ids or claim the stage passed.
- Do not recommend tools outside the visible/stage-allowed tool list.
- If an active RepairDirective is present, it is higher priority than your strategy.
- Prefer wait/read/consolidate/submit over launching duplicate scans when recent output exists.
- For external_attack_surface coverage repair, avoid broad rediscovery and prefer exact asset/technique actions.

JSON schema:
{
  "strategy_kind": "continue|strategy_pivot|wait_for_background|consolidate_and_submit|stop_and_report|generic",
  "root_cause": "short reason",
  "actions": [
    {"tool": "visible tool name", "command_hint": "optional exact command or args", "reason": "why this action helps"}
  ],
  "submit_after_actions": true
}"#
}

pub fn runtime_supervisor_user_prompt(ctx: &RuntimeSupervisorContext) -> String {
    let stage = ctx
        .stage
        .map(|stage| stage.as_str().to_string())
        .unwrap_or_else(|| "(none)".to_string());
    let stage_spec = ctx
        .stage
        .and_then(|stage| load_embedded_stage_spec(stage).ok());
    let allowed_types = stage_spec
        .as_ref()
        .map(|spec| spec.allowed_tool_types.join(", "))
        .unwrap_or_else(|| "(not provided)".to_string());
    let stage_allowed = stage_spec
        .as_ref()
        .map(|spec| {
            allowed_tool_names(&spec.allowed_tool_types)
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "(not provided)".to_string());
    let repair = ctx.active_repair_directive.as_deref().unwrap_or("(none)");

    format!(
        r#"Current stage: {stage}
Agent path: {agent_path}
Agent role: {agent_role}
Task: {task}
Trigger: {trigger}
Failed pattern: {repeated_tool} x{repeat_count}

Visible tools:
{visible_tools}

Stage allowed tool types:
{allowed_types}

Stage canonical allowed tools:
{stage_allowed}

Active RepairDirective:
{repair}

Recent tool calls and outcomes:
{recent_calls}

Most recent tool:
{last_tool_name}

Most recent result:
{last_tool_result}

Return the next narrow strategy as strict JSON."#,
        stage = stage,
        agent_path = ctx.agent_path,
        agent_role = ctx.agent_role,
        task = safe_preview(&ctx.task, 1200),
        trigger = ctx.trigger,
        repeated_tool = ctx.repeated_tool,
        repeat_count = ctx.repeat_count,
        visible_tools = join_limited(&ctx.visible_tools, 80),
        allowed_types = allowed_types,
        stage_allowed = stage_allowed,
        repair = safe_preview(repair, 1200),
        recent_calls = safe_preview(&ctx.recent_calls, 3000),
        last_tool_name = ctx.last_tool_name,
        last_tool_result = safe_preview(&ctx.last_tool_result, 3500),
    )
}

pub fn directive_from_model_response(
    ctx: &RuntimeSupervisorContext,
    model_response: Option<&str>,
) -> StrategyDirective {
    let parsed = model_response.and_then(parse_model_directive);
    let raw_model_preview = model_response
        .map(|s| safe_preview(s, 500))
        .filter(|s| !s.trim().is_empty());
    let directive = match parsed {
        Some(model) => directive_from_model(ctx, model, raw_model_preview),
        None => fallback_directive(ctx, raw_model_preview),
    };
    sanitize_directive(ctx, directive)
}

impl StrategyDirective {
    pub fn model_instruction(&self, hard: bool) -> String {
        let mut out = String::new();
        if hard {
            out.push_str(HARD_SUPERVISOR_MARKER);
            out.push('\n');
        }
        out.push_str("--- RUNTIME SUPERVISOR DIRECTIVE ---\n");
        out.push_str(&format!(
            "Strategy: {:?}. Cause: {}\n",
            self.strategy_kind, self.root_cause
        ));
        if !self.allowed_tools.is_empty() {
            out.push_str(&format!(
                "Allowed next tools: [{}].\n",
                self.allowed_tools.join(", ")
            ));
        }
        if !self.forbidden_tools.is_empty() {
            out.push_str(&format!(
                "Forbidden for this correction: [{}].\n",
                self.forbidden_tools.join(", ")
            ));
        }
        if self.actions.is_empty() {
            out.push_str("Next: stop repeating the previous action, inspect current evidence/output, then choose the narrowest stage-allowed action.\n");
        } else {
            out.push_str("Next actions:\n");
            for (idx, action) in self.actions.iter().enumerate() {
                out.push_str(&format!("{}. {}", idx + 1, action.reason));
                if let Some(tool) = action.tool.as_deref() {
                    out.push_str(&format!(" tool={tool}"));
                }
                if let Some(hint) = action.command_hint.as_deref() {
                    out.push_str(&format!(" hint={hint}"));
                }
                out.push('\n');
            }
        }
        if self.submit_after_actions {
            out.push_str("After these exact actions, call submit_stage_deliverable once with real evidence ids and terminal coverage states.\n");
        }
        out.push_str("--------------------------");
        out
    }

    pub fn strategy_kind_label(&self) -> &'static str {
        match self.strategy_kind {
            StrategyKind::Continue => "continue",
            StrategyKind::StrategyPivot => "strategy_pivot",
            StrategyKind::WaitForBackground => "wait_for_background",
            StrategyKind::ConsolidateAndSubmit => "consolidate_and_submit",
            StrategyKind::StopAndReport => "stop_and_report",
            StrategyKind::Generic => "generic",
        }
    }
}

fn directive_from_model(
    ctx: &RuntimeSupervisorContext,
    model: ModelDirective,
    raw_model_preview: Option<String>,
) -> StrategyDirective {
    let root_cause = model
        .root_cause
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback_root_cause(ctx));
    let actions = model
        .actions
        .into_iter()
        .filter_map(|a| {
            let reason = a.reason.unwrap_or_default();
            if reason.trim().is_empty() && a.tool.is_none() && a.command_hint.is_none() {
                return None;
            }
            Some(StrategyAction {
                tool: a.tool.map(clean_tool_name).filter(|s| !s.is_empty()),
                command_hint: a.command_hint.map(|s| safe_preview(&s, 300)),
                reason: if reason.trim().is_empty() {
                    "Use this narrow stage-allowed action to make measurable progress.".to_string()
                } else {
                    safe_preview(&reason, 240)
                },
            })
        })
        .collect();
    directive_with_hash(StrategyDirective {
        schema_v: 1,
        strategy_kind: model.strategy_kind.unwrap_or(StrategyKind::Generic),
        trigger: ctx.trigger.clone(),
        root_cause,
        actions,
        allowed_tools: Vec::new(),
        forbidden_tools: Vec::new(),
        submit_after_actions: model.submit_after_actions.unwrap_or(false),
        directive_hash: String::new(),
        raw_model_preview,
    })
}

fn fallback_directive(
    ctx: &RuntimeSupervisorContext,
    raw_model_preview: Option<String>,
) -> StrategyDirective {
    directive_with_hash(StrategyDirective {
        schema_v: 1,
        strategy_kind: StrategyKind::StrategyPivot,
        trigger: ctx.trigger.clone(),
        root_cause: fallback_root_cause(ctx),
        actions: vec![StrategyAction {
            tool: best_fallback_tool(ctx),
            command_hint: None,
            reason: "Stop repeating the same tool; inspect current DB/evidence state and take one narrow stage-allowed action that closes a real gap.".to_string(),
        }],
        allowed_tools: Vec::new(),
        forbidden_tools: Vec::new(),
        submit_after_actions: false,
        directive_hash: String::new(),
        raw_model_preview,
    })
}

fn sanitize_directive(
    ctx: &RuntimeSupervisorContext,
    mut directive: StrategyDirective,
) -> StrategyDirective {
    let allowed = policy_allowed_tools(ctx);
    let forbidden = policy_forbidden_tools(ctx);
    directive.allowed_tools = allowed.clone();
    directive.forbidden_tools = forbidden.clone();
    directive.actions = directive
        .actions
        .into_iter()
        .filter_map(|mut action| {
            if let Some(tool) = action.tool.clone() {
                let normalized = normalize_tool_for_policy(&tool, &allowed, ctx);
                if normalized
                    .as_deref()
                    .is_some_and(|t| forbidden.iter().any(|f| f == t))
                {
                    action.tool = best_fallback_tool(ctx);
                } else {
                    action.tool = normalized;
                }
            }
            if action
                .tool
                .as_ref()
                .is_some_and(|tool| !allowed.is_empty() && !allowed.iter().any(|a| a == tool))
            {
                return None;
            }
            Some(action)
        })
        .collect();
    if directive.actions.is_empty() {
        directive.actions.push(StrategyAction {
            tool: best_fallback_tool(ctx),
            command_hint: None,
            reason: "Use the narrowest allowed tool to inspect current evidence/coverage before launching more scans.".to_string(),
        });
    }
    directive_with_hash(directive)
}

fn policy_allowed_tools(ctx: &RuntimeSupervisorContext) -> Vec<String> {
    let mut allowed = Vec::new();
    for tool in &ctx.visible_tools {
        push_unique(&mut allowed, tool);
    }
    if let Some(stage) = ctx.stage {
        if let Ok(spec) = load_embedded_stage_spec(stage) {
            let canonical = allowed_tool_names(&spec.allowed_tool_types);
            if !canonical.is_empty() && ctx.visible_tools.iter().any(|tool| tool == "pentest_run") {
                push_unique(&mut allowed, "pentest_run");
            }
            for tool in COMMON_STAGE_TOOLS {
                if ctx.visible_tools.is_empty() || ctx.visible_tools.iter().any(|t| t == *tool) {
                    push_unique(&mut allowed, tool);
                }
            }
        }
    }
    allowed
}

fn policy_forbidden_tools(ctx: &RuntimeSupervisorContext) -> Vec<String> {
    let mut forbidden = Vec::new();
    let in_repair = ctx.active_repair_directive.is_some();
    let eas = ctx.stage == Some(StageKind::ExternalAttackSurface);
    if in_repair || eas {
        for tool in BROAD_DISCOVERY_TOOLS {
            push_unique(&mut forbidden, tool);
        }
    }
    forbidden
}

fn normalize_tool_for_policy(
    tool: &str,
    allowed: &[String],
    ctx: &RuntimeSupervisorContext,
) -> Option<String> {
    let tool = clean_tool_name(tool);
    if tool.is_empty() {
        return None;
    }
    if allowed.iter().any(|a| a == &tool) {
        return Some(tool);
    }
    if ctx
        .stage
        .and_then(|stage| load_embedded_stage_spec(stage).ok())
        .is_some_and(|spec| {
            allowed_tool_names(&spec.allowed_tool_types)
                .into_iter()
                .any(|inner| inner == tool)
        })
        && allowed.iter().any(|a| a == "pentest_run")
    {
        return Some("pentest_run".to_string());
    }
    None
}

fn best_fallback_tool(ctx: &RuntimeSupervisorContext) -> Option<String> {
    let allowed = policy_allowed_tools(ctx);
    for preferred in [
        "check_stage_asset_coverage",
        "query_target_data",
        "wait_for_background_jobs",
        "submit_stage_deliverable",
        "pentest_run",
    ] {
        if allowed.iter().any(|tool| tool == preferred) {
            return Some(preferred.to_string());
        }
    }
    allowed.into_iter().next()
}

fn fallback_root_cause(ctx: &RuntimeSupervisorContext) -> String {
    format!(
        "Runtime trigger '{}' fired after '{}' produced the same failed pattern {} time(s).",
        ctx.trigger, ctx.repeated_tool, ctx.repeat_count
    )
}

fn parse_model_directive(text: &str) -> Option<ModelDirective> {
    let trimmed = text.trim();
    serde_json::from_str::<ModelDirective>(trimmed)
        .ok()
        .or_else(|| extract_json_object(trimmed).and_then(|json| serde_json::from_str(&json).ok()))
}

fn extract_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| text[start..=end].to_string())
}

fn directive_with_hash(mut directive: StrategyDirective) -> StrategyDirective {
    let mut clone = directive.clone();
    clone.directive_hash.clear();
    let bytes = serde_json::to_vec(&clone).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    directive.directive_hash = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    directive
}

fn clean_tool_name(tool: impl AsRef<str>) -> String {
    tool.as_ref().trim().trim_matches('`').trim().to_string()
}

fn push_unique(items: &mut Vec<String>, value: impl AsRef<str>) {
    let value = value.as_ref().trim();
    if !value.is_empty() && !items.iter().any(|item| item == value) {
        items.push(value.to_string());
    }
}

fn join_limited(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "(none)".to_string();
    }
    let mut out = items
        .iter()
        .take(limit)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        out.push_str(&format!(", +{} more", items.len() - limit));
    }
    out
}

fn safe_preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RuntimeSupervisorContext {
        RuntimeSupervisorContext {
            stage: Some(StageKind::ExternalAttackSurface),
            agent_path: "main>prober".to_string(),
            agent_role: "prober".to_string(),
            task: "Run EAS".to_string(),
            trigger: "execution_monitor".to_string(),
            repeated_tool: "whatweb".to_string(),
            repeat_count: 3,
            recent_calls: "whatweb({})".to_string(),
            last_tool_name: "pentest_run".to_string(),
            last_tool_result: "{}".to_string(),
            visible_tools: vec![
                "pentest_run".to_string(),
                "query_target_data".to_string(),
                "submit_stage_deliverable".to_string(),
                "list_in_scope_targets".to_string(),
            ],
            active_repair_directive: Some("coverage gaps".to_string()),
        }
    }

    #[test]
    fn supervisor_parses_and_sanitizes_broad_tools() {
        let raw = r#"{
          "strategy_kind": "strategy_pivot",
          "root_cause": "broad repeat",
          "actions": [{"tool": "list_in_scope_targets", "reason": "list again"}],
          "submit_after_actions": false
        }"#;
        let directive = directive_from_model_response(&ctx(), Some(raw));
        assert!(directive
            .forbidden_tools
            .contains(&"list_in_scope_targets".to_string()));
        assert!(!directive
            .actions
            .iter()
            .any(|a| a.tool.as_deref() == Some("list_in_scope_targets")));
    }

    #[test]
    fn supervisor_maps_inner_stage_tool_to_pentest_run() {
        let raw = r#"{
          "strategy_kind": "strategy_pivot",
          "root_cause": "need http probe",
          "actions": [{"tool": "httpx", "command_hint": "httpx -u https://a.test", "reason": "fill HTTP probe"}],
          "submit_after_actions": true
        }"#;
        let directive = directive_from_model_response(&ctx(), Some(raw));
        assert_eq!(directive.actions[0].tool.as_deref(), Some("pentest_run"));
        assert!(directive.submit_after_actions);
    }
}
