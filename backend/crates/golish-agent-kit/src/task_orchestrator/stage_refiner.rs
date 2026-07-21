//! Stage-aware deterministic refiner for gate/submit repair.
//!
//! The deterministic gate remains the only PASS/BLOCK authority. This module
//! owns the next-step repair directive after a gate/submit failure: compact the
//! DB-backed gate result, stage spec, and tool guidance into a structured
//! directive that the runtime can persist and convert into sub-agent
//! `SubmitRepairMode`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::harness::{
    load_embedded_stage_spec, suggested_capabilities_for_technique, CoverageGapAction, StageKind,
};

const MODEL_RECOVERY_ACTION_SAMPLE_LIMIT: usize = 20;
const MODEL_RECOVERY_INSTRUCTION_MAX_BYTES: usize = 32 * 1024;
const MODEL_RECOVERY_PROJECTION_MARKER: &str = "Recovery actions: total=";
const MODEL_RECOVERY_TRUNCATION_SUFFIX: &str =
    "\n[Recovery instruction truncated. Use stage_worklist_next for bounded DB-backed pages.]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairKind {
    EvidenceRefs,
    BackgroundJobs,
    CoverageGap,
    GateBlock,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageCellDraft {
    pub asset: String,
    pub technique: String,
    pub expected_status: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDraft {
    pub kind: String,
    pub subject: String,
    pub technique: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmitGuidanceMode {
    RebuildEvidenceRefs,
    WaitForBackgroundJobs,
    FillCoverageCells,
    ResubmitAfterGateBlock,
    GenericRepair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitGuidance {
    pub mode: SubmitGuidanceMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_coverage_cells: Vec<CoverageCellDraft>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_claims: Vec<ClaimDraft>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_level_evidence_refs: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairAction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technique: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairDirective {
    pub schema_v: u32,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<Uuid>,
    pub agent_path: String,
    pub repair_kind: RepairKind,
    pub root_cause: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<RepairAction>,
    pub submit_guidance: SubmitGuidance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale_or_unusable_evidence_ids: Vec<i64>,
    pub gate_reason_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_hash: Option<String>,
    pub llm_escalated: bool,
}

#[derive(Debug, Clone)]
pub struct RefinerContext {
    pub stage: StageKind,
    pub org_id: Option<Uuid>,
    pub agent_path: String,
    pub reasons: Vec<String>,
    pub coverage_gap_actions: Vec<CoverageGapAction>,
    pub available_evidence_ids: Vec<i64>,
    pub running_background_jobs: Vec<String>,
}

impl RepairDirective {
    pub fn model_instruction(&self) -> String {
        let mut out = format!(
            "STAGE REFINER DIRECTIVE (deterministic, DB-backed): {}\n\
             Stage: {}. Repair kind: {:?}. Do not restart the stage; perform only the actions below.",
            bounded_model_field(&self.root_cause, 2_048),
            bounded_model_field(&self.stage, 256),
            self.repair_kind
        );
        let sampled_actions = self
            .actions
            .iter()
            .take(MODEL_RECOVERY_ACTION_SAMPLE_LIMIT)
            .collect::<Vec<_>>();
        if !self.actions.is_empty() {
            let stable_hash = self
                .gap_hash
                .clone()
                .unwrap_or_else(|| short_hash(&self.actions));
            out.push_str(&format!(
                "\n{MODEL_RECOVERY_PROJECTION_MARKER}{} stable_hash={} sample_count={}. \
                 Only this bounded sample is shown; the complete ordered action set remains \
                 enforced internally. Call stage_worklist_next for bounded DB-backed pages; \
                 do not infer authorization from this sample.",
                self.actions.len(),
                stable_hash,
                sampled_actions.len()
            ));
        }
        if !self.allowed_tools.is_empty() {
            out.push_str(&format!(
                "\nAllowed next tools: [{}].",
                bounded_model_list(&self.allowed_tools, 64, 128).join(", ")
            ));
        }
        if !self.forbidden_tools.is_empty() {
            out.push_str(&format!(
                "\nForbidden in this repair: [{}].",
                bounded_model_list(&self.forbidden_tools, 64, 128).join(", ")
            ));
        }
        if let Some(batch_hint) = self.eas_batch_instruction(&sampled_actions) {
            out.push_str(&batch_hint);
        }
        if !sampled_actions.is_empty() {
            out.push_str("\nAction sample:");
            for (idx, action) in sampled_actions.iter().enumerate() {
                let mut line =
                    format!(" {}. {}", idx + 1, bounded_model_field(&action.reason, 512));
                if let Some(asset) = action.asset.as_deref() {
                    line.push_str(&format!(" asset={}", bounded_model_field(asset, 512)));
                }
                if let Some(technique) = action.technique.as_deref() {
                    line.push_str(&format!(
                        " technique={}",
                        bounded_model_field(technique, 256)
                    ));
                }
                if let Some(capability_id) = action.capability_id.as_deref() {
                    line.push_str(&format!(
                        " capability={}",
                        bounded_model_field(capability_id, 256)
                    ));
                }
                if let Some(tool) = action.tool.as_deref() {
                    line.push_str(&format!(" tool={}", bounded_model_field(tool, 256)));
                }
                if let Some(status) = action.expected_status.as_deref() {
                    line.push_str(&format!(
                        " submit_status={}",
                        bounded_model_field(status, 256)
                    ));
                }
                if let Some(hint) = action.command_hint.as_deref() {
                    line.push_str(&format!(" hint={}", bounded_model_field(hint, 1_024)));
                }
                out.push('\n');
                out.push_str(&line);
            }
        }
        out.push_str("\nThen call submit_stage_deliverable once with terminal coverage/claims that cite real evidence ids when required.");
        cap_recovery_model_text(out)
    }

    fn eas_batch_instruction(&self, sampled_actions: &[&RepairAction]) -> Option<String> {
        if self.stage != StageKind::ExternalAttackSurface.as_str() {
            return None;
        }
        if !matches!(
            self.repair_kind,
            RepairKind::CoverageGap | RepairKind::GateBlock
        ) {
            return None;
        }

        let assets_for = |technique: &str| -> (usize, Vec<String>) {
            let total = self
                .actions
                .iter()
                .filter(|action| action.technique.as_deref() == Some(technique))
                .count();
            let sample = sampled_actions
                .iter()
                .filter(|action| action.technique.as_deref() == Some(technique))
                .filter_map(|action| action.asset.clone())
                .take(5)
                .collect();
            (total, sample)
        };
        let (liveness_total, liveness) = assets_for("GOLISH-EAS-LIVENESS");
        let (ports_total, ports) = assets_for("GOLISH-EAS-PORT");
        let (services_total, services) = assets_for("GOLISH-EAS-SERVICE-FINGERPRINT");
        let (web_total, web) = assets_for("GOLISH-EAS-WEB-FINGERPRINT");
        if liveness_total == 0 && ports_total == 0 && services_total == 0 && web_total == 0 {
            return None;
        }

        let mut out = String::from(
            "\nBatching: EAS repair is batch-first. Group sibling gap actions by \
             technique and use as few EAS wrapper calls as possible; do not run \
             one foreground tool call per asset. Use eas_probe_http_liveness, \
             eas_discover_ports, eas_fingerprint_services, and \
             eas_fingerprint_web_stack directly. Do not call raw pentest_run or \
             raw whatweb/nmap/httpx in repair mode.",
        );
        if liveness_total > 0 {
            out.push_str(&format!(
                "\n- LIVENESS/eas_probe_http_liveness: for domain/URL/vhost gaps, one wrapper \
                 call with targets[] like below. For concrete IP/CIDR liveness gaps, run \
                 PORT/eas_discover_ports first instead; port evidence closes IP liveness. \
                 Sample targets:\n{}",
                sample_assets(&liveness, liveness_total)
            ));
        }
        if ports_total > 0 {
            out.push_str(&format!(
                "\n- PORT/eas_discover_ports: one wrapper call with concrete IP/CIDR targets[] and scan_profile=full. The backend owns scanner/range/rate/retries/timeout; quick/standard remain partial and cannot close PORT. Full is bounded to at most four expanded IPv4 hosts (CIDR /30 or narrower) or exact IPv6 /128; a wider existing range produces a no-network, evidence-backed LIVENESS/PORT policy block, so do not alter or split its authorization. Sample targets:\n{}",
                sample_assets(&ports, ports_total)
            ));
        }
        if services_total > 0 {
            out.push_str(&format!(
                "\n- SERVICE/eas_fingerprint_services: use one wrapper call with the concrete-IP \
                 targets[] and normally omit ports[]. The backend reads exact pending confirmed-open \
                 ports per IP, chunks and schedules them concurrently, isolates slow targets, and \
                 performs one bounded recovery pass. ports[] can only narrow that server-owned set. \
                 Do not group by shared ports, increase timeouts, or replay an entire timed-out batch. \
                 Use eas_fingerprint_web_stack only for confirmed \
                 HTTP(S) web origins; never use web fingerprinting for DNS/MySQL/SSH/non-HTTP \
                 service gaps. \
                 Do not include unresolved hosts or assets with no confirmed open ports; close \
                 those cells as not_applicable/blocked with a concrete note instead. Sample targets:\n{}",
                sample_assets(&services, services_total)
            ));
        }
        if web_total > 0 {
            out.push_str(&format!(
                "\n- WEB-FINGERPRINT/eas_fingerprint_web_stack: one wrapper batch per confirmed \
                 HTTP(S) origin set with target_urls[]. Use absolute scheme://host[:port] URLs, and run once per \
                 confirmed vhost/origin even when several domains share one IP:port. Sample targets:\n{}",
                sample_assets(&web, web_total)
            ));
        }
        Some(out)
    }

    pub fn to_submit_repair_mode(&self) -> Option<golish_sub_agents::SubmitRepairMode> {
        let kind = match self.repair_kind {
            RepairKind::EvidenceRefs => golish_sub_agents::SubmitRepairKind::EvidenceRefs,
            RepairKind::BackgroundJobs => golish_sub_agents::SubmitRepairKind::BackgroundJobs,
            RepairKind::CoverageGap | RepairKind::GateBlock => {
                golish_sub_agents::SubmitRepairKind::CoverageGap
            }
            RepairKind::Generic => return None,
        };
        Some(golish_sub_agents::SubmitRepairMode {
            kind,
            reason: self.root_cause.clone(),
            missing_required_checks: Vec::new(),
            coverage_gap_actions: self
                .actions
                .iter()
                .filter_map(|action| {
                    Some(golish_sub_agents::CoverageGapAction {
                        asset: action.asset.clone()?,
                        technique: action.technique.clone()?,
                        reason: action.reason.clone(),
                        suggested_capabilities: action_capability_suggestions(action),
                        suggested_tools: normalized_action_tools(action),
                    })
                })
                .collect(),
            eas_web_repair_targets: None,
            allowed_tools_override: self.allowed_tools.clone(),
            forbidden_tools: self.forbidden_tools.clone(),
            directive_message: Some(self.model_instruction()),
        })
    }
}

pub fn refine_submit_needs_fix(ctx: RefinerContext) -> RepairDirective {
    let repair_kind = if !ctx.running_background_jobs.is_empty()
        || reasons_contain(
            &ctx.reasons,
            &["background job", "wait_for_background_jobs"],
        ) {
        RepairKind::BackgroundJobs
    } else if !ctx.coverage_gap_actions.is_empty() || reasons_look_like_coverage(&ctx.reasons) {
        RepairKind::CoverageGap
    } else if reasons_contain(
        &ctx.reasons,
        &["evidence_ref", "evidence id", "fabricated", "real evidence"],
    ) {
        RepairKind::EvidenceRefs
    } else {
        RepairKind::Generic
    };
    directive_from_context(ctx, repair_kind)
}

pub fn refine_gate_block(ctx: RefinerContext) -> RepairDirective {
    let repair_kind =
        if !ctx.coverage_gap_actions.is_empty() || reasons_look_like_coverage(&ctx.reasons) {
            RepairKind::CoverageGap
        } else {
            RepairKind::GateBlock
        };
    directive_from_context(ctx, repair_kind)
}

fn directive_from_context(ctx: RefinerContext, repair_kind: RepairKind) -> RepairDirective {
    let root_cause = root_cause_for(&ctx, repair_kind);
    let actions = repair_actions_for(&ctx, repair_kind);
    let submit_guidance = submit_guidance_for(repair_kind, &actions, &ctx.available_evidence_ids);
    let allowed_tools = allowed_tools_for(ctx.stage, repair_kind);
    let forbidden_tools = forbidden_tools_for(ctx.stage, repair_kind);
    let gate_reason_hash = short_hash(&ctx.reasons);
    let gap_hash =
        (!ctx.coverage_gap_actions.is_empty()).then(|| short_hash(&ctx.coverage_gap_actions));

    RepairDirective {
        schema_v: 1,
        stage: ctx.stage.as_str().to_string(),
        org_id: ctx.org_id,
        agent_path: ctx.agent_path,
        repair_kind,
        root_cause,
        actions,
        submit_guidance,
        forbidden_tools,
        allowed_tools,
        evidence_ids: ctx.available_evidence_ids,
        stale_or_unusable_evidence_ids: Vec::new(),
        gate_reason_hash,
        gap_hash,
        llm_escalated: false,
    }
}

fn root_cause_for(ctx: &RefinerContext, repair_kind: RepairKind) -> String {
    match repair_kind {
        RepairKind::BackgroundJobs => {
            "submit arrived before background jobs settled; wait for the existing jobs and reuse their output".to_string()
        }
        RepairKind::EvidenceRefs => {
            "deliverable evidence references are missing, fabricated, or not mapped to the claims; rebuild from real ledger ids".to_string()
        }
        RepairKind::CoverageGap => {
            let count = ctx.coverage_gap_actions.len();
            if count > 0 {
                format!("deterministic gate found {count} non-terminal coverage gap action(s)")
            } else {
                "deterministic gate found coverage gaps; close only the named stage-gate reasons"
                    .to_string()
            }
        }
        RepairKind::GateBlock => {
            "per-org stage gate blocked this worker; repair the named gate reasons and resubmit"
                .to_string()
        }
        RepairKind::Generic => ctx
            .reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "submit_stage_deliverable returned needs_fix".to_string()),
    }
}

fn repair_actions_for(ctx: &RefinerContext, repair_kind: RepairKind) -> Vec<RepairAction> {
    match repair_kind {
        RepairKind::BackgroundJobs => ctx
            .running_background_jobs
            .iter()
            .map(|job| RepairAction {
                asset: None,
                technique: None,
                capability_id: None,
                tool: Some("wait_for_background_jobs".to_string()),
                command_hint: Some(format!("wait_for_background_jobs for job {job}")),
                expected_status: Some("resubmit_after_jobs_complete".to_string()),
                evidence_refs: Vec::new(),
                note: Some("do not re-run the same background command".to_string()),
                reason: format!("wait for existing background job {job} and inspect its output"),
            })
            .collect(),
        RepairKind::EvidenceRefs => vec![RepairAction {
            asset: None,
            technique: None,
            capability_id: None,
            tool: Some("query_target_data".to_string()),
            command_hint: None,
            expected_status: Some("resubmit_with_real_evidence_refs".to_string()),
            evidence_refs: ctx.available_evidence_ids.clone(),
            note: None,
            reason: "map real evidence ids to claims/coverage, then resubmit without new scans"
                .to_string(),
        }],
        RepairKind::CoverageGap | RepairKind::GateBlock => {
            if ctx.coverage_gap_actions.is_empty() {
                return ctx
                    .reasons
                    .iter()
                    .take(20)
                    .map(|reason| RepairAction {
                        asset: None,
                        technique: None,
                        capability_id: None,
                        tool: Some("query_target_data".to_string()),
                        command_hint: None,
                        expected_status: Some("terminal_coverage_or_claim".to_string()),
                        evidence_refs: Vec::new(),
                        note: None,
                        reason: reason.clone(),
                    })
                    .collect();
            }
            ctx.coverage_gap_actions
                .iter()
                .map(|gap| {
                    let suggested_capabilities = if gap.suggested_capabilities.is_empty() {
                        suggested_capabilities_for_technique(ctx.stage, &gap.technique)
                    } else {
                        gap.suggested_capabilities.clone()
                    };
                    let capability_id = suggested_capabilities
                        .first()
                        .map(|capability| capability.id.clone());
                    let tool = gap
                        .suggested_tools
                        .first()
                        .and_then(|tool| {
                            normalized_stage_tool_hint(ctx.stage, &gap.technique, tool)
                        })
                        .or_else(|| {
                            suggested_capabilities
                                .iter()
                                .flat_map(|capability| capability.tools.iter())
                                .find_map(|tool| {
                                    normalized_stage_tool_hint(ctx.stage, &gap.technique, tool)
                                })
                        })
                        .or_else(|| suggested_tool_for(ctx.stage, &gap.technique));
                    RepairAction {
                        asset: Some(gap.asset.clone()),
                        technique: Some(gap.technique.clone()),
                        capability_id,
                        command_hint: tool.as_deref().map(|tool| {
                            command_hint_for(ctx.stage, tool, &gap.asset, &gap.technique)
                        }),
                        tool,
                        expected_status: Some(expected_status_for(ctx.stage, &gap.technique)),
                        evidence_refs: Vec::new(),
                        note: note_for(ctx.stage, &gap.technique),
                        reason: gap.reason.clone(),
                    }
                })
                .collect()
        }
        RepairKind::Generic => ctx
            .reasons
            .iter()
            .take(8)
            .map(|reason| RepairAction {
                asset: None,
                technique: None,
                capability_id: None,
                tool: Some("submit_stage_deliverable".to_string()),
                command_hint: None,
                expected_status: Some("needs_fix_resolved".to_string()),
                evidence_refs: Vec::new(),
                note: None,
                reason: reason.clone(),
            })
            .collect(),
    }
}

fn submit_guidance_for(
    repair_kind: RepairKind,
    actions: &[RepairAction],
    evidence_ids: &[i64],
) -> SubmitGuidance {
    let mode = match repair_kind {
        RepairKind::EvidenceRefs => SubmitGuidanceMode::RebuildEvidenceRefs,
        RepairKind::BackgroundJobs => SubmitGuidanceMode::WaitForBackgroundJobs,
        RepairKind::CoverageGap => SubmitGuidanceMode::FillCoverageCells,
        RepairKind::GateBlock => SubmitGuidanceMode::ResubmitAfterGateBlock,
        RepairKind::Generic => SubmitGuidanceMode::GenericRepair,
    };
    SubmitGuidance {
        mode,
        required_coverage_cells: actions
            .iter()
            .filter_map(|action| {
                Some(CoverageCellDraft {
                    asset: action.asset.clone()?,
                    technique: action.technique.clone()?,
                    expected_status: action
                        .expected_status
                        .clone()
                        .unwrap_or_else(|| "terminal".to_string()),
                    note: action.note.clone(),
                })
            })
            .collect(),
        required_claims: Vec::new(),
        top_level_evidence_refs: evidence_ids.to_vec(),
    }
}

fn allowed_tools_for(stage: StageKind, repair_kind: RepairKind) -> Vec<String> {
    match repair_kind {
        RepairKind::EvidenceRefs => vec![
            "query_target_data",
            "wait_for_background_jobs",
            "submit_stage_deliverable",
        ],
        RepairKind::BackgroundJobs => vec![
            "wait_for_background_jobs",
            "check_job",
            "kill_job",
            "query_target_data",
            "submit_stage_deliverable",
        ],
        RepairKind::CoverageGap | RepairKind::GateBlock => match stage {
            StageKind::TargetIntel => vec![
                "query_target_data",
                "check_stage_asset_coverage",
                "recon_map_assets",
                "recon_lookup_whois",
                "wait_for_background_jobs",
                "submit_stage_deliverable",
            ],
            StageKind::ExternalAttackSurface => vec![
                "stage_worklist_status",
                "stage_worklist_next",
                "list_recent_evidence",
                "query_target_data",
                "check_stage_asset_coverage",
                "eas_probe_http_liveness",
                "eas_discover_ports",
                "eas_fingerprint_services",
                "eas_fingerprint_web_stack",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
            StageKind::Enumeration => vec![
                "list_enumeration_web_roots",
                "stage_worklist_status",
                "stage_worklist_next",
                "list_recent_evidence",
                "enum_crawl_same_origin_urls",
                "browser_collect_js_api",
                "js_extract_apis",
                "route_probe_paths",
                "query_target_data",
                "check_stage_asset_coverage",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
                "submit_stage_deliverable",
            ],
            StageKind::VulnTriage => vec![
                "stage_worklist_status",
                "stage_worklist_next",
                "list_recent_evidence",
                "vuln_nuclei_general",
                "vuln_nuclei_fingerprint_targeted",
                "vuln_probe_anonymous_access",
                "query_target_data",
                "check_stage_asset_coverage",
                "submit_stage_deliverable",
            ],
            _ => stage_allowed_tools(stage),
        },
        RepairKind::Generic => vec![
            "query_target_data",
            "check_stage_asset_coverage",
            "wait_for_background_jobs",
            "submit_stage_deliverable",
        ],
    }
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn forbidden_tools_for(stage: StageKind, repair_kind: RepairKind) -> Vec<String> {
    let mut out = Vec::new();
    if matches!(repair_kind, RepairKind::CoverageGap | RepairKind::GateBlock) {
        out.extend([
            "list_in_scope_targets",
            "list_attack_surface_seeds",
            "manage_targets",
            "manage_organizations",
        ]);
    }
    if matches!(stage, StageKind::TargetIntel) {
        out.extend(["pentest_run", "run_pty_cmd", "run_command"]);
    }
    if matches!(stage, StageKind::VulnTriage) {
        out.extend(["nuclei", "pentest_run", "run_pty_cmd", "run_command"]);
    }
    out.into_iter().map(str::to_string).collect()
}

fn stage_allowed_tools(stage: StageKind) -> Vec<&'static str> {
    load_embedded_stage_spec(stage)
        .ok()
        .map(|spec| crate::harness::allowed_tool_names(&spec.allowed_tool_types))
        .unwrap_or_default()
}

fn suggested_tool_for(stage: StageKind, technique: &str) -> Option<String> {
    match (stage, technique) {
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-LIVENESS") => {
            Some("eas_probe_http_liveness".to_string())
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-PORT") => {
            Some("eas_discover_ports".to_string())
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-SERVICE-FINGERPRINT") => {
            Some("eas_fingerprint_services".to_string())
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-WEB-FINGERPRINT") => {
            Some("eas_fingerprint_web_stack".to_string())
        }
        (StageKind::Enumeration, "GOLISH-ENUM-JSAPI") => Some("browser_collect_js_api".to_string()),
        (StageKind::Enumeration, "GOLISH-ENUM-DIR") => Some("route_probe_paths".to_string()),
        (StageKind::Enumeration, "GOLISH-ENUM-PARAM") => Some("js_extract_apis".to_string()),
        (StageKind::TargetIntel, "GOLISH-INTEL-WHOIS") => Some("recon_lookup_whois".to_string()),
        (StageKind::TargetIntel, _) => Some("recon_map_assets".to_string()),
        (StageKind::VulnTriage, "GOLISH-NDAY") => {
            Some("vuln_nuclei_fingerprint_targeted".to_string())
        }
        (StageKind::VulnTriage, "WSTG-ATHN-04") => Some("vuln_probe_anonymous_access".to_string()),
        (StageKind::VulnTriage, technique) if technique.starts_with("WSTG-") => {
            Some("vuln_nuclei_general".to_string())
        }
        _ => None,
    }
}

fn command_hint_for(stage: StageKind, tool: &str, asset: &str, technique: &str) -> String {
    match (stage, tool, technique) {
        (StageKind::ExternalAttackSurface, "eas_probe_http_liveness", _) => format!(
            "eas_probe_http_liveness wrapper: include {asset} in targets[] only when it is a domain, URL, or confirmed web-origin seed; concrete IP/CIDR liveness must be closed through eas_discover_ports first"
        ),
        (StageKind::ExternalAttackSurface, "eas_discover_ports", _) => format!(
            "eas_discover_ports wrapper: include {asset} in targets[] only when it is a concrete IP/CIDR needing PORT coverage and use scan_profile=full; quick/standard remain partial, while the backend-owned full recipe writes terminal port outcomes only after its complete target manifest lands"
        ),
        (StageKind::ExternalAttackSurface, "eas_fingerprint_services", _) => {
            format!(
                "eas_fingerprint_services wrapper: include {asset} in targets[] only after confirmed open ports exist; normally omit ports[] because the backend selects pending ports per IP, isolates slow chunks, and performs one bounded recovery pass"
            )
        }
        (StageKind::ExternalAttackSurface, "eas_fingerprint_web_stack", _) => format!(
            "eas_fingerprint_web_stack wrapper: include {asset} in target_urls[] only for GOLISH-EAS-WEB-FINGERPRINT after the origin is confirmed HTTP(S); use absolute scheme://host[:port] URLs and run once per confirmed Host/SNI origin"
        ),
        (StageKind::ExternalAttackSurface, "httpx", _) => format!(
            "httpx batch: include {asset} with sibling domain/URL LIVENESS gaps in one JSONL run; use args `-json -sc -title -td -server -silent` plus pentest_run.input_lines. For concrete IP/CIDR liveness, prefer PORT/naabu first because port evidence closes IP liveness."
        ),
        (StageKind::ExternalAttackSurface, "naabu", _) => format!(
            "Use eas_discover_ports(targets=[{asset}], scan_profile=full) instead of raw naabu; scanner, port range, rate, retries and timeout are server-owned, and quick/standard profiles remain partial"
        ),
        (StageKind::ExternalAttackSurface, "nmap", _) => {
            format!(
                "nmap batch: fingerprint {asset} only if it has confirmed open ports; group sibling SERVICE gaps by shared port set and use args `-Pn -sV -iL {{{{input_file}}}} -p <confirmed-open-ports> -T3 --open` plus pentest_run.input_lines. Every confirmed open port must be in the port set; rerun only newly discovered ports if the port list expands. Use whatweb only once per confirmed HTTP(S) web origin, not generic service gaps. Do not include unresolved/no-open-port assets in the nmap batch."
            )
        }
        (StageKind::ExternalAttackSurface, "whatweb", _) => format!(
            "whatweb batch: include {asset} only for GOLISH-EAS-WEB-FINGERPRINT after the origin is confirmed HTTP(S); use absolute scheme://host[:port] URLs and run once per confirmed Host/SNI origin"
        ),
        (StageKind::Enumeration, "browser_collect_js_api", _) => format!(
            "browser_collect_js_api direct call: target_url={asset}, crawl_mode=\"standard\", ai_assist=true; use a bounded recipe only for one same-mode JS closure follow-up"
        ),
        (StageKind::Enumeration, "route_probe_paths", _) => format!(
            "route_probe_paths direct call: base_url={asset}; use observed JS/API prefixes plus the small local wordlist when available, keep requests bounded, and avoid external directory tools by default"
        ),
        (StageKind::Enumeration, "js_extract_apis", _) => format!(
            "js_extract_apis direct call: use saved JS/browser observations for {asset}; merge observed query keys, form field names, and targeted param_hints into api_endpoints.params"
        ),
        (StageKind::VulnTriage, "vuln_nuclei_general", _) => format!(
            "vuln_nuclei_general foreground call: resolve the work item's target_id and exact target_url={asset}, then pass techniques=[\"{technique}\"]; the backend owns the safe Nuclei profile and lands evidence before technique_outcomes"
        ),
        (StageKind::VulnTriage, "vuln_nuclei_fingerprint_targeted", _) => format!(
            "vuln_nuclei_fingerprint_targeted foreground call: resolve the work item's target_id and exact target_url={asset}, then pass techniques=[\"GOLISH-NDAY\"]; the backend freezes template ids from current-owner fingerprints and never falls back to a general scan"
        ),
        (StageKind::VulnTriage, "vuln_probe_anonymous_access", _) => format!(
            "vuln_probe_anonymous_access foreground call: resolve the work item's server-side target_id and exact target_url={asset}, query_target_data sections=[\"endpoints\"], review the complete potentially-sensitive inventory, then pass reviewed_endpoint_ids=[every eligible id] plus a bounded selected_probes=[{{endpoint_id, query_values, rationale}}] subset (maximum 16); never pass per-endpoint URL/method/header/cookie/token/body/redirect/CLI controls or blindly probe the full inventory"
        ),
        (StageKind::TargetIntel, "recon_lookup_whois", _) => {
            "recon_lookup_whois(organization_id=<current org>)".to_string()
        }
        (StageKind::TargetIntel, "recon_map_assets", _) => {
            "recon_map_assets(organization_id=<current org>)".to_string()
        }
        _ => format!("{tool} targeted at {asset} for {technique}"),
    }
}

fn expected_status_for(stage: StageKind, technique: &str) -> String {
    match (stage, technique) {
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-LIVENESS") => {
            "found if live; otherwise not_applicable with probe note".to_string()
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-PORT") => {
            "found if open ports; otherwise not_applicable with no-open-port note".to_string()
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-SERVICE-FINGERPRINT") => {
            "found with tested_units=total_units for open ports; not_applicable if no open ports"
                .to_string()
        }
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-WEB-FINGERPRINT") => {
            "found when WhatWeb fingerprints the confirmed web origin; checked_empty if it ran and found no stack".to_string()
        }
        _ => "terminal coverage status".to_string(),
    }
}

fn note_for(stage: StageKind, technique: &str) -> Option<String> {
    match (stage, technique) {
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-LIVENESS") => Some(
            "For concrete IP/CIDR assets, close liveness through port discovery first. For domain/URL assets, use httpx and terminalize only with real probe evidence."
                .to_string(),
        ),
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-SERVICE-FINGERPRINT") => Some(
            "For every discovered open port, set tested_units=total_units after fingerprinting; if new ports appear, fingerprint the new ports too. If there are no open ports, use not_applicable with note."
                .to_string(),
        ),
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-WEB-FINGERPRINT") => Some(
            "Run WhatWeb once per confirmed HTTP(S) origin. Do not use a WhatWeb result for one domain to cover another Host/SNI origin on the same IP:port."
                .to_string(),
        ),
        _ => None,
    }
}

