//! Build the per-turn list of `rig::completion::ToolDefinition`s by
//! delegating to the active [`crate::execution_mode::ExecutionModePolicy`].
//!
//! The hard-coded chat / task `if/else` branches that used to live here —
//! including the `tool.name.starts_with("pentest_")` filter that silently
//! dropped the eight `pentest_bridge` tools in chat mode — have been
//! lifted out into per-mode policies under
//! `crate::execution_mode::modes::*`. Adding a new mode is now a matter of
//! creating one new file and registering it in
//! `ExecutionModeRegistry::default`. This module no longer needs touching.

use golish_agent_kit::tool_definitions::get_all_tool_definitions;
use golish_sub_agents::SubAgentContext;

use super::context::AgenticLoopContext;
use crate::execution_mode::context::PolicyContext;
use crate::execution_mode::selection_apply::apply_tool_selection;

pub(crate) async fn build_tool_list(
    ctx: &AgenticLoopContext<'_>,
    sub_agent_context: &SubAgentContext,
) -> Vec<rig::completion::ToolDefinition> {
    let mode_id: &str = ctx.execution_mode.into();
    let policy = match ctx.execution_mode_registry.get(mode_id) {
        Some(p) => p,
        None => {
            tracing::error!(
                "[tool_list] unknown execution mode '{}', falling back to chat",
                mode_id
            );
            ctx.execution_mode_registry
                .get("chat")
                .expect("default ExecutionModeRegistry must contain `chat`")
        }
    };

    let workspace_guard = ctx.workspace.read().await;
    let policy_ctx = PolicyContext::new(&workspace_guard, golish_core::AgentMode::default())
        .with_depth(sub_agent_context.depth)
        .with_mcp_tool_count(ctx.additional_tool_definitions.len())
        .with_harness_active(ctx.harness_stage.is_some());
    let selection = if sub_agent_context.depth == 0 {
        policy.primary_tools(&policy_ctx).await
    } else {
        policy.subtask_tools(&policy_ctx).await
    };
    drop(workspace_guard);

    let mut tools = apply_tool_selection(selection, ctx, sub_agent_context).await;
    // D1 · hide scan tools entirely for an active stage that permits none (e.g.
    // scoping / target_intel / reporting have empty `allowed_tool_types`). The
    // per-call guard already blocks them, but hiding them stops the model from
    // wasting turns trying a tool it could only ever be denied.
    hide_scans_for_zero_scan_stage(&mut tools, ctx.harness_stage);
    // Scoping is org-tree/review-only: the in-scope seed list (domains / IPs /
    // URLs) is ingested by the trusted UI/CLI before the stage, never authored
    // here or by Target Intel. Hide
    // `manage_targets` so the model literally cannot turn discovered subsidiaries
    // into targets or pop a target `scope_review` during scoping (user directive
    // 2026-06-13; methodology backstop at the tool boundary).
    hide_manage_targets_in_scoping(&mut tools, ctx.harness_stage);
    // Stages with an effective per-org specialist from the static spec must be
    // driven through `stage_run`; the
    // depth-0 primary is only a coordinator. Hide direct work tools so the model
    // cannot bypass per-org isolation and gates by calling sub-agents itself.
    if sub_agent_context.depth == 0 {
        hide_direct_work_tools_for_specialist_stage(&mut tools, ctx.harness_stage);
    }
    // Read-only coverage self-check: the depth-0 stage orchestrator builds its
    // coverage matrix from what actually landed in the DB, but task
    // `primary_tools` is orchestration-only (no static groups) so the read-only
    // data-query tools never reach it — live evidence 2026-06-16 shows the
    // model calling `query_target_data`, hitting a tool-guard BLOCK, then
    // guessing the matrix (target_intel coverage dead-loop). Surface them just
    // for the depth-0 primary while a stage is active. Read-only: no scan/write.
    if sub_agent_context.depth == 0 {
        add_read_only_target_query_tools_for_stage(&mut tools, ctx.harness_stage);
        confine_target_intel_primary_tools(&mut tools, ctx.harness_stage);
    }
    confine_unified_investigation_cognitive_tools(&mut tools, ctx.harness_stage);
    configure_target_intel_fixture_public_tools(&mut tools, ctx);
    tools
}

