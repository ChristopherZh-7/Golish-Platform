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

use crate::harness::{load_embedded_stage_spec, CoverageGapAction, StageKind};

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
            self.root_cause, self.stage, self.repair_kind
        );
        if !self.allowed_tools.is_empty() {
            out.push_str(&format!(
                "\nAllowed next tools: [{}].",
                self.allowed_tools.join(", ")
            ));
        }
        if !self.forbidden_tools.is_empty() {
            out.push_str(&format!(
                "\nForbidden in this repair: [{}].",
                self.forbidden_tools.join(", ")
            ));
        }
        if let Some(batch_hint) = self.eas_batch_instruction() {
            out.push_str(&batch_hint);
        }
        if !self.actions.is_empty() {
            out.push_str("\nActions:");
            for (idx, action) in self.actions.iter().enumerate() {
                let mut line = format!(" {}. {}", idx + 1, action.reason);
                if let Some(asset) = action.asset.as_deref() {
                    line.push_str(&format!(" asset={asset}"));
                }
                if let Some(technique) = action.technique.as_deref() {
                    line.push_str(&format!(" technique={technique}"));
                }
                if let Some(tool) = action.tool.as_deref() {
                    line.push_str(&format!(" tool={tool}"));
                }
                if let Some(status) = action.expected_status.as_deref() {
                    line.push_str(&format!(" submit_status={status}"));
                }
                if let Some(hint) = action.command_hint.as_deref() {
                    line.push_str(&format!(" hint={hint}"));
                }
                out.push('\n');
                out.push_str(&line);
            }
        }
        out.push_str("\nThen call submit_stage_deliverable once with terminal coverage/claims that cite real evidence ids when required.");
        out
    }

    fn eas_batch_instruction(&self) -> Option<String> {
        if self.stage != StageKind::ExternalAttackSurface.as_str() {
            return None;
        }
        if !matches!(
            self.repair_kind,
            RepairKind::CoverageGap | RepairKind::GateBlock
        ) {
            return None;
        }

        let assets_for = |technique: &str| -> Vec<String> {
            self.actions
                .iter()
                .filter(|action| action.technique.as_deref() == Some(technique))
                .filter_map(|action| action.asset.clone())
                .collect()
        };
        let liveness = assets_for("GOLISH-EAS-LIVENESS");
        let ports = assets_for("GOLISH-EAS-PORT");
        let services = assets_for("GOLISH-EAS-SERVICE-FINGERPRINT");
        if liveness.is_empty() && ports.is_empty() && services.is_empty() {
            return None;
        }

        let mut out = String::from(
            "\nBatching: EAS repair is batch-first. Group sibling gap actions by \
             technique and use as few pentest_run calls as possible; do not run \
             one foreground tool call per asset when httpx/naabu/masscan/nmap/whatweb/gowitness \
             can consume batch input. For stdin-capable tools, pass newline-separated targets \
             through `input_lines` or `stdin`; for list-file tools, put `{{input_file}}` in \
             `args` and pass the actual targets through `input_lines`.",
        );
        if !liveness.is_empty() {
            out.push_str(&format!(
                "\n- LIVENESS/httpx: one pentest_run tool_name=httpx with args like \
                 `-json -sc -title -td -server -silent` and input_lines like:\n{}",
                sample_assets(&liveness)
            ));
        }
        if !ports.is_empty() {
            out.push_str(&format!(
                "\n- PORT/naabu: one pentest_run tool_name=naabu with args like \
                 `-list {{{{input_file}}}} -top-ports 1000 -s c -silent` and input_lines like:\n{}",
                sample_assets(&ports)
            ));
        }
        if !services.is_empty() {
            out.push_str(&format!(
                "\n- SERVICE/nmap: group hosts that share the same open-port set \
                 and run one pentest_run tool_name=nmap with args like \
                 `-sV -iL {{{{input_file}}}} -p <confirmed-open-ports> -T3 --open` per group. \
                 Do not include unresolved hosts or assets with no confirmed open ports; close \
                 those cells as not_applicable/blocked with a concrete note instead. Sample targets:\n{}",
                sample_assets(&services)
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
                        suggested_tools: action
                            .tool
                            .iter()
                            .cloned()
                            .chain(action.command_hint.iter().filter_map(|hint| {
                                hint.split_whitespace().next().map(str::to_string)
                            }))
                            .collect(),
                    })
                })
                .collect(),
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
    } else if reasons_contain(
        &ctx.reasons,
        &["evidence_ref", "evidence id", "fabricated", "real evidence"],
    ) {
        RepairKind::EvidenceRefs
    } else if !ctx.coverage_gap_actions.is_empty() || reasons_look_like_coverage(&ctx.reasons) {
        RepairKind::CoverageGap
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
                    let tool = gap
                        .suggested_tools
                        .first()
                        .cloned()
                        .or_else(|| suggested_tool_for(ctx.stage, &gap.technique));
                    RepairAction {
                        asset: Some(gap.asset.clone()),
                        technique: Some(gap.technique.clone()),
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
            StageKind::ExternalAttackSurface | StageKind::Enumeration => vec![
                "pentest_list_tools",
                "pentest_run",
                "query_target_data",
                "check_stage_asset_coverage",
                "wait_for_background_jobs",
                "check_job",
                "kill_job",
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
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-LIVENESS") => Some("httpx".to_string()),
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-PORT") => Some("naabu".to_string()),
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-SERVICE-FINGERPRINT") => {
            Some("nmap".to_string())
        }
        (StageKind::TargetIntel, "GOLISH-INTEL-WHOIS") => Some("recon_lookup_whois".to_string()),
        (StageKind::TargetIntel, _) => Some("recon_map_assets".to_string()),
        _ => None,
    }
}