fn normalized_tool_hint(tool: &str) -> Option<String> {
    let token = tool.split_whitespace().next()?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn normalized_stage_tool_hint(stage: StageKind, technique: &str, tool: &str) -> Option<String> {
    let token = normalized_tool_hint(tool)?;
    if stage == StageKind::VulnTriage {
        return match (technique, token.as_str()) {
            ("GOLISH-NDAY", "vuln_nuclei_fingerprint_targeted") => Some(token.clone()),
            ("WSTG-ATHN-04", "vuln_probe_anonymous_access") => Some(token.clone()),
            (technique, "vuln_nuclei_general") if technique.starts_with("WSTG-") => {
                if technique == "WSTG-ATHN-04" {
                    Some("vuln_probe_anonymous_access".to_string())
                } else {
                    Some(token.clone())
                }
            }
            ("GOLISH-NDAY", "nuclei") => Some("vuln_nuclei_fingerprint_targeted".to_string()),
            (technique, "nuclei") if technique.starts_with("WSTG-") => {
                suggested_tool_for(stage, technique)
            }
            _ => suggested_tool_for(stage, technique),
        };
    }
    if stage != StageKind::ExternalAttackSurface {
        return Some(token);
    }
    let mapped = match token.as_str() {
        "httpx" => "eas_probe_http_liveness",
        "naabu" | "masscan" => "eas_discover_ports",
        "nmap" => match technique {
            "GOLISH-EAS-PORT" => "eas_discover_ports",
            _ => "eas_fingerprint_services",
        },
        "whatweb" | "wappalyzer" => "eas_fingerprint_web_stack",
        other => other,
    };
    Some(mapped.to_string())
}

fn normalized_action_tools(action: &RepairAction) -> Vec<String> {
    let mut tools: Vec<String> = action
        .tool
        .iter()
        .filter_map(|tool| normalized_tool_hint(tool))
        .chain(
            action
                .command_hint
                .iter()
                .filter_map(|hint| normalized_tool_hint(hint)),
        )
        .collect();
    tools.sort();
    tools.dedup();
    tools
}

fn action_capability_suggestions(
    action: &RepairAction,
) -> Vec<golish_sub_agents::StageCapabilitySuggestion> {
    let Some(technique) = action.technique.as_deref() else {
        return Vec::new();
    };
    let Some(stage) = crate::harness::stage_for_technique(technique) else {
        return Vec::new();
    };
    suggested_capabilities_for_technique(stage, technique)
        .into_iter()
        .filter(|suggestion| {
            action
                .capability_id
                .as_deref()
                .map(|id| id == suggestion.id)
                .unwrap_or(true)
        })
        .map(|suggestion| golish_sub_agents::StageCapabilitySuggestion {
            id: suggestion.id,
            label: suggestion.label,
            tools: suggestion.tools,
            risk: suggestion.risk,
            batchable: suggestion.batchable,
            max_batch: suggestion.max_batch,
            reason: suggestion.reason,
        })
        .collect()
}

fn sample_assets(assets: &[String], total: usize) -> String {
    let mut sample: Vec<String> = assets
        .iter()
        .take(5)
        .map(|asset| bounded_model_field(asset, 512))
        .collect();
    if total > sample.len() {
        sample.push(format!(
            "# plus {} more; use stage_worklist_next",
            total - sample.len()
        ));
    }
    sample.join("\n")
}

fn reasons_contain(reasons: &[String], needles: &[&str]) -> bool {
    let joined = reasons.join(" | ").to_ascii_lowercase();
    needles.iter().any(|needle| joined.contains(needle))
}

fn reasons_look_like_coverage(reasons: &[String]) -> bool {
    reasons_contain(
        reasons,
        &[
            "coverage",
            "not_attempted",
            "never attempted",
            "missing terminal",
            "liveness",
            "service-fingerprint",
            "service fingerprint",
            "web-fingerprint",
            "web fingerprint",
            "stage gate",
        ],
    )
}

fn short_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn bounded_model_list(values: &[String], limit: usize, field_max_bytes: usize) -> Vec<String> {
    let mut sample = values
        .iter()
        .take(limit)
        .map(|value| bounded_model_field(value, field_max_bytes))
        .collect::<Vec<_>>();
    if values.len() > sample.len() {
        sample.push(format!("... +{} more", values.len() - sample.len()));
    }
    sample
}

fn bounded_model_field(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.replace(['\r', '\n'], " ");
    }
    const SUFFIX: &str = "...[truncated]";
    let prefix_budget = max_bytes.saturating_sub(SUFFIX.len());
    let end = utf8_boundary_at_or_before(value, prefix_budget);
    format!("{}{}", value[..end].replace(['\r', '\n'], " "), SUFFIX)
}