/// Unified Investigation actors are cognition-only profiles. A role name such
/// as `pentester`, `browser`, `coder`, or `installer` must never recover that
/// role's ordinary raw executor surface while it is bound to this stage. The
/// only mutable operations retained here are orchestration/delegation and the
/// typed stage submission boundary; external actions are compiled and executed
/// by the host-owned Verification Operator outside these actors.
fn confine_unified_investigation_cognitive_tools(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    stage: Option<golish_agent_kit::harness::StageKind>,
) {
    if stage != Some(golish_agent_kit::harness::StageKind::Investigation) {
        return;
    }
    tools.retain(|tool| {
        matches!(
            tool.name.as_str(),
            "stage_run"
                | "update_plan"
                | "query_target_data"
                | "list_in_scope_targets"
                | "list_recent_evidence"
                | "search_memories"
                | "search_knowledge_base"
                | "read_knowledge"
                | "graph_search"
                | "graph_neighbors"
                | "graph_attack_paths"
                | "harness_trace"
                | "submit_result"
                | "submit_stage_deliverable"
        ) || tool.name.starts_with("sub_agent_")
    });
}

fn configure_target_intel_fixture_public_tools(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    ctx: &AgenticLoopContext<'_>,
) {
    let enabled = ctx
        .target_intel_goal_shadow
        .is_some_and(|fixture| fixture.strict_passive_public_tools_enabled())
        // Existing/production operations hard-skip before an adapter or
        // provider request can be selected.
        && ctx.harness_operation_id.is_none();
    apply_target_intel_fixture_public_tools(tools, enabled);
}

fn apply_target_intel_fixture_public_tools(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    enabled: bool,
) {
    if !enabled {
        return;
    }
    tools.retain(|tool| {
        !matches!(
            tool.name.as_str(),
            "web_search" | "web_fetch" | "intel_public_search" | "intel_public_fetch"
        )
    });
    tools.extend(golish_agent_kit::tool_executors::intel_public_tool_definitions());
    tools.push(recon_search_intel_fixture_definition());
    tools.push(rig::completion::ToolDefinition {
        name: golish_sub_agents::STAGE_TEAM_SPAWN_INTEL_SUBAGENTS.to_string(),
        description: "Fixture/dev-only Goal-owner primitive for dynamic generic Intel workers. The model supplies only display name, exact task prompt, and subject refs; the host stamps role, kind, tools and terminal contract.".to_string(),
        parameters: golish_sub_agents::target_intel_spawn_subagents_schema(),
    });
    tools.push(rig::completion::ToolDefinition {
        name: golish_sub_agents::STAGE_TEAM_REQUEST_INTEL_REVIEW.to_string(),
        description: "Fixture/dev-only observe-only review request. The model supplies only its bounded completion claim; the host freezes state/actions/contract and invokes a read-only reviewer.".to_string(),
        parameters: golish_sub_agents::target_intel_request_review_schema(),
    });
}

fn recon_search_intel_fixture_definition() -> rig::completion::ToolDefinition {
    rig::completion::ToolDefinition {
        name: "recon_search_intel".to_string(),
        description: "Fixture/dev-only semantic Target Intel collection. The model supplies only organization, semantic pivot, and intent; provider selection, query compilation, evidence and projection remain host-owned. Identity-anchored brand/domain/email/GitHub/repository/app hypotheses are passive candidate-only searches and grant no scope, reachability, promotion, or active authorization.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "organization_id": { "type": "string", "format": "uuid" },
                "pivot": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": [
                                "company_name", "brand", "domain", "hostname", "ip", "cidr",
                                "asn", "certificate", "icp", "email_domain", "github_org",
                                "repository", "app_id"
                            ]
                        },
                        "value": { "type": "string", "minLength": 1, "maxLength": 512 }
                    },
                    "required": ["kind", "value"]
                },
                "intent": {
                    "type": "string",
                    "enum": ["discover_related_assets", "verify_attribution", "enrich_known_asset"]
                }
            },
            "required": ["organization_id", "pivot", "intent"]
        }),
    }
}