fn command_hint_for(stage: StageKind, tool: &str, asset: &str, technique: &str) -> String {
    match (stage, tool, technique) {
        (StageKind::ExternalAttackSurface, "httpx", _) => format!(
            "httpx batch: include {asset} with sibling LIVENESS gaps in one JSONL run; use args `-json -sc -title -td -server -silent` plus pentest_run.input_lines"
        ),
        (StageKind::ExternalAttackSurface, "naabu", _) => format!(
            "naabu batch: include {asset} with sibling PORT gaps in one pentest_run; use args `-list {{{{input_file}}}} -top-ports 1000 -s c -silent` plus pentest_run.input_lines"
        ),
        (StageKind::ExternalAttackSurface, "nmap", _) => {
            format!(
                "nmap batch: fingerprint {asset} only if it has confirmed open ports; group sibling SERVICE gaps by shared port set and use args `-sV -iL {{{{input_file}}}} -p <confirmed-open-ports> -T3 --open` plus pentest_run.input_lines. Do not include unresolved/no-open-port assets in the nmap batch."
            )
        }
        (StageKind::ExternalAttackSurface, "whatweb", _) => format!(
            "whatweb batch: include {asset} with sibling HTTP(S) services when Ruby is ready; otherwise prefer nmap -sV/httpx evidence"
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
        _ => "terminal coverage status".to_string(),
    }
}

fn note_for(stage: StageKind, technique: &str) -> Option<String> {
    match (stage, technique) {
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-LIVENESS") => Some(
            "If the host cannot resolve or has no reachable endpoint after the targeted probe, use not_applicable with the probe failure note; do not loop on checked_empty without evidence."
                .to_string(),
        ),
        (StageKind::ExternalAttackSurface, "GOLISH-EAS-SERVICE-FINGERPRINT") => Some(
            "For every discovered open port, set tested_units=total_units after fingerprinting; if there are no open ports, use not_applicable with note."
                .to_string(),
        ),
        _ => None,
    }
}

fn sample_assets(assets: &[String]) -> String {
    let mut sample: Vec<String> = assets.iter().take(5).cloned().collect();
    if assets.len() > sample.len() {
        sample.push(format!("# plus {} more", assets.len() - sample.len()));
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
            "stage gate",
        ],
    )
}

fn short_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(mode.model_instruction().contains("STAGE REFINER DIRECTIVE"));
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
                suggested_tools: Vec::new(),
            }],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        assert!(d.allowed_tools.contains(&"recon_lookup_whois".to_string()));
        assert!(d.forbidden_tools.contains(&"pentest_run".to_string()));
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
                    suggested_tools: vec!["httpx".to_string()],
                },
                CoverageGapAction {
                    asset: "b.example.com".to_string(),
                    technique: "GOLISH-EAS-LIVENESS".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_tools: vec!["httpx".to_string()],
                },
                CoverageGapAction {
                    asset: "c.example.com".to_string(),
                    technique: "GOLISH-EAS-PORT".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_tools: vec!["naabu".to_string()],
                },
                CoverageGapAction {
                    asset: "d.example.com".to_string(),
                    technique: "GOLISH-EAS-SERVICE-FINGERPRINT".to_string(),
                    reason: "missing_terminal_coverage".to_string(),
                    suggested_tools: vec!["nmap".to_string()],
                },
            ],
            available_evidence_ids: Vec::new(),
            running_background_jobs: Vec::new(),
        });

        let instruction = d.model_instruction();
        assert!(instruction.contains("EAS repair is batch-first"));
        assert!(instruction.contains("tool_name=httpx"));
        assert!(instruction.contains("tool_name=naabu"));
        assert!(instruction.contains("tool_name=nmap"));
        assert!(instruction.contains("{{input_file}}"));
        assert!(instruction.contains("input_lines"));
        assert!(instruction.contains("a.example.com"));
        assert!(instruction.contains("b.example.com"));
        assert!(d.actions[0]
            .command_hint
            .as_deref()
            .unwrap()
            .starts_with("httpx batch:"));
        assert!(d.actions[2]
            .command_hint
            .as_deref()
            .unwrap()
            .contains("-list {{input_file}}"));
        assert!(d.actions[3]
            .command_hint
            .as_deref()
            .unwrap()
            .contains("-iL {{input_file}}"));
        assert!(instruction.contains("<confirmed-open-ports>"));
        assert!(instruction.contains("Do not include unresolved hosts"));
        assert!(d.actions[3]
            .command_hint
            .as_deref()
            .unwrap()
            .contains("confirmed open ports"));
    }
}
