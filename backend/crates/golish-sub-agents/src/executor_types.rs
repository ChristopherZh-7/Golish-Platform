use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rig::completion::request::ToolDefinition;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use golish_core::events::AiEvent;
use golish_core::ApiRequestStats;
use golish_tools::ToolRegistry;

/// Barrier tool name used by all sub-agents to submit structured results.
/// When a sub-agent calls this tool, the executor terminates the loop and
/// returns the structured result to the parent agent (PentAGI barrier pattern).
pub const BARRIER_TOOL_NAME: &str = "submit_result";

/// Trait for providing tool definitions to the sub-agent executor.
/// This allows the executor to be decoupled from the tool definition source.
#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    /// Get all available tool definitions
    fn get_all_tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Filter tools to only those allowed by the sub-agent
    fn filter_tools_by_allowed(
        &self,
        tools: Vec<ToolDefinition>,
        allowed: &[String],
    ) -> Vec<ToolDefinition>;

    /// Execute a web fetch tool
    async fn execute_web_fetch_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> (serde_json::Value, bool);

    async fn execute_memory_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Execute a knowledge-base / vulnerability-wiki tool (`search_knowledge_base`,
    /// `read_knowledge`, `write_knowledge`, `ingest_cve`, `save_poc`, …). Returns
    /// `None` when `tool_name` isn't a KB tool so the caller falls through to the
    /// router / registry — same contract as [`Self::execute_memory_tool`].
    async fn execute_knowledge_base_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Normalize run_pty_cmd arguments
    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value;
}