/// Surface the read-only target/coverage query tools
/// (`list_in_scope_targets` + `query_target_data`) for the depth-0 orchestrator
/// while a harness stage is active.
///
/// Task `primary_tools` is orchestration-only (`static_groups: none`), so these
/// static-catalogue tools never reach the stage orchestrator. Without them the
/// model calls `query_target_data` to self-check coverage, hits the per-turn
/// tool-guard ("not in allowed list"), and falls back to guessing the coverage
/// matrix — the live target_intel coverage dead-loop. Both are read-only DB
/// reads (no scan / write / exec surface), so they are safe in every stage.
///
/// Idempotent: skips any tool already present (subtask depth>0 gets them via the
/// static groups) and is a no-op when no harness stage is active.
fn add_read_only_target_query_tools_for_stage(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    if harness_stage.is_none()
        || harness_stage == Some(golish_agent_kit::harness::StageKind::TargetIntel)
    {
        return;
    }
    // `query_target_data(target_id)` needs ids; `list_in_scope_targets` is its
    // documented companion ("call FIRST to discover targets, then drill in").
    const READ_ONLY_QUERY_TOOLS: &[&str] = &[
        "list_in_scope_targets",
        "list_attack_surface_seeds",
        "query_target_data",
        "check_stage_asset_coverage",
        "stage_worklist_status",
        "stage_worklist_next",
        "list_recent_evidence",
    ];
    let any_missing = READ_ONLY_QUERY_TOOLS
        .iter()
        .any(|name| !tools.iter().any(|t| t.name == *name));
    if !any_missing {
        return;
    }
    let catalogue = get_all_tool_definitions();
    for name in READ_ONLY_QUERY_TOOLS {
        if tools.iter().any(|t| t.name == *name) {
            continue;
        }
        if let Some(def) = catalogue.iter().find(|t| t.name == *name) {
            tools.push(def.clone());
            tracing::debug!(
                target: "harness::hook",
                tool = *name,
                "tool-list: surfaced read-only target query tool for the depth-0 stage orchestrator"
            );
        }
    }
}

/// Target Intel's depth-0 actor is only a host-stage coordinator. The actual
/// Goal owner is the durable Company Controller created by `stage_run`; DB
/// coverage/worklist reads and direct recon entry points belong to the retired
/// formulaic flow and must not leak back into the primary tool surface.
fn confine_target_intel_primary_tools(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    if harness_stage != Some(golish_agent_kit::harness::StageKind::TargetIntel) {
        return;
    }
    tools.retain(|tool| {
        matches!(
            tool.name.as_str(),
            "stage_run" | "update_plan" | "submit_stage_deliverable"
        )
    });
}

/// Remove `manage_targets` from the exposed list when the active harness stage is
/// `scoping`. Scoping locks the ORG TREE/review decision; trusted UI/CLI seed
/// ingestion owns individual target creation. Hiding the tool here enforces that boundary even if the
/// model ignores the methodology. No-op for every other stage (and when no stage
/// is active), so target recording stays available downstream.
fn hide_manage_targets_in_scoping(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    if !matches!(
        harness_stage,
        Some(golish_agent_kit::harness::StageKind::Scoping)
    ) {
        return;
    }
    let before = tools.len();
    tools.retain(|t| t.name != "manage_targets");
    if tools.len() != before {
        tracing::debug!(
            target: "harness::hook",
            stage = "scoping",
            "tool-list: hid manage_targets (scoping is org-tree-only; targets belong to target_intel)"
        );
    }
}

fn hide_direct_work_tools_for_specialist_stage(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    let Some(kind) = harness_stage else {
        return;
    };
    let Ok(spec) = golish_agent_kit::harness::load_embedded_stage_spec(kind) else {
        return;
    };
    let specialist = spec
        .specialist
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let Some(specialist) = specialist else {
        return;
    };

    const DIRECT_WORK_TOOLS: &[&str] = &[
        "manage_targets",
        "recon_discover_subsidiaries",
        "recon_lookup_company",
        "recon_lookup_whois",
        "recon_list_providers",
        "recon_map_assets",
        "recon_search_intel",
    ];

    let before = tools.len();
    tools.retain(|t| {
        !DIRECT_WORK_TOOLS.contains(&t.name.as_str()) && !t.name.starts_with("sub_agent_")
    });
    if tools.len() != before {
        tracing::debug!(
            target: "harness::hook",
            stage = %kind.as_str(),
            specialist,
            removed = before - tools.len(),
            "tool-list: hid direct work tools for specialist stage; use stage_run"
        );
    }
}