fn cap_recovery_model_text(mut value: String) -> String {
    if value.len() <= MODEL_RECOVERY_INSTRUCTION_MAX_BYTES {
        return value;
    }
    let prefix_budget =
        MODEL_RECOVERY_INSTRUCTION_MAX_BYTES.saturating_sub(MODEL_RECOVERY_TRUNCATION_SUFFIX.len());
    let end = utf8_boundary_at_or_before(&value, prefix_budget);
    value.truncate(end);
    value.push_str(MODEL_RECOVERY_TRUNCATION_SUFFIX);
    value
}

fn utf8_boundary_at_or_before(value: &str, max_bytes: usize) -> usize {
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn many_enumeration_gap_actions(count: usize) -> Vec<CoverageGapAction> {
        (0..count)
            .map(|idx| CoverageGapAction {
                asset: format!("https://asset-{idx:04}.example.test:443"),
                technique: "GOLISH-ENUM-DIR".to_string(),
                reason: format!("missing-terminal-gap-{idx:04}"),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["route_probe_paths".to_string()],
            })
            .collect()
    }

    #[test]
    fn recovery_projection_bounds_1176_actions_without_losing_internal_guard_data() {
        let directive = refine_gate_block(RefinerContext {
            stage: StageKind::Enumeration,
            org_id: None,
            agent_path: "main>stage_run:enumeration>org:o>enumerator".to_string(),
            reasons: vec!["enumeration coverage has pending cells".to_string()],
            coverage_gap_actions: many_enumeration_gap_actions(1_176),
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        assert_eq!(directive.actions.len(), 1_176);
        let gap_hash = directive.gap_hash.as_deref().expect("full gap hash");
        let first = directive.model_instruction();
        assert_eq!(
            first,
            directive.model_instruction(),
            "projection is byte-stable"
        );
        assert!(first.contains("total=1176"));
        assert!(first.contains(gap_hash));
        assert!(first.contains("stage_worklist_next"));
        assert!(first.contains("asset-0000.example.test"));
        assert!(first.contains("asset-0019.example.test"));
        assert!(!first.contains("asset-0020.example.test"));
        assert!(!first.contains("asset-1175.example.test"));
        assert!(first.len() <= 32 * 1024, "{} bytes", first.len());

        let mode = directive
            .to_submit_repair_mode()
            .expect("coverage gap maps to repair mode");
        assert_eq!(mode.coverage_gap_actions.len(), 1_176);
        let mode_instruction = mode.model_instruction();
        assert_eq!(mode_instruction, mode.model_instruction());
        assert_eq!(
            mode_instruction
                .matches("missing-terminal-gap-0000")
                .count(),
            1,
            "directive_message must not expand the same action list twice"
        );
        assert!(!mode_instruction.contains("asset-0020.example.test"));
        assert!(mode_instruction.len() <= 32 * 1024);
    }

    #[test]
    fn eas_coverage_gap_directive_converts_to_repair_lock() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::ExternalAttackSurface,
            org_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:o>prober".to_string(),
            reasons: vec!["external attack surface incomplete: never attempted".to_string()],
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "example.com".to_string(),
                technique: "GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["nmap".to_string()],
            }],
            available_evidence_ids: vec![42],
            running_background_jobs: Vec::new(),
        });

        assert_eq!(d.repair_kind, RepairKind::CoverageGap);
        assert_eq!(d.actions.len(), 1);
        assert!(d
            .forbidden_tools
            .contains(&"list_in_scope_targets".to_string()));
        let mode = d
            .to_submit_repair_mode()
            .expect("coverage maps to repair mode");
        assert_eq!(mode.kind, golish_sub_agents::SubmitRepairKind::CoverageGap);
        assert_eq!(mode.coverage_gap_actions.len(), 1);
        assert!(d
            .allowed_tools
            .contains(&"eas_fingerprint_services".to_string()));
        assert!(!d.allowed_tools.contains(&"pentest_run".to_string()));
        assert_eq!(
            mode.coverage_gap_actions[0].suggested_tools,
            vec!["eas_fingerprint_services".to_string()]
        );
        assert!(mode.model_instruction().contains("STAGE REFINER DIRECTIVE"));
    }

    #[test]
    fn submit_needs_fix_prioritizes_eas_coverage_gap_over_evidence_rewrite() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::ExternalAttackSurface,
            org_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:o>prober".to_string(),
            reasons: vec![
                "every claim must cite a real evidence id".to_string(),
                "external attack surface incomplete: GOLISH-EAS-SERVICE-FINGERPRINT never attempted".to_string(),
            ],
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "118.31.21.136".to_string(),
                technique: "GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["nmap -sV".to_string(), "whatweb".to_string()],
            }],
            available_evidence_ids: vec![14091],
            running_background_jobs: Vec::new(),
        });

        assert_eq!(d.repair_kind, RepairKind::CoverageGap);
        assert_eq!(
            d.actions[0].tool.as_deref(),
            Some("eas_fingerprint_services")
        );
        let hint = d.actions[0].command_hint.as_deref().unwrap();
        assert!(hint.starts_with("eas_fingerprint_services wrapper:"));
        assert!(hint.contains("confirmed-open port set"));
        let mode = d.to_submit_repair_mode().unwrap();
        assert!(mode.allows("eas_fingerprint_services"));
        assert!(!mode.allows("pentest_run"));
        assert_eq!(
            mode.coverage_gap_actions[0].suggested_tools,
            vec!["eas_fingerprint_services"]
        );
    }

    #[test]
    fn eas_web_fingerprint_repair_uses_wrapper_not_raw_whatweb() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::ExternalAttackSurface,
            org_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:o>prober".to_string(),
            reasons: vec![
                "external attack surface incomplete: GOLISH-EAS-WEB-FINGERPRINT never attempted"
                    .to_string(),
            ],
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "https://app.example.com".to_string(),
                technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["whatweb".to_string()],
            }],
            available_evidence_ids: vec![14092],
            running_background_jobs: Vec::new(),
        });

        assert_eq!(
            d.actions[0].tool.as_deref(),
            Some("eas_fingerprint_web_stack")
        );
        assert!(d
            .allowed_tools
            .contains(&"eas_fingerprint_web_stack".to_string()));
        assert!(!d.allowed_tools.contains(&"pentest_run".to_string()));
        let hint = d.actions[0].command_hint.as_deref().unwrap();
        assert!(hint.starts_with("eas_fingerprint_web_stack wrapper:"));
        let mode = d.to_submit_repair_mode().unwrap();
        assert!(mode
            .block_result_with_args(
                "eas_fingerprint_web_stack",
                &serde_json::json!({"target_urls": ["https://app.example.com"]})
            )
            .is_none());
        assert!(mode.block_result("whatweb").is_some());
        assert!(mode.block_result("pentest_run").is_some());
        assert_eq!(
            mode.coverage_gap_actions[0].suggested_tools,
            vec!["eas_fingerprint_web_stack"]
        );
    }

    #[test]
    fn target_intel_directive_forbids_scan_fallback() {
        let d = refine_gate_block(RefinerContext {
            stage: StageKind::TargetIntel,
            org_id: None,
            agent_path: "main>stage_run:target_intel>org:o>recon".to_string(),
            reasons: vec!["coverage incomplete: GOLISH-INTEL-WHOIS".to_string()],
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "example.com".to_string(),
                technique: "GOLISH-INTEL-WHOIS".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: Vec::new(),
            }],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        assert!(d.allowed_tools.contains(&"recon_lookup_whois".to_string()));
        assert!(d.forbidden_tools.contains(&"pentest_run".to_string()));
    }

    #[test]
    fn enumeration_coverage_gap_directive_preserves_worklist_refresh_tools() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::Enumeration,
            org_id: None,
            agent_path: "main>stage_run:enumeration>org:o>enumerator".to_string(),
            reasons: vec!["enumeration incomplete: never attempted".to_string()],
            coverage_gap_actions: vec![CoverageGapAction {
                asset: "https://app.example.com".to_string(),
                technique: "GOLISH-ENUM-JSAPI".to_string(),
                reason: "missing_terminal_coverage".to_string(),
                suggested_capabilities: Vec::new(),
                suggested_tools: vec!["browser_collect_js_api".to_string()],
            }],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        assert!(d
            .allowed_tools
            .contains(&"stage_worklist_status".to_string()));
        assert!(d.allowed_tools.contains(&"stage_worklist_next".to_string()));
        assert!(d
            .allowed_tools
            .contains(&"list_enumeration_web_roots".to_string()));
        assert!(d
            .allowed_tools
            .contains(&"list_recent_evidence".to_string()));
        assert!(d
            .allowed_tools
            .contains(&"enum_crawl_same_origin_urls".to_string()));
        assert!(!d.allowed_tools.contains(&"pentest_run".to_string()));
        assert!(!d.allowed_tools.contains(&"pentest_list_tools".to_string()));
        let instruction = d.model_instruction();
        assert!(instruction.contains("stage_worklist_status"));
        assert!(instruction.contains("stage_worklist_next"));

        let mode = d.to_submit_repair_mode().unwrap();
        assert!(mode.block_result("stage_worklist_status").is_none());
        assert!(mode.block_result("stage_worklist_next").is_none());
        assert!(mode.block_result("list_enumeration_web_roots").is_none());
        assert!(mode.block_result("list_recent_evidence").is_none());
        assert!(mode.block_result("pentest_run").is_some());
        assert!(mode
            .block_result_with_args(
                "enum_crawl_same_origin_urls",
                &serde_json::json!({"target_urls": ["https://app.example.com"]})
            )
            .is_none());
        let blocked = mode
            .block_result_with_args(
                "browser_collect_js_api",
                &serde_json::json!({"target_url": "https://package.moresec.cn"}),
            )
            .expect("off-action direct enumeration target remains blocked");
        assert!(blocked["blocked_reason"]
            .as_str()
            .unwrap()
            .contains("not in coverage_gap_actions"));
    }

    #[test]
    fn vuln_coverage_gap_directive_selects_each_exact_formulaic_wrapper() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::VulnTriage,
            org_id: None,
            agent_path: "main>stage_run:vuln_triage>org:o>vuln_scanner".to_string(),
            reasons: vec!["vuln_triage incomplete: never attempted".to_string()],
            coverage_gap_actions: vec![
                CoverageGapAction {
                    asset: "https://app.example.com".to_string(),
                    technique: "WSTG-INPV-05".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: Vec::new(),
                },
                CoverageGapAction {
                    asset: "https://cms.example.com".to_string(),
                    technique: "GOLISH-NDAY".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: Vec::new(),
                },
                CoverageGapAction {
                    asset: "https://api.example.com".to_string(),
                    technique: "WSTG-ATHN-04".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: Vec::new(),
                },
            ],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        assert_eq!(d.repair_kind, RepairKind::CoverageGap);
        assert!(d.allowed_tools.contains(&"vuln_nuclei_general".to_string()));
        assert!(d
            .allowed_tools
            .contains(&"vuln_nuclei_fingerprint_targeted".to_string()));
        assert!(d
            .allowed_tools
            .contains(&"vuln_probe_anonymous_access".to_string()));
        assert!(!d.allowed_tools.contains(&"pentest_run".to_string()));
        let anonymous_hint = d.actions[2]
            .command_hint
            .as_deref()
            .expect("anonymous-access repair hint");
        assert!(anonymous_hint.contains("reviewed_endpoint_ids"));
        assert!(anonymous_hint.contains("selected_probes"));
        assert!(anonymous_hint.contains("query_values"));
        let mode = d.to_submit_repair_mode().unwrap();
        assert!(mode.allows("vuln_nuclei_general"));
        assert!(mode.allows("vuln_nuclei_fingerprint_targeted"));
        assert!(mode.allows("vuln_probe_anonymous_access"));
        assert!(!mode.allows("pentest_run"));
        assert_eq!(
            mode.coverage_gap_actions[0].suggested_tools,
            vec!["vuln_nuclei_general".to_string()]
        );
        assert_eq!(
            mode.coverage_gap_actions[1].suggested_tools,
            vec!["vuln_nuclei_fingerprint_targeted".to_string()]
        );
        assert_eq!(
            mode.coverage_gap_actions[2].suggested_tools,
            vec!["vuln_probe_anonymous_access".to_string()]
        );
    }

    #[test]
    fn eas_coverage_gap_instruction_is_batch_first() {
        let d = refine_submit_needs_fix(RefinerContext {
            stage: StageKind::ExternalAttackSurface,
            org_id: None,
            agent_path: "main>stage_run:external_attack_surface>org:o>prober".to_string(),
            reasons: vec!["external attack surface incomplete: never attempted".to_string()],
            coverage_gap_actions: vec![
                CoverageGapAction {
                    asset: "a.example.com".to_string(),
                    technique: "GOLISH-EAS-LIVENESS".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["httpx".to_string()],
                },
                CoverageGapAction {
                    asset: "b.example.com".to_string(),
                    technique: "GOLISH-EAS-LIVENESS".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["httpx".to_string()],
                },
                CoverageGapAction {
                    asset: "c.example.com".to_string(),
                    technique: "GOLISH-EAS-PORT".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["naabu".to_string()],
                },
                CoverageGapAction {
                    asset: "d.example.com".to_string(),
                    technique: "GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["nmap".to_string()],
                },
                CoverageGapAction {
                    asset: "https://a.example.com".to_string(),
                    technique: "GOLISH-EAS-WEB-FINGERPRINT".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_capabilities: Vec::new(),
                    suggested_tools: vec!["whatweb".to_string()],
                },
            ],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        let instruction = d.model_instruction();
        assert!(instruction.contains("EAS repair is batch-first"));
        assert!(instruction.contains("eas_probe_http_liveness"));
        assert!(instruction.contains("eas_discover_ports"));
        assert!(instruction.contains("scan_profile=full"));
        assert!(instruction.contains("quick/standard remain partial"));
        assert!(instruction.contains("eas_fingerprint_services"));
        assert!(instruction.contains("WEB-FINGERPRINT/eas_fingerprint_web_stack"));
        assert!(!instruction.contains("tool_name=httpx"));
        assert!(!instruction.contains("{{input_file}}"));
        assert!(!instruction.contains("input_lines"));
        assert!(instruction.contains("a.example.com"));
        assert!(instruction.contains("b.example.com"));
        assert!(d.actions[0]
            .command_hint
            .as_deref()
            .unwrap()
            .starts_with("eas_probe_http_liveness wrapper:"));
        assert!(d.actions[2]
            .command_hint
            .as_deref()
            .unwrap()
            .starts_with("eas_discover_ports wrapper:"));
        assert!(d.actions[3]
            .command_hint
            .as_deref()
            .unwrap()
            .starts_with("eas_fingerprint_services wrapper:"));
        assert!(instruction.contains("normally omit ports[]"));
        assert!(instruction.contains("isolates slow targets"));
        assert!(instruction.contains("Do not group by shared ports, increase timeouts"));
        assert!(instruction.contains("Do not include unresolved hosts"));
        assert!(instruction.contains("run once per confirmed Host/SNI origin"));
        assert!(d.actions[3]
            .command_hint
            .as_deref()
            .unwrap()
            .contains("confirmed open ports"));
    }
}