/// Trait for sub-agent chain persistence operations (decoupled from sqlx).
///
/// Provides the message-chain DB operations the executor needs to
/// save/restore persistent chains across sub-agent invocations.
#[async_trait::async_trait]
pub trait SubAgentChainPersistence: Send + Sync {
    async fn chain_create(
        &self,
        session_id: uuid::Uuid,
        task_id: Option<uuid::Uuid>,
        subtask_id: Option<uuid::Uuid>,
        agent_type: &str,
        parent_chain_id: Option<uuid::Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<uuid::Uuid>;

    async fn chain_update(
        &self,
        id: uuid::Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()>;

    async fn chain_update_usage(
        &self,
        id: uuid::Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()>;

    /// Load the most recent persisted chain for `(session, agent)`, if any, so a
    /// `resume` delegation can replay the same sub-agent's prior conversation
    /// (including the tool results / evidence ids it produced). Returns
    /// `(chain_id, chain_json)`. Default impl: no prior chain (never resumes).
    async fn chain_load_latest(
        &self,
        _session_id: uuid::Uuid,
        _task_id: Option<uuid::Uuid>,
        _agent_type: &str,
    ) -> anyhow::Result<Option<(uuid::Uuid, serde_json::Value)>> {
        Ok(None)
    }

    /// Load a SPECIFIC persisted chain by its id, so a `resume` delegation can
    /// continue an exact prior sub-agent conversation. The id is handed back to
    /// the orchestrator when the sub-agent finishes, so it can name precisely
    /// which worker to resume. Default impl: not found.
    async fn chain_load_by_id(
        &self,
        _chain_id: uuid::Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)>;
}

/// Shared cancellation probe for sub-agent workers.
///
/// The top-level [`AgentBridge`](golish_agent_bridge) owns the flag. Worker
/// executors only borrow it so user "Stop" requests can interrupt nested
/// sub-agent loops without letting a child clear the parent cancellation.
pub(crate) fn cancellation_requested(cancelled: Option<&Arc<AtomicBool>>) -> bool {
    cancelled
        .map(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or(false)
}

pub(crate) async fn wait_for_cancelled(cancelled: Option<&Arc<AtomicBool>>) {
    loop {
        if cancellation_requested(cancelled) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Async callback invoked after a shell command completes.
///
/// Arguments: (command, stdout, project_path, organization_id).
/// The closure captures external resources (e.g. a DB pool) it needs.
pub type PostShellHook = Arc<
    dyn Fn(
            String,
            String,
            Option<String>,
            Option<uuid::Uuid>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Extra tool executor for tools that live OUTSIDE the [`ToolRegistry`]
/// (for example security-analysis read helpers and graph tools). Given
/// `(tool_name, args)` it returns
/// `Some((value, success))` if it handled the call, else `None` to fall through
/// to the registry. A plain `Fn` so this crate stays free of harness-crate
/// deps; the runtime builds it with the needed app/runtime backends wired in.
pub type SubAgentToolRouter = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
        > + Send
        + Sync,
>;

/// Optional hook invoked after a sub-agent tool returns and before the result is
/// emitted back to the model. Runtime crates use this to mirror main-agent
/// harness side effects (for example evidence/source-query materialization)
/// without making `golish-sub-agents` depend on the harness or database layers.
pub type SubAgentToolResultHook = Arc<
    dyn Fn(
            String,
            serde_json::Value,
            serde_json::Value,
            bool,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = (serde_json::Value, bool)> + Send>>
        + Send
        + Sync,
>;

/// Owned snapshot of a completed sub-agent tool call for runtime observers.
///
/// This crate intentionally does not know about harness gates, DB evidence, or
/// mentor LLMs. Runtime crates can attach a callback that watches the generic
/// stream of tool outcomes and optionally returns model-visible guidance.
#[derive(Debug, Clone)]
pub struct SubAgentToolObservation {
    pub agent_id: String,
    pub agent_name: String,
    pub parent_request_id: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub result: serde_json::Value,
    pub success: bool,
}

/// Optional observer invoked after normal sub-agent tool post-processing.
///
/// Returning `Some(text)` appends that text to the model-visible ToolResult.
/// Returning `None` keeps the observation trace-only.
pub type SubAgentToolObserver = Arc<
    dyn Fn(
            SubAgentToolObservation,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// Per-stage tool-boundary guard for sub-agent tool calls.
///
/// Given `(tool_name, args)` it returns `Err(reason)` when the call is NOT
/// allowed in the active harness stage (deny-by-default category whitelist —
/// see `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`). The
/// sub-agent executor calls it before running each tool and turns `Err` into a
/// synthetic error result instead of executing.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolGuard = Arc<dyn Fn(&str, &serde_json::Value) -> Result<(), String> + Send + Sync>;

/// Per-stage tool-list filter controlling tool *visibility* (D1).
///
/// Returns `true` when a tool name should be HIDDEN from the sub-agent's tool
/// list for the active harness stage (e.g. scan tools during a no-scan stage
/// like `scoping`). Mirrors the main agent's `hide_scans_for_zero_scan_stage`
/// so a delegated sub-agent never even *sees* a tool it could only be denied —
/// which stops the retry spin where the model hammers a blocked tool. The
/// call-time [`StageToolGuard`] remains the backstop.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolHider = Arc<dyn Fn(&str) -> bool + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitRepairKind {
    EvidenceRefs,
    BackgroundJobs,
    CoverageGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageGapAction {
    pub asset: String,
    pub technique: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_tools: Vec<String>,
}

/// Deterministic refiner directive produced from `submit_stage_deliverable`
/// `needs_fix`. It is intentionally small and serializable so upper runtime
/// layers can persist it in `operation_state.state_blob.agent_run`, then inject
/// it back into a resumed sub-agent without depending on transient loop memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitRepairMode {
    pub kind: SubmitRepairKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_required_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coverage_gap_actions: Vec<CoverageGapAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools_override: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_message: Option<String>,
}

impl SubmitRepairMode {
    fn base_allowed_tools(&self) -> &'static [&'static str] {
        match self.kind {
            SubmitRepairKind::EvidenceRefs => &[
                "submit_stage_deliverable",
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
            ],
            SubmitRepairKind::BackgroundJobs => &[
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "check_stage_asset_coverage",
                "submit_stage_deliverable",
            ],
            SubmitRepairKind::CoverageGap if self.coverage_gap_actions.is_empty() => &[
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
            SubmitRepairKind::CoverageGap => &[
                "pentest_list_tools",
                "pentest_run",
                "list_recent_evidence",
                "check_stage_asset_coverage",
                "query_target_data",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
        }
    }

    fn effective_allowed_tools(&self) -> Vec<String> {
        let mut tools = if self.allowed_tools_override.is_empty() {
            self.base_allowed_tools()
                .iter()
                .map(|tool| (*tool).to_string())
                .collect()
        } else {
            self.allowed_tools_override.clone()
        };
        if self.kind == SubmitRepairKind::CoverageGap {
            append_coverage_gap_worklist_tools(&mut tools);
            if !self.coverage_gap_actions.is_empty() {
                append_direct_enumeration_repair_tools(&mut tools, &self.coverage_gap_actions);
            }
        }
        tools
    }

    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            SubmitRepairKind::EvidenceRefs => "evidence_refs",
            SubmitRepairKind::BackgroundJobs => "background_jobs",
            SubmitRepairKind::CoverageGap => "coverage_gap",
        }
    }

    pub fn allowed_tool_names(&self) -> Vec<String> {
        self.effective_allowed_tools()
    }

    pub fn allows(&self, tool_name: &str) -> bool {
        !self.forbidden_tools.iter().any(|tool| tool == tool_name)
            && self
                .effective_allowed_tools()
                .iter()
                .any(|tool| tool == tool_name)
    }

    pub fn model_instruction(&self) -> String {
        let mut message = self
            .directive_message
            .clone()
            .unwrap_or_else(|| match self.kind {
                SubmitRepairKind::EvidenceRefs => {
                    "submit_stage_deliverable returned needs_fix for evidence/submit fields, so \
                 repair-only mode is active. Do NOT start fresh discovery or launch new scans. \
                 Rebuild the StageDeliverable from the real evidence ids already returned, use \
                 query_target_data only if you need to map ids to claims, then resubmit."
                        .to_string()
                }
                SubmitRepairKind::BackgroundJobs => {
                    "submit_stage_deliverable returned needs_fix because background jobs are still \
                 pending. Do NOT launch replacement scans. Call wait_for_background_jobs, inspect \
                 the completed output tails, then resubmit."
                        .to_string()
                }
                SubmitRepairKind::CoverageGap => {
                    if self.coverage_gap_actions.is_empty() {
                        "submit_stage_deliverable returned needs_fix for coverage, but the gate \
                     did not name any concrete coverage_gap_actions. Repair-only mode is active. \
                     Do NOT launch discovery, guessed-domain probes, pentest_run, CIDR/range \
                     sweeps, or broad rediscovery. Call stage_worklist_status/stage_worklist_next, \
                     check_stage_asset_coverage, or query_target_data to reconcile DB truth; if \
                     there are no in-scope assets or no named gaps, resubmit the \
                     no-asset/terminal deliverable instead of inventing targets."
                            .to_string()
                    } else {
                        "submit_stage_deliverable returned needs_fix because the stage coverage matrix \
                     still has missing or non-terminal cells. Targeted gap-closure mode is active: \
                     use the gate feedback plus read-only stage_worklist_status/stage_worklist_next, \
                     check_stage_asset_coverage, or query_target_data instead of re-listing the \
                     entire attack surface. Run only stage-allowed probes for the exact \
                     asset/technique pairs named in the gate feedback. Batch sibling gap assets with \
                     input_lines/list-file mode when every target is present in coverage_gap_actions. \
                     Do NOT call list_in_scope_targets, list_attack_surface_seeds, \
                     CIDR/range sweeps, targets outside coverage_gap_actions, or broad rediscovery. \
                     When each named gap has a terminal coverage cell, resubmit."
                            .to_string()
                    }
                }
            });
        if !self.missing_required_checks.is_empty() {
            message.push_str(&format!(
                " Also include these required_checks_done entries if the cited evidence backs \
                 them: [{}].",
                self.missing_required_checks.join(", ")
            ));
        }
        if self.kind == SubmitRepairKind::CoverageGap && !self.coverage_gap_actions.is_empty() {
            message.push_str(&coverage_gap_action_instruction(&self.coverage_gap_actions));
        }
        message
    }

    pub fn block_result(&self, tool_name: &str) -> Option<serde_json::Value> {
        self.block_result_with_args(tool_name, &serde_json::Value::Null)
    }

    pub fn block_result_with_args(
        &self,
        tool_name: &str,
        tool_args: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        if self.allows(tool_name) {
            if self.kind == SubmitRepairKind::CoverageGap && tool_name == "pentest_run" {
                if let Some(reason) = coverage_gap_pentest_run_block_reason(tool_args) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
                if let Some(reason) =
                    coverage_gap_action_target_block_reason(tool_args, &self.coverage_gap_actions)
                {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            if self.kind == SubmitRepairKind::CoverageGap
                && is_direct_enumeration_repair_tool(tool_name)
            {
                if let Some(reason) = coverage_gap_direct_tool_target_block_reason(
                    tool_name,
                    tool_args,
                    &self.coverage_gap_actions,
                ) {
                    return Some(self.block_payload(tool_name, Some(reason)));
                }
            }
            return None;
        }
        Some(self.block_payload(tool_name, None))
    }

    fn block_payload(&self, tool_name: &str, blocked_reason: Option<String>) -> serde_json::Value {
        let mut value = serde_json::json!({
            "error": self.model_instruction(),
            "blocked_by_submit_repair": true,
            "repair_kind": self.kind_str(),
            "blocked_tool": tool_name,
            "allowed_tools": self.effective_allowed_tools(),
            "last_needs_fix_reason": self.reason,
        });
        if let Some(reason) = blocked_reason {
            value["blocked_reason"] = serde_json::Value::String(reason);
        }
        if !self.coverage_gap_actions.is_empty() {
            value["coverage_gap_actions"] =
                serde_json::to_value(&self.coverage_gap_actions).unwrap_or_default();
        }
        if !self.forbidden_tools.is_empty() {
            value["forbidden_tools"] =
                serde_json::to_value(&self.forbidden_tools).unwrap_or_default();
        }
        value
    }
}

fn coverage_gap_action_instruction(actions: &[CoverageGapAction]) -> String {
    let mut lines = vec![format!(
        " Exact coverage_gap_actions from the gate: run ONLY these {} target/technique pairs. \
         Do not run a target or technique that is absent from this list. Direct enumeration tools \
         (browser_collect_js_api/js_collect/js_extract_apis/route_probe_paths) may be called by \
         name when suggested here; directory discovery must use route_probe_paths, not external \
         ffuf/gobuster/feroxbuster. Bounded crawler CLIs such as katana must be run through \
         pentest_run(tool_name=...) and used only as URL sources.",
        actions.len()
    )];
    for (idx, action) in actions.iter().enumerate() {
        let tools = if action.suggested_tools.is_empty() {
            String::new()
        } else {
            format!("; suggested_tools={}", action.suggested_tools.join(", "))
        };
        lines.push(format!(
            "{}. asset={} technique={} reason={}{}",
            idx + 1,
            action.asset,
            action.technique,
            action.reason,
            tools
        ));
    }
    format!("\n{}", lines.join("\n"))
}

fn append_direct_enumeration_repair_tools(tools: &mut Vec<String>, actions: &[CoverageGapAction]) {
    for action in actions {
        for suggested in &action.suggested_tools {
            let suggested = suggested
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if is_direct_enumeration_repair_tool(&suggested) {
                push_unique_tool(tools, &suggested);
            }
        }
        match action.technique.as_str() {
            "GOLISH-ENUM-JSAPI" => {
                push_unique_tool(tools, "browser_collect_js_api");
                push_unique_tool(tools, "js_collect");
                push_unique_tool(tools, "js_extract_apis");
            }
            "GOLISH-ENUM-DIR" => push_unique_tool(tools, "route_probe_paths"),
            "GOLISH-ENUM-PARAM" => push_unique_tool(tools, "js_extract_apis"),
            _ => {}
        }
    }
}

fn append_coverage_gap_worklist_tools(tools: &mut Vec<String>) {
    push_unique_tool(tools, "stage_worklist_status");
    push_unique_tool(tools, "stage_worklist_next");
}

fn push_unique_tool(tools: &mut Vec<String>, tool: &str) {
    if !tool.is_empty() && !tools.iter().any(|existing| existing == tool) {
        tools.push(tool.to_string());
    }
}

fn is_direct_enumeration_repair_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "browser_collect_js_api" | "js_collect" | "js_extract_apis" | "route_probe_paths"
    )
}

fn coverage_gap_pentest_run_block_reason(tool_args: &serde_json::Value) -> Option<String> {
    let args = pentest_run_args(tool_args);

    let tool = tool_args
        .get("tool_name")
        .or_else(|| tool_args.get("tool"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        tool.as_str(),
        "" | "httpx" | "nmap" | "naabu" | "masscan" | "whatweb"
    ) {
        return None;
    }

    if contains_cidr_target(&args) || contains_cidr_target_values(&input_line_targets(tool_args)) {
        return Some(
            "coverage-gap repair blocks CIDR/range sweeps; probe one exact named asset".to_string(),
        );
    }
    None
}

fn coverage_gap_direct_tool_target_block_reason(
    tool_name: &str,
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    // Single + batch inputs (design 2026-07-03): route_probe_paths uses
    // base_url / targets[].base_url; the JS tools use target_url / target_urls[].
    let (single_key, batch_key) = match tool_name {
        "route_probe_paths" => ("base_url", "targets"),
        "browser_collect_js_api" | "js_collect" | "js_extract_apis" => {
            ("target_url", "target_urls")
        }
        _ => return None,
    };
    let mut targets: Vec<String> = Vec::new();
    if let Some(single) = tool_args.get(single_key).and_then(|v| v.as_str()) {
        let single = single.trim();
        if !single.is_empty() {
            targets.push(single.to_string());
        }
    }
    if let Some(batch) = tool_args.get(batch_key).and_then(|v| v.as_array()) {
        for item in batch {
            // route_probe_paths batch entries are objects with a base_url; the
            // JS tools' batch entries are bare URL strings.
            let candidate = item
                .as_str()
                .or_else(|| item.get("base_url").and_then(|v| v.as_str()));
            if let Some(candidate) = candidate.map(str::trim).filter(|s| !s.is_empty()) {
                targets.push(candidate.to_string());
            }
        }
    }
    if targets.is_empty() {
        return Some(format!(
            "coverage-gap repair requires {tool_name} target(s) ({single_key} or {batch_key}) so they can be checked against coverage_gap_actions"
        ));
    }
    let allowed = actions
        .iter()
        .map(|action| normalize_probe_target(&action.asset))
        .filter(|asset| !asset.is_empty())
        .collect::<HashSet<_>>();
    // Any single target outside the named gaps blocks the whole call — a batch
    // must not smuggle an un-named target past the coverage-gap fence.
    let blocked = targets
        .iter()
        .find(|target| !allowed.contains(&normalize_probe_target(target)))?;
    let mut preview = actions
        .iter()
        .take(20)
        .map(|action| format!("{} × {}", action.asset, action.technique))
        .collect::<Vec<_>>();
    if actions.len() > preview.len() {
        preview.push(format!("... +{} more", actions.len() - preview.len()));
    }
    Some(format!(
        "coverage-gap repair blocks {tool_name} target '{blocked}' because it is not in coverage_gap_actions; only probe [{}]",
        preview.join(", ")
    ))
}

fn coverage_gap_action_target_block_reason(
    tool_args: &serde_json::Value,
    actions: &[CoverageGapAction],
) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let args = pentest_run_args(tool_args);
    if args.is_empty() {
        return None;
    }
    let targets = probe_targets_from_tool_args(tool_args);
    if targets.is_empty() {
        if uses_hidden_target_file(&args) {
            return Some(
                "coverage-gap repair list-file/stdin probes must provide input_lines/stdin so targets can be checked against coverage_gap_actions"
                    .to_string(),
            );
        }
        return None;
    }
    let allowed = actions
        .iter()
        .map(|action| normalize_probe_target(&action.asset))
        .filter(|asset| !asset.is_empty())
        .collect::<HashSet<_>>();
    let blocked = targets
        .iter()
        .find(|target| !allowed.contains(&normalize_probe_target(target)))?;
    let mut preview = actions
        .iter()
        .take(20)
        .map(|action| format!("{} × {}", action.asset, action.technique))
        .collect::<Vec<_>>();
    if actions.len() > preview.len() {
        preview.push(format!("... +{} more", actions.len() - preview.len()));
    }
    Some(format!(
        "coverage-gap repair blocks target '{blocked}' because it is not in coverage_gap_actions; only probe [{}]",
        preview.join(", ")
    ))
}

fn pentest_run_args(tool_args: &serde_json::Value) -> String {
    tool_args
        .get("args")
        .or_else(|| tool_args.get("arguments"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn probe_targets_from_tool_args(tool_args: &serde_json::Value) -> Vec<String> {
    let mut targets = probe_targets(&pentest_run_args(tool_args));
    targets.extend(input_line_targets(tool_args));
    targets.sort();
    targets.dedup();
    targets
}

fn input_line_targets(tool_args: &serde_json::Value) -> Vec<String> {
    let mut targets = Vec::new();
    if let Some(lines) = tool_args.get("input_lines").and_then(|v| v.as_array()) {
        for line in lines {
            if let Some(line) = line.as_str() {
                targets.extend(probe_targets(line));
            }
        }
    }
    if let Some(stdin) = tool_args.get("stdin").and_then(|v| v.as_str()) {
        for line in stdin.lines() {
            targets.extend(probe_targets(line));
        }
    }
    targets
}

fn uses_hidden_target_file(args: &str) -> bool {
    let args_lc = args.to_ascii_lowercase();
    args_lc.contains("<<")
        || args_lc.contains("/dev/stdin")
        || args_lc.contains(" -l ")
        || args_lc.starts_with("-l ")
        || args_lc.contains(" --list ")
        || args_lc.starts_with("--list ")
        || args_lc.contains(" -list ")
        || args_lc.starts_with("-list ")
        || args_lc.contains(" -il ")
        || args_lc.starts_with("-il ")
        || args_lc.contains(" --input-file")
        || args_lc.starts_with("--input-file")
}

fn contains_cidr_target(args: &str) -> bool {
    args.split_whitespace().any(|token| {
        let token = clean_probe_token(token);
        let Some((host, prefix)) = token.split_once('/') else {
            return false;
        };
        looks_like_ipv4(host) && prefix.parse::<u8>().is_ok_and(|bits| bits <= 32)
    })
}

fn contains_cidr_target_values(values: &[String]) -> bool {
    values.iter().any(|value| contains_cidr_target(value))
}

fn probe_targets(args: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut skip_next = false;
    for raw in args.split_whitespace() {
        let token = clean_probe_token(raw);
        if token.is_empty() {
            continue;
        }
        let token_lc = token.to_ascii_lowercase();
        if skip_next {
            skip_next = false;
            continue;
        }
        if option_takes_value(&token_lc) {
            skip_next = true;
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        if is_probe_target_token(&token_lc) {
            targets.push(token.to_string());
        }
    }
    targets
}

fn option_takes_value(token_lc: &str) -> bool {
    matches!(
        token_lc,
        "-o" | "-oa"
            | "-og"
            | "-on"
            | "-ox"
            | "-ol"
            | "-p"
            | "-ports"
            | "--ports"
            | "--top-ports"
            | "--rate"
            | "-rate"
            | "-t"
            | "-timeout"
            | "--timeout"
            | "-output"
            | "--output"
    )
}

fn is_probe_target_token(token_lc: &str) -> bool {
    if matches!(
        token_lc,
        "http" | "https" | "tcp" | "udp" | "true" | "false"
    ) {
        return false;
    }
    token_lc.starts_with("http://")
        || token_lc.starts_with("https://")
        || looks_like_ipv4(token_lc)
        || (token_lc.contains('.') && !looks_like_output_path(token_lc))
}

fn looks_like_output_path(token_lc: &str) -> bool {
    matches!(
        token_lc.rsplit('.').next(),
        Some("out" | "txt" | "json" | "xml" | "csv" | "log")
    )
}

fn normalize_probe_target(value: &str) -> String {
    let mut s = clean_probe_token(value).trim().to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("http://") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("https://") {
        s = rest.to_string();
    }
    if let Some(idx) = s.find(['/', '?', '#']) {
        s.truncate(idx);
    }
    if let Some(idx) = s.rfind('@') {
        s = s[idx + 1..].to_string();
    }
    if let Some((host, port)) = s.rsplit_once(':') {
        if !host.contains(':') && port.chars().all(|c| c.is_ascii_digit()) {
            s = host.to_string();
        }
    }
    s.trim_end_matches('.').to_string()
}

fn clean_probe_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '\'' | '"' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    })
}

fn looks_like_ipv4(value: &str) -> bool {
    let mut parts = value.split('.');
    let mut count = 0;
    for part in &mut parts {
        count += 1;
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if part.parse::<u8>().is_err() {
            return false;
        }
    }
    count == 4
}

/// Context needed for sub-agent execution.
pub struct SubAgentExecutorContext<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
    /// Provider name (e.g., "openai", "anthropic_vertex") for model capability checks
    pub provider_name: &'a str,
    /// Model name for model capability checks
    pub model_name: &'a str,
    /// Session ID for Langfuse tracing (propagated from parent agent)
    pub session_id: Option<&'a str>,
    /// Base directory for transcript files (e.g., `~/.golish/transcripts`)
    /// If set, sub-agent internal events will be written to separate transcript files.
    pub transcript_base_dir: Option<&'a std::path::Path>,
    /// API request stats collector (per session, optional)
    pub api_request_stats: Option<&'a Arc<ApiRequestStats>>,
    /// Shared top-level cancel flag. When set, this worker should stop before
    /// starting a new LLM request/tool call and should interrupt stream waits.
    pub cancelled: Option<&'a Arc<AtomicBool>>,
    /// Orchestrator briefing injected before execution. Contains relevant memories,
    /// execution plan context, and findings from other agents. Appended to the
    /// effective system prompt as a `## Briefing from Orchestrator` section.
    pub briefing: Option<String>,
    /// Per-agent temperature override from settings (None = use default 0.3).
    pub temperature_override: Option<f32>,
    /// Per-agent max_tokens override from settings (None = use default 8192).
    pub max_tokens_override: Option<u32>,
    /// Per-agent top_p override from settings (None = not sent to provider).
    pub top_p_override: Option<f32>,
    /// Chain persistence backend for saving/restoring sub-agent conversation
    /// chains (PentAGI-style). Replaces the raw `sqlx::PgPool`.
    pub chain_persistence: Option<&'a Arc<dyn SubAgentChainPersistence>>,
    /// Sub-agent registry for nested delegation (PentAGI hierarchical pattern).
    /// When set, agents with `delegatable_agents` can invoke other sub-agents.
    pub sub_agent_registry: Option<&'a Arc<RwLock<crate::definition::SubAgentRegistry>>>,
    /// Optional hook called after a successful shell tool execution.
    pub post_shell_hook: Option<PostShellHook>,
    /// AI-controlled resume handle for this delegation:
    /// `Some("<chain_id>")` continues that exact prior sub-agent conversation;
    /// `Some("latest")` continues this agent's most recent chain; `None` is a
    /// fresh sub-agent (default). Enables "go back to the same worker".
    pub resume: Option<String>,
    /// Extra tool executor for tools outside the `ToolRegistry` (security/graph),
    /// tried before the registry fallback so a delegated sub-agent can actually
    /// run them (e.g. `graph_add_entity`) instead of getting "Unknown tool".
    pub sub_tool_router: Option<SubAgentToolRouter>,
    /// Writable active-org side-channel used by legacy registry tools. Per-org
    /// stage_run workers prefer hidden per-call org args for org-aware tools
    /// (`manage_targets` / `manage_organizations`) and keep this as a fallback.
    pub active_org_id_source: Option<Arc<RwLock<Option<uuid::Uuid>>>>,
    /// The org id this sub-agent is bound to. Org-aware registry tools receive it
    /// as an internal hidden arg; non-injectable tools may fall back to the
    /// side-channel above. `None` preserves the parent/global binding.
    pub active_org_id_override: Option<uuid::Uuid>,
    /// Optional post-processing hook for regular tool results. This keeps the
    /// executor generic while allowing the agent runtime to attach harness
    /// evidence/source logging to sub-agent tool calls.
    pub post_tool_result_hook: Option<SubAgentToolResultHook>,
    /// Optional observer for completed regular tool calls. Used by upper layers
    /// to implement runtime mentor/supervisor logic without making this crate
    /// depend on those layers.
    pub tool_observer: Option<SubAgentToolObserver>,
    /// Optional persisted submit-repair directive restored from
    /// `operation_state.state_blob.agent_run`. When present, the sub-agent starts
    /// in deterministic repair-only mode instead of rediscovering the same stage
    /// from scratch after a resume/retry.
    pub initial_submit_repair_mode: Option<SubmitRepairMode>,
    /// Optional per-stage tool boundary guard (forbidden-only). When the
    /// sub-agent runs inside a harness stage, this blocks tool calls whose
    /// resolved capability is in the stage's forbidden list (e.g. `dig` in
    /// scoping), before execution. `None` = no active stage (legacy behaviour).
    pub stage_tool_guard: Option<StageToolGuard>,
    /// Optional per-stage tool-list filter (D1). When the sub-agent runs inside
    /// a harness stage that permits no scan tools (e.g. `scoping`), this hides
    /// scan tools from the exposed list so the model never even attempts one —
    /// preventing the retry spin the call-time guard alone can't stop. `None` =
    /// no filtering (legacy behaviour).
    pub hide_tool_in_stage: Option<StageToolHider>,
}