/// D1 · when an active harness stage allows no scan tools, strip scan-execution
/// tools AND offensive sub-agent dispatchers (pentester / browser) from the
/// exposed list so the model never attempts (or delegates) work it could only be
/// denied. No-op when no stage is active or the stage permits ≥1 scan type.
///
/// Phase 2 (2026-06-12-redteam-phase2): `recon/osint` is the API-driven passive
/// class — its tools (`recon_discover_subsidiaries` / `recon_map_assets`, the
/// ENScan providers) are registry tools that never go through a scanner/shell
/// surface. A stage whose whitelist contains ONLY such API classes (scoping
/// after the subsidiary gate landed) still has no use for `pentest_run` /
/// `run_pty_cmd` / offensive sub-agents, so it keeps the zero-scan hiding.
fn hide_scans_for_zero_scan_stage(
    tools: &mut Vec<rig::completion::ToolDefinition>,
    harness_stage: Option<golish_agent_kit::harness::StageKind>,
) {
    /// Tool types whose tools dispatch via the registry (HTTP API providers),
    /// not via a scan-execution surface.
    const API_ONLY_TOOL_TYPES: &[&str] = &["recon/osint"];
    let Some(kind) = harness_stage else {
        return;
    };
    let Ok(spec) = golish_agent_kit::harness::load_embedded_stage_spec(kind) else {
        return;
    };
    let needs_scan_surface = spec
        .allowed_tool_types
        .iter()
        .any(|t| !API_ONLY_TOOL_TYPES.contains(&t.as_str()));
    if needs_scan_surface {
        return;
    }
    let before = tools.len();
    // Hide scan-execution tools AND offensive sub-agent dispatchers: a stage that
    // permits no scans must not delegate active recon / exploitation either, or a
    // weak model burns the whole stage re-submitting + spawning a pentester it
    // could only ever be blocked on (the per-call guard still backstops scans).
    tools.retain(|t| {
        !golish_agent_kit::harness::is_scan_tool_name(&t.name)
            && !golish_agent_kit::harness::is_offensive_sub_agent(&t.name)
    });
    if tools.len() != before {
        tracing::debug!(
            target: "harness::hook",
            stage = %kind.as_str(),
            removed = before - tools.len(),
            "tool-list: hid scan + offensive sub-agent tools for a stage that permits none"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContextBuilder;

    fn td(name: &str) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: name.to_string(),
            description: "d".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn root_fixture_receives_only_host_owned_intel_public_tools() {
        let mut tools = vec![td("web_search"), td("web_fetch"), td("query_target_data")];
        apply_target_intel_fixture_public_tools(&mut tools, true);
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"intel_public_search"));
        assert!(names.contains(&"intel_public_fetch"));
        assert!(!names.contains(&"web_search"));
        assert!(!names.contains(&"web_fetch"));
        assert!(names.contains(&"query_target_data"));
    }

    #[test]
    fn unified_investigation_profiles_never_receive_raw_action_tools() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![
            td("update_plan"),
            td("query_target_data"),
            td("sub_agent_pentester"),
            td("web_fetch"),
            td("browser_navigate"),
            td("pentest_run"),
            td("vault"),
            td("record_finding"),
            td("write_file"),
        ];
        confine_unified_investigation_cognitive_tools(&mut tools, Some(StageKind::Investigation));
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["update_plan", "query_target_data", "sub_agent_pentester"]
        );
    }

    /// D1 · a stage with no scan-surface tool types (scoping: whitelist is the
    /// API-only `recon/osint` since Phase 2) hides scan tools but keeps
    /// meta/control-plane tools; a scan-permitting stage (enumeration) and
    /// the no-stage case leave the list untouched.
    #[test]
    fn hide_scans_strips_scan_tools_only_in_zero_scan_stage() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![
            td("pentest_run"),
            td("run_pty_cmd"),
            td("submit_stage_deliverable"),
            td("query_target_data"),
            td("sub_agent_pentester"),
            td("sub_agent_reporter"),
        ];
        hide_scans_for_zero_scan_stage(&mut tools, Some(StageKind::Scoping));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"pentest_run"),
            "scan wrapper must be hidden"
        );
        assert!(
            !names.contains(&"run_pty_cmd"),
            "scan wrapper must be hidden"
        );
        assert!(
            !names.contains(&"sub_agent_pentester"),
            "offensive sub-agent must be hidden in a zero-scan stage: {names:?}"
        );
        assert!(
            names.contains(&"submit_stage_deliverable")
                && names.contains(&"query_target_data")
                && names.contains(&"sub_agent_reporter"),
            "meta + non-offensive sub-agents must be kept: {names:?}"
        );

        // enumeration permits scans → nothing stripped.
        let mut tools2 = vec![td("pentest_run")];
        hide_scans_for_zero_scan_stage(&mut tools2, Some(StageKind::Enumeration));
        assert_eq!(tools2.len(), 1, "scan-permitting stage must not hide scans");

        // no active stage → no-op.
        let mut tools3 = vec![td("pentest_run")];
        hide_scans_for_zero_scan_stage(&mut tools3, None);
        assert_eq!(tools3.len(), 1, "no stage → no filtering");
    }

    /// `manage_targets` is hidden in scoping (org-tree-only) but kept everywhere
    /// else (target_intel records the in-scope target list) and when no stage is
    /// active. manage_organizations + ask_human always survive scoping.
    #[test]
    fn manage_targets_hidden_only_in_scoping() {
        use golish_agent_kit::harness::StageKind;

        let base = || {
            vec![
                td("manage_targets"),
                td("manage_organizations"),
                td("ask_human"),
                td("recon_discover_subsidiaries"),
            ]
        };

        let mut scoping = base();
        hide_manage_targets_in_scoping(&mut scoping, Some(StageKind::Scoping));
        let names: Vec<&str> = scoping.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"manage_targets"),
            "manage_targets must be hidden in scoping: {names:?}"
        );
        assert!(
            names.contains(&"manage_organizations") && names.contains(&"ask_human"),
            "org/ask tools must survive scoping: {names:?}"
        );

        // This helper is scoping-only; specialist-stage filtering is tested below.
        let mut ti = base();
        hide_manage_targets_in_scoping(&mut ti, Some(StageKind::TargetIntel));
        assert!(ti.iter().any(|t| t.name == "manage_targets"));

        // no active stage → untouched.
        let mut none = base();
        hide_manage_targets_in_scoping(&mut none, None);
        assert!(none.iter().any(|t| t.name == "manage_targets"));
    }

    #[test]
    fn specialist_stage_primary_hides_direct_work_tools() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![
            td("stage_run"),
            td("submit_stage_deliverable"),
            td("manage_organizations"),
            td("manage_targets"),
            td("recon_list_providers"),
            td("recon_discover_subsidiaries"),
            td("recon_map_assets"),
            td("recon_lookup_whois"),
            td("sub_agent_recon"),
            td("query_target_data"),
        ];
        hide_direct_work_tools_for_specialist_stage(&mut tools, Some(StageKind::TargetIntel));

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for kept in [
            "stage_run",
            "submit_stage_deliverable",
            "manage_organizations",
            "query_target_data",
        ] {
            assert!(
                names.contains(&kept),
                "specialist stage primary must keep {kept}; got: {names:?}"
            );
        }
        for hidden in [
            "manage_targets",
            "recon_list_providers",
            "recon_discover_subsidiaries",
            "recon_map_assets",
            "recon_lookup_whois",
            "sub_agent_recon",
        ] {
            assert!(
                !names.contains(&hidden),
                "specialist stage primary must use stage_run instead of {hidden}; got: {names:?}"
            );
        }

        let mut scoping_tools = vec![td("recon_map_assets")];
        hide_direct_work_tools_for_specialist_stage(&mut scoping_tools, Some(StageKind::Scoping));
        assert_eq!(
            scoping_tools.len(),
            1,
            "non-specialist stages must not use the specialist-stage filter"
        );
    }

    /// Ordinary depth-0 stage orchestrators get DB-backed query/worklist tools.
    /// Target Intel is excluded because its Goal owner plans from semantic
    /// search/frontier receipts and delegates review to the host-owned reviewer.
    #[test]
    fn read_only_query_tools_surfaced_for_active_stage_orchestrator() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![td("submit_stage_deliverable"), td("update_plan")];
        add_read_only_target_query_tools_for_stage(&mut tools, Some(StageKind::Enumeration));
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"query_target_data"),
            "stage orchestrator must get query_target_data for coverage self-check: {names:?}"
        );
        assert!(
            names.contains(&"list_in_scope_targets"),
            "must also get its list companion (source of target ids): {names:?}"
        );
        assert!(
            names.contains(&"check_stage_asset_coverage"),
            "stage orchestrator must get the DB-truth coverage preflight: {names:?}"
        );
        assert!(
            names.contains(&"stage_worklist_status"),
            "stage orchestrator must get compact stage worklist status: {names:?}"
        );
        assert!(
            names.contains(&"stage_worklist_next"),
            "stage orchestrator must get the next DB-truth work batch: {names:?}"
        );
        assert!(
            names.contains(&"list_recent_evidence"),
            "stage orchestrator must get the real-evidence-id lister so it can cite ids: {names:?}"
        );

        // Idempotent: a second pass (or a tool already present) adds no dupes.
        let before = tools.len();
        add_read_only_target_query_tools_for_stage(&mut tools, Some(StageKind::Enumeration));
        assert_eq!(
            tools.len(),
            before,
            "must not duplicate already-present query tools"
        );

        // No active stage → untouched (chat / non-stage turns).
        let mut none = vec![td("submit_stage_deliverable")];
        add_read_only_target_query_tools_for_stage(&mut none, None);
        assert_eq!(none.len(), 1, "no stage → no additions");

        let mut target_intel = vec![td("stage_run"), td("update_plan")];
        add_read_only_target_query_tools_for_stage(&mut target_intel, Some(StageKind::TargetIntel));
        assert_eq!(
            target_intel.len(),
            2,
            "Target Intel must not recover the retired coverage/worklist surface"
        );
    }

    #[test]
    fn target_intel_primary_is_stage_run_only() {
        use golish_agent_kit::harness::StageKind;

        let mut tools = vec![
            td("stage_run"),
            td("update_plan"),
            td("submit_stage_deliverable"),
            td("manage_organizations"),
            td("query_target_data"),
            td("stage_worklist_next"),
            td("recon_search_intel"),
        ];
        confine_target_intel_primary_tools(&mut tools, Some(StageKind::TargetIntel));
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["stage_run", "update_plan", "submit_stage_deliverable"]
        );
    }

    use golish_agent_kit::execution_mode::ExecutionMode;
    use golish_agent_kit::tool_definitions::{ToolPreset, ToolSelectionConfig};
    use golish_llm_providers::LlmClient;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// Chat-mode primary turn must include the static toolbox basics
    /// (read_file, run_pty_cmd, ask_human). This is the live regression
    /// guard for the `tool.name.starts_with("pentest_")` filter bug.
    #[tokio::test]
    async fn chat_mode_includes_static_tools_and_run_command() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Chat)
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.iter().any(|n| n == "read_file"),
            "chat must expose static file_ops; got: {:?}",
            names
        );
        assert!(names.iter().any(|n| n == "run_pty_cmd"));
        assert!(names.iter().any(|n| n == "ask_human"));
        assert!(
            names.iter().all(|n| !n.starts_with("sub_agent_")),
            "chat mode must NOT expose sub_agent_* dispatchers"
        );
    }

    /// ToolPreset::None is used by silent utility sessions such as
    /// title generation. It must suppress policy-level aliases too.
    #[tokio::test]
    async fn none_tool_preset_exposes_no_tools_even_in_chat_mode() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Chat)
            .tool_config(ToolSelectionConfig::with_preset(ToolPreset::None))
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.is_empty(),
            "ToolPreset::None must expose no tools; got: {:?}",
            names
        );
    }

    /// Task/profile lead turns are flexible for chat/coding, but they must not
    /// be able to run security operations directly. Real scoping/recon/pentest
    /// work enters the harness only through `start_operation`.
    #[tokio::test]
    async fn task_lead_primary_includes_decision_tools_without_security_or_shell() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        {
            let mut reg = test_ctx.tool_registry.write().await;
            reg.register_tool(Arc::new(MockNamedTool("start_operation")));
            reg.register_tool(Arc::new(MockNamedTool("submit_stage_deliverable")));
            reg.register_tool(Arc::new(MockNamedTool("manage_organizations")));
            reg.register_tool(Arc::new(MockNamedTool("manage_targets")));
            reg.register_tool(Arc::new(MockNamedTool("recon_map_assets")));
            reg.register_tool(Arc::new(MockNamedTool("pentest_run")));
        }
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.iter().any(|n| n == "read_file"),
            "task lead must expose static file_ops; got: {:?}",
            names
        );
        for expected in ["grep_file", "list_files", "ast_grep"] {
            assert!(
                names.iter().any(|n| n == expected),
                "task lead must keep normal coding/search helper {expected}; got: {:?}",
                names
            );
        }
        assert!(
            !names.iter().any(|n| n == "run_pty_cmd"),
            "task lead must not expose shell; got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == "run_command"),
            "task lead must not expose shell alias; got: {:?}",
            names
        );
        assert!(names.iter().any(|n| n == "ask_human"));
        assert!(names.iter().any(|n| n == "start_operation"));
        assert!(
            !names.iter().any(|n| n == "submit_stage_deliverable"),
            "task lead must not expose harness submit; got: {:?}",
            names
        );
        for forbidden in [
            "query_target_data",
            "list_in_scope_targets",
            "list_attack_surface_seeds",
            "check_stage_asset_coverage",
            "stage_worklist_status",
            "stage_worklist_next",
            "list_recent_evidence",
            "ingest_cve",
            "save_poc",
            "search_exploits",
            "manage_organizations",
            "manage_targets",
            "recon_map_assets",
            "pentest_run",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "task lead must enter the harness via start_operation instead of exposing {forbidden}; got: {:?}",
                names
            );
        }
    }

    /// Once a harness stage is active, the depth-0 primary switches to the
    /// stage-orchestrator tool surface: no static tools or shell.
    #[tokio::test]
    async fn active_task_stage_primary_has_no_static_tools_or_run_command() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        {
            let mut reg = test_ctx.tool_registry.write().await;
            reg.register_tool(Arc::new(MockNamedTool("start_operation")));
        }
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.harness_stage = Some(golish_agent_kit::harness::StageKind::Scoping);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            !names.iter().any(|n| n == "read_file"),
            "active task stage primary must NOT expose static file_ops; got: {:?}",
            names
        );
        assert!(!names.iter().any(|n| n == "run_pty_cmd"));
        assert!(names.iter().any(|n| n == "ask_human"));
        assert!(
            !names.iter().any(|n| n == "start_operation"),
            "active task stage primary must not expose nested start_operation; got: {:?}",
            names
        );
    }

    #[tokio::test]
    async fn active_specialist_stage_primary_must_use_stage_run_not_direct_recon() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        {
            let mut reg = test_ctx.tool_registry.write().await;
            for name in [
                "submit_stage_deliverable",
                "manage_organizations",
                "manage_targets",
                "recon_list_providers",
                "recon_discover_subsidiaries",
                "recon_map_assets",
                "recon_lookup_whois",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
            ] {
                reg.register_tool(Arc::new(MockNamedTool(name)));
            }
        }
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.harness_stage = Some(golish_agent_kit::harness::StageKind::TargetIntel);

        let names: Vec<String> = build_tool_list(&ctx, &SubAgentContext::default())
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(
            names.iter().any(|n| n == "stage_run"),
            "target_intel primary must see stage_run; got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n == "submit_stage_deliverable"),
            "target_intel primary must keep submit for pass-token closeout; got: {:?}",
            names
        );
        for forbidden in [
            "manage_organizations",
            "manage_targets",
            "recon_list_providers",
            "recon_discover_subsidiaries",
            "recon_map_assets",
            "recon_lookup_whois",
            "query_target_data",
            "check_stage_asset_coverage",
            "stage_worklist_status",
            "stage_worklist_next",
            "wait_for_background_jobs",
            "check_job",
            "kill_job",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "target_intel primary must not bypass stage_run via {forbidden}; got: {:?}",
                names
            );
        }
    }

    /// A minimal registry tool used to prove the bridge allow-list path
    /// actually surfaces a dynamically-registered tool by name.
    struct MockNamedTool(&'static str);

    #[async_trait::async_trait]
    impl golish_core::Tool for MockNamedTool {
        fn name(&self) -> &'static str {
            self.0
        }
        fn description(&self) -> &'static str {
            "mock tool for tool-list wiring tests"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _workspace: &std::path::Path,
        ) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({ "status": "ok" }))
        }
    }

    /// End-to-end wiring guard: a registry tool named `submit_stage_deliverable`
    /// must reach the LLM tool list only after the task harness is active (the
    /// depth-0 stage orchestrator and depth-1 specialists) via the bridge
    /// allow-list, but must NOT leak into task lead turns or chat mode.
    #[tokio::test]
    async fn submit_stage_deliverable_surfaces_in_active_task_stage_not_lead_or_chat() {
        async fn names_for(mode: ExecutionMode, depth: usize, active_stage: bool) -> Vec<String> {
            let test_ctx = TestContextBuilder::new().execution_mode(mode).build().await;
            {
                let mut reg = test_ctx.tool_registry.write().await;
                reg.register_tool(Arc::new(MockNamedTool("submit_stage_deliverable")));
            }
            let client = Arc::new(RwLock::new(LlmClient::Mock));
            let mut ctx = test_ctx.as_agentic_context_with_client(&client);
            if active_stage {
                ctx.harness_stage = Some(golish_agent_kit::harness::StageKind::Scoping);
            }
            let sub = SubAgentContext {
                depth,
                ..Default::default()
            };
            build_tool_list(&ctx, &sub)
                .await
                .into_iter()
                .map(|t| t.name)
                .collect()
        }

        let task_lead = names_for(ExecutionMode::Task, 0, false).await;
        assert!(
            !task_lead.iter().any(|n| n == "submit_stage_deliverable"),
            "task lead must NOT expose submit_stage_deliverable; got: {task_lead:?}"
        );

        let task_stage_primary = names_for(ExecutionMode::Task, 0, true).await;
        assert!(
            task_stage_primary
                .iter()
                .any(|n| n == "submit_stage_deliverable"),
            "active task stage primary must expose submit_stage_deliverable; got: {task_stage_primary:?}"
        );

        let task_subtask = names_for(ExecutionMode::Task, 1, true).await;
        assert!(
            task_subtask.iter().any(|n| n == "submit_stage_deliverable"),
            "task subtask (specialist) must expose submit_stage_deliverable; got: {task_subtask:?}"
        );

        let chat_primary = names_for(ExecutionMode::Chat, 0, false).await;
        assert!(
            !chat_primary.iter().any(|n| n == "submit_stage_deliverable"),
            "chat mode must NOT expose submit_stage_deliverable; got: {chat_primary:?}"
        );
    }

    /// Task subtask (depth=1) inherits the full toolbox minus update_plan
    /// and ask_human (subtasks must not block on user input).
    #[tokio::test]
    async fn task_subtask_includes_static_minus_update_plan_and_ask_human() {
        let test_ctx = TestContextBuilder::new()
            .execution_mode(ExecutionMode::Task)
            .build()
            .await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let ctx = test_ctx.as_agentic_context_with_client(&client);

        let subtask = SubAgentContext {
            depth: 1,
            ..Default::default()
        };

        let names: Vec<String> = build_tool_list(&ctx, &subtask)
            .await
            .into_iter()
            .map(|t| t.name)
            .collect();

        assert!(names.iter().any(|n| n == "read_file"));
        assert!(names.iter().any(|n| n == "run_pty_cmd"));
        assert!(
            !names.iter().any(|n| n == "update_plan"),
            "subtask must NOT expose update_plan; got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == "ask_human"),
            "subtasks must NOT block on ask_human"
        );
    }
}
