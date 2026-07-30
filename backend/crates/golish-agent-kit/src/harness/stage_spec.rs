//! StageSpec DTO + JSON loader (Doc 3 §4).
//!
//! Phase 1c MVP: 仅 external_attack_surface stage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1;

use super::types::{AgentContinuity, FindingSeverity, RiskLevel, StageKind};
use super::StageRuntimeContract;

/// P2 · per-stage "trustworthy conclusion" rule (verification gate).
///
/// Findings at/above `min_severity` must carry evidence: non-empty
/// `evidence_refs` (deliverable structural layer) and — when
/// `require_evidence_kinds` is set — at least one of those evidence rows must be
/// of a listed kind (ledger layer, enforced caller-side). Declarative: you set
/// this per stage in the stage JSON to define what "verified" means there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingVerificationRule {
    pub min_severity: FindingSeverity,
    #[serde(default)]
    pub require_evidence_kinds: Vec<String>,
}

/// Doc 3 §4.1 human_approval policy 嵌入字段.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HumanApprovalPolicy {
    #[serde(default)]
    pub required_before: Vec<String>,
}

/// Doc 3 §9.2 carry_over 白名单条目.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InheritsEvidenceFrom {
    pub stage_kind: StageKind,
    pub evidence_kinds: Vec<String>,
}

/// C4/P6a Post-Exploit execution boundary. This is descriptive input to the
/// capability/router checks; database and app-layer policy remain the final
/// authority. Side-effect classes are closed strings so an unknown future
/// class fails stage-spec contract tests before becoming routable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostExploitPolicy {
    pub p6a_only: bool,
    #[serde(default)]
    pub allowed_side_effect_classes: Vec<String>,
    pub requires_cleanup_kernel_for_side_effects: bool,
    pub work_item_kind: String,
    #[serde(default)]
    pub canonical_rows: Vec<String>,
}

fn deserialize_nonzero_u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    if value == 0 {
        return Err(serde::de::Error::custom("value must be non-zero"));
    }
    Ok(value)
}

/// Company Controller scheduler policy. When present, one company-scoped Lead
/// owns coordination and final submission while any later children are
/// server-validated durable sibling WorkItems. C/G/K are required non-zero
/// limits: company controllers, global provider calls, and per-company live
/// workers respectively.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageTeamSchedulerPolicy {
    pub schema_version: u32,
    #[serde(default)]
    pub enabled_in_v2_only: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
    pub aggregator_kind: String,
    pub aggregator_role: String,
    /// Maximum number of company-scoped Unit controllers that the outer
    /// `stage_run` queue may keep active (C).
    #[serde(deserialize_with = "deserialize_nonzero_u32")]
    pub max_company_units_active: u32,
    /// Global ceiling for concurrent provider calls across all active company
    /// controllers and their children (G).
    #[serde(deserialize_with = "deserialize_nonzero_u32")]
    pub global_provider_cap: u32,
    /// Per-company active WorkerRun cap. In company-controller mode this
    /// includes the Lead itself as well as its active children (K).
    #[serde(deserialize_with = "deserialize_nonzero_u32")]
    pub max_workers: u32,
    /// Frozen compatibility metadata for existing TeamPlan hashes. Company
    /// Controller admission does not enforce this lifetime count; exact scope,
    /// request epoch, replay, live K and per-WorkItem attempts are authoritative.
    #[serde(default)]
    pub max_dynamic_requests: u32,
    /// Closed server allowlist for model-proposed sibling WorkItem kinds.
    #[serde(default)]
    pub allowed_dynamic_request_kinds: Vec<String>,
    /// Maximum number of canonical, same-scope subject refs per request.
    #[serde(default)]
    pub max_dynamic_subject_refs: u32,
    #[serde(default)]
    pub risk_lane: String,
}

/// Candidate-only two-wave reasoning policy. This authority is intentionally
/// separate from the general Company Controller scheduler and from the
/// Verification execution policy: it bounds read-only analysis over a frozen
/// registry snapshot, never tools or target interaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateAnalysisTeamPolicy {
    pub schema_version: u32,
    pub controller_role: String,
    pub analyst_role: String,
    pub critic_role: String,
    pub final_submitter_role: String,
    pub min_live_analysis_lanes: u32,
    pub max_live_analysis_lanes: u32,
    pub single_lane_input_limit: u32,
    pub max_inputs_per_microbatch: u32,
    pub chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub attack_class_checklist_contract_version: u32,
    pub trust_boundary_checklist_contract_version: u32,
    pub coverage_partition_contract_version: u32,
    pub coverage_synthesis_contract_version: u32,
    pub hypothesis_coverage_sampling_contract_version: u32,
    pub require_checked_tool_truth_temporal_authority: bool,
    pub knowledge_feed_snapshot_contract_version: u32,
    pub product_version_match_contract_version: u32,
    pub max_knowledge_feed_age_seconds: u64,
    pub require_signed_knowledge_feeds: bool,
    pub required_tool_truth_root_families: Vec<ToolTruthRootFamilyV1>,
    pub max_source_bytes_per_input: u64,
    pub max_chunk_bytes: u32,
    pub max_chunks_per_input: u32,
    pub max_chunks_per_coverage_partition: u32,
    pub max_coverage_subreview_work_items: u32,
    pub max_synthesis_inputs_per_partition: u32,
    pub max_proposals_per_artifact: u32,
    pub max_controller_page_size: u32,
    pub max_attempts_per_work_item: u32,
    pub max_analysis_attempts: u32,
    pub require_read_only_children: bool,
    pub require_tool_free_children: bool,
}

impl CandidateAnalysisTeamPolicy {
    fn validate(&self) -> Result<(), String> {
        let roles = [
            self.controller_role.as_str(),
            self.analyst_role.as_str(),
            self.critic_role.as_str(),
        ];
        if roles.iter().any(|role| role.trim().is_empty()) {
            return Err("Candidate analysis roles must be non-empty".to_string());
        }
        if self.final_submitter_role != self.controller_role
            || self.controller_role == self.analyst_role
            || self.controller_role == self.critic_role
            || self.analyst_role == self.critic_role
        {
            return Err(
                "Candidate Controller must be the unique final submitter and all roles must differ"
                    .to_string(),
            );
        }
        if self.min_live_analysis_lanes < 2
            || self.min_live_analysis_lanes > self.max_live_analysis_lanes
            || self.max_live_analysis_lanes > 8
            || self.single_lane_input_limit == 0
        {
            return Err("Candidate live-lane bounds must satisfy 2 <= min <= max <= 8 and the single-lane threshold must be non-zero".to_string());
        }

        let contract_versions = [
            self.schema_version,
            self.chunking_contract_version,
            self.redaction_contract_version,
            self.attack_class_checklist_contract_version,
            self.trust_boundary_checklist_contract_version,
            self.coverage_partition_contract_version,
            self.coverage_synthesis_contract_version,
            self.hypothesis_coverage_sampling_contract_version,
            self.knowledge_feed_snapshot_contract_version,
            self.product_version_match_contract_version,
        ];
        let u32_ceilings = [
            self.max_inputs_per_microbatch,
            self.max_chunk_bytes,
            self.max_chunks_per_input,
            self.max_chunks_per_coverage_partition,
            self.max_coverage_subreview_work_items,
            self.max_synthesis_inputs_per_partition,
            self.max_proposals_per_artifact,
            self.max_controller_page_size,
            self.max_attempts_per_work_item,
            self.max_analysis_attempts,
        ];
        if contract_versions.contains(&0)
            || u32_ceilings.contains(&0)
            || self.max_source_bytes_per_input == 0
            || self.max_knowledge_feed_age_seconds == 0
        {
            return Err(
                "Candidate contract versions and hard ceilings must be non-zero".to_string(),
            );
        }
        if !self.require_checked_tool_truth_temporal_authority
            || !self.require_signed_knowledge_feeds
            || !self.require_read_only_children
            || !self.require_tool_free_children
        {
            return Err("Candidate analysis must require checked temporal Tool Truth, signed feeds, and read-only tool-free children".to_string());
        }
        if self.required_tool_truth_root_families.as_slice() != ToolTruthRootFamilyV1::ALL {
            return Err(
                "Candidate analysis must require the exact Plan A TI/EAS/Enum/Vuln root set"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Doc 3 §4.1 StageSpec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSpec {
    pub id: String,
    pub kind: StageKind,
    pub risk_level: RiskLevel,

    #[serde(default)]
    pub requires_stages: Vec<StageKind>,
    /// Contract-aware dependency used only by an exact V2-only operation.
    /// Keeping it separate avoids changing legacy static graph semantics.
    #[serde(default)]
    pub v2_requires_stages: Vec<StageKind>,
    #[serde(default)]
    pub allowed_next_stages: Vec<StageKind>,

    /// Category-based stage tool whitelist (deny-by-default). Each entry is a
    /// **type selector**: a bare category (`"recon"`), a `category/subcategory`
    /// (`"recon/dns"`), or a specific tool name (`"nmap"`). The per-stage tool
    /// boundary is enforced from this list via [`super::tool_taxonomy::stage_allows`]
    /// (only for scan invocations; agent/meta tools are exempt). Empty = no scan
    /// tools permitted (e.g. scoping / reporting). See
    /// `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`.
    #[serde(default)]
    pub allowed_tool_types: Vec<String>,

    pub deliverable_schema: String,
    pub gate_validator: String,

    // gate-rules-migration (2026-06-05): 旧 `required_checks: Vec<String>` 固定菜单
    // 已删除；过关标准统一由下方 `gate_rules` 声明（数据积木 + named_check 逃生舱）。
    #[serde(default)]
    pub min_invocations: HashMap<String, u32>,

    /// Doc 3 §8.3 vacuous detector 上限 (Other-type skip 数).
    #[serde(default)]
    pub max_other_skips: Option<u32>,

    #[serde(default)]
    pub human_approval: Option<HumanApprovalPolicy>,

    #[serde(default = "default_continuity")]
    pub agent_continuity: AgentContinuity,

    #[serde(default)]
    pub inherits_evidence_from: Vec<InheritsEvidenceFrom>,

    /// Present only on the four post-exploit stages. C4 permits `none` only;
    /// C6 may expand this after the cleanup-obligation kernel is installed.
    #[serde(default)]
    pub post_exploit_policy: Option<PostExploitPolicy>,

    /// Declarative durable owner/scope/lease/final-seal contract. Absent on
    /// legacy/root-only stages; present on the four Runtime Memory V2
    /// specialist stages. This field describes stage ownership only and never
    /// selects or upgrades an operation's persisted rollout contract.
    #[serde(default)]
    pub runtime_memory: Option<StageRuntimeContract>,

    // ── P2 · 配置驱动的「过关证据」声明（你填这里，gate 照执，零代码） ──────────
    /// P2 · 该 stage 交付物必须含的 evidence 种类（ledger 回查；空=不强制）。
    /// 例：信息收集阶段填 ["dns_a","http_probe","subdomain"] 表示要有这些证据才过。
    #[serde(default)]
    pub required_evidence_kinds: Vec<String>,

    /// P2 · finding 验证规则：达到阈值 severity 的 finding 必须有证据 / PoC。
    /// 例：verification 阶段填 {"min_severity":"high","require_evidence_kinds":["poc","exploit_verified"]}。
    #[serde(default)]
    pub finding_verification: Option<FindingVerificationRule>,

    /// P2 · 交付物最少 finding / claim 数（None=不强制）。
    #[serde(default)]
    pub min_findings: Option<u32>,
    #[serde(default)]
    pub min_claims: Option<u32>,

    /// P2 · 数据驱动 gate 规则（设计 2026-06-05）。每条规则用固定积木 op 声明一条
    /// 过关标准，由 `super::gate::rule_engine::eval` 执行。缺省空 = 行为与旧版逐字节一致。
    #[serde(default)]
    pub gate_rules: Vec<super::gate::rule_engine::GateRule>,

    /// Coverage matrix（设计 2026-06-05）：本 stage 期望覆盖的技术类清单，由
    /// `gate_rules` 的 `coverage_complete` op 读取，对每个（自报）资产核对是否每类
    /// 技术都有终态。缺省空 = `coverage_complete` 视为 no-op（向后兼容）。值约定为
    /// **OWASP WSTG / MITRE ATT&CK id**（"挂标准"）；MVP 暂不做词典校验，先用字符串
    /// （taxonomy 词典化 + 动态 skeleton 生成见设计 §6.5，待资产库合入后接）。
    #[serde(default)]
    pub expected_techniques: Vec<String>,

    // ── stage_run fan-out 配置（设计 2026-06-13-stage-run-fanout §3.2） ──────────
    /// The specialist sub-agent slug the `stage_run` tool fans out per org for
    /// this stage (intel → `recon`, EAS → `prober`, …). `None` = this stage has
    /// no per-org specialist (e.g. scoping / reporting), so `stage_run` does not
    /// apply. Config-driven so 12 stages share one mechanism with no new code.
    #[serde(default)]
    pub specialist: Option<String>,

    /// Optional durable sibling Team Scheduler policy. Absence deliberately
    /// means the stage remains on its existing single-worker scheduler.
    #[serde(default)]
    pub team_scheduler: Option<StageTeamSchedulerPolicy>,

    /// Independent Candidate registry analysis policy. It is legal only for
    /// `attack_candidate` and deliberately owns no Tool Truth TTL/skew fields.
    #[serde(default)]
    pub candidate_analysis_team: Option<CandidateAnalysisTeamPolicy>,

    /// Display-only coverage technique columns the `stage_run` view renders per
    /// org (intel → `["DNS","WHOIS","ASN","CT","SUBDOMAIN","OSINT"]`). Distinct
    /// from `expected_techniques` (the gate's registered ids): this is the
    /// human-readable axis shown on each org row. Empty = derive from
    /// `expected_techniques` at the call site.
    #[serde(default)]
    pub coverage_axis: Vec<String>,

    /// Facts-from-DB-truth opt-in (design 2026-06-15 §5 PR2). When true AND the gate
    /// is handed real DB/ledger evidence facts, `vacuous_check` treats the stage's
    /// facts as coming from DB truth (`coverage_complete` adjudicates), so an
    /// otherwise-empty deliverable is not "vacuous". Completeness is still enforced
    /// by `coverage_complete` (per in-scope asset × expected technique). Default
    /// false = byte-for-byte unchanged. Enable only for facts-only intel/recon stages.
    #[serde(default)]
    pub facts_from_db_truth: bool,

    /// Per-dimension freshness window (design 2026-06-22). When true, the gate's
    /// DB-truth org-intel projection (ASN/CT/WHOIS/OSINT) only counts a dimension
    /// as Found when its `organizations.<dim>_collected_at >= this stage-run start`
    /// (`operation_state.stage_started_at`), so a stale row left by a previous run
    /// can't satisfy the cell this run. Default false = presence-only (byte-for-byte
    /// unchanged); enable on facts-from-DB-truth intel stages together with the
    /// write-path `*_collected_at` stamping.
    #[serde(default)]
    pub freshness_window: bool,

    /// Stage expansion wave barrier (design 2026-06-28): when true, the current
    /// wave's coverage denominator is frozen to assets that already existed when
    /// the stage started. Assets discovered during the wave remain persisted as DB
    /// truth, but are held for a next-wave expansion check instead of moving the
    /// current gate target.
    #[serde(default)]
    pub asset_wave_barrier: bool,

    /// Host-aware coverage (design 2026-06-15-host-aware-coverage, Phase 2a):
    /// when true, `coverage_complete` holds each in-scope asset only to the
    /// techniques that apply to its class (a bare IP is not asked for
    /// SUBDOMAIN/DNS/CT). Default false = byte-for-byte unchanged; enable only
    /// on a stage with a green PASS/BLOCK parity test.
    #[serde(default)]
    pub host_aware_coverage: bool,

    /// Enumeration IP-web coverage (design 2026-07-01): when true, the gate/UI
    /// may inject EAS/httpx-proven IP/CIDR web services into the content
    /// enumeration denominator. Default false = old bare-IP behavior.
    #[serde(default)]
    pub enum_ip_web_coverage: bool,

    /// Dead-asset denominator exclusion (design 2026-07-02-dead-asset-liveness-
    /// state §5.2): when true, the coverage gate drops assets EAS confirmed dead
    /// (`targets.liveness_state = 'dead'`) from this stage's denominator, so a
    /// confirmed-dead host no longer forces the model to probe it or book
    /// `checked_empty`. Only downstream stages (enumeration onward) enable it —
    /// EAS itself must NOT (it is the stage that judges liveness, so filtering its
    /// own denominator would leave it nothing to probe). `'unreachable'` is never
    /// dropped (may be transient). Default false = byte-for-byte unchanged.
    #[serde(default)]
    pub skip_dead_assets: bool,

    /// Anchor-only coverage denominator (design 2026-06-16-coverage-anchor-axis):
    /// when true, `coverage_complete` first drops any in-scope asset that is a
    /// strict subdomain of ANOTHER in-scope asset in the same set, so subdomains
    /// passively discovered + registered as `scope='in'` during target_intel do
    /// not inflate the (asset × technique) denominator — the root's SUBDOMAIN cell
    /// already represents "subdomains were enumerated". The maximal roots always
    /// remain, so a non-empty axis can never become empty (no spurious
    /// empty-matrix BLOCK). Default false = byte-for-byte unchanged. Enable on the
    /// passive-intel stage whose spec declares "no enumeration denominator".
    #[serde(default)]
    pub coverage_anchor_only: bool,

    /// Whether this stage's deliverable may carry security `findings` (design
    /// 2026-06-15-recon-stage-findings-suppression). Discovery / recon stages
    /// (scoping / target_intel / external_attack_surface / enumeration) set this
    /// `false`: their deliverable is observations (`claims`) + a coverage matrix,
    /// NOT vulnerabilities, so a weak model dumping junk into `findings` there is
    /// noise. The `submit_stage_deliverable` tool drops findings for such a stage
    /// (and tells the model to put discoveries in `claims`). Attack-stage static
    /// specs preserve legacy compatibility; the persisted Candidate execution
    /// contract derives the stricter V2Only policy at runtime. Default true also
    /// remains back-compatible for specs that predate the flag.
    #[serde(default = "default_findings_allowed")]
    pub findings_allowed: bool,
}

impl StageSpec {
    /// Resolve the dependency set from the operation's already-frozen
    /// execution contract. An empty V2 override intentionally falls back to
    /// the legacy declaration so existing specs remain byte-compatible.
    pub fn required_stages_for_contract(&self, v2_only: bool) -> &[StageKind] {
        if v2_only && !self.v2_requires_stages.is_empty() {
            &self.v2_requires_stages
        } else {
            &self.requires_stages
        }
    }
}

fn default_continuity() -> AgentContinuity {
    AgentContinuity::SingleSession
}

fn default_findings_allowed() -> bool {
    true
}

#[derive(Debug, Error)]
pub enum StageSpecLoadError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid stage contract: {0}")]
    InvalidContract(String),
}

pub fn load_stage_spec_from_json(raw: &str) -> Result<StageSpec, StageSpecLoadError> {
    let spec: StageSpec = serde_json::from_str(raw)?;
    if let Some(policy) = spec.candidate_analysis_team.as_ref() {
        if spec.kind != StageKind::AttackCandidate {
            return Err(StageSpecLoadError::InvalidContract(
                "candidate_analysis_team is legal only on attack_candidate".to_string(),
            ));
        }
        policy
            .validate()
            .map_err(StageSpecLoadError::InvalidContract)?;
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTERNAL_ATTACK_SURFACE_JSON: &str =
        include_str!("../../../../../resources/harness/stages/external_attack_surface/spec.json");

    const TARGET_INTEL_JSON: &str =
        include_str!("../../../../../resources/harness/stages/target_intel/spec.json");

    const ENUMERATION_JSON: &str =
        include_str!("../../../../../resources/harness/stages/enumeration/spec.json");

    const VULN_TRIAGE_JSON: &str =
        include_str!("../../../../../resources/harness/stages/vuln_triage/spec.json");

    const REPORTING_JSON: &str =
        include_str!("../../../../../resources/harness/stages/reporting/spec.json");

    const ATTACK_CANDIDATE_JSON: &str =
        include_str!("../../../../../resources/harness/stages/attack_candidate/spec.json");

    const POST_EXPLOIT_STAGE_JSON: [&str; 4] = [
        include_str!("../../../../../resources/harness/stages/access_validation/spec.json"),
        include_str!("../../../../../resources/harness/stages/internal_discovery/spec.json"),
        include_str!("../../../../../resources/harness/stages/objective_pathing/spec.json"),
        include_str!("../../../../../resources/harness/stages/objective_simulation/spec.json"),
    ];

    #[test]
    fn load_external_attack_surface_basic_shape() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.id, "external_attack_surface");
        assert_eq!(s.kind, StageKind::ExternalAttackSurface);
        assert_eq!(s.risk_level, RiskLevel::Medium);
        assert_eq!(s.deliverable_schema, "ExternalAttackSurfaceDeliverable");
        assert_eq!(s.gate_validator, "validate_external_attack_surface_gate");
    }

    #[test]
    fn target_intel_declares_a_bounded_company_controller_scheduler() {
        let spec = load_stage_spec_from_json(TARGET_INTEL_JSON).expect("parse");
        let team = spec.team_scheduler.expect("target_intel team scheduler");

        assert!(team.enabled_in_v2_only);
        assert_eq!(team.aggregator_role, "company_stage_controller");
        assert!(team.allowed_roles.contains(&"intel_provider".to_string()));
        assert!(team.max_company_units_active > 0);
        assert!(team.global_provider_cap > 0);
        assert!(team.max_workers > 0);
        assert_eq!(team.max_dynamic_requests, 12);
        assert!(!team.allowed_dynamic_request_kinds.is_empty());
        assert!(team.max_dynamic_subject_refs > 0);
    }

    #[test]
    fn downstream_company_stages_declare_bounded_company_controller_schedulers() {
        for (raw, stage, specialist, child_role, request_kind, risk_lane) in [
            (
                EXTERNAL_ATTACK_SURFACE_JSON,
                StageKind::ExternalAttackSurface,
                "prober",
                "prober",
                "surface_probe",
                "active_recon",
            ),
            (
                ENUMERATION_JSON,
                StageKind::Enumeration,
                "enumerator",
                "enumerator",
                "content_enumeration",
                "active_recon",
            ),
            (
                VULN_TRIAGE_JSON,
                StageKind::VulnTriage,
                "vuln_scanner",
                "vuln_scanner",
                "formulaic_scan",
                "formulaic_scan",
            ),
        ] {
            let spec = load_stage_spec_from_json(raw).expect("embedded downstream stage spec");
            assert_eq!(spec.specialist.as_deref(), Some(specialist));
            let team = spec
                .team_scheduler
                .unwrap_or_else(|| panic!("{} must use Company Controller", stage.as_str()));

            assert!(team.enabled_in_v2_only);
            assert_eq!(team.aggregator_role, "company_stage_controller");
            assert!(team
                .allowed_roles
                .contains(&"company_stage_controller".to_string()));
            assert!(team.allowed_roles.contains(&child_role.to_string()));
            assert!(team
                .allowed_dynamic_request_kinds
                .contains(&request_kind.to_string()));
            assert!(team.max_company_units_active > 0);
            assert!(team.global_provider_cap > 0);
            assert!(team.max_workers > 0);
            assert!(team.max_dynamic_requests > 0);
            assert!(team.max_dynamic_subject_refs > 0);
            assert_eq!(team.risk_lane, risk_lane);
        }
    }

    #[test]
    fn company_controller_concurrency_caps_are_required_and_nonzero() {
        let source = serde_json::from_str::<serde_json::Value>(TARGET_INTEL_JSON).expect("json");
        for field in [
            "max_company_units_active",
            "global_provider_cap",
            "max_workers",
        ] {
            let mut missing = source.clone();
            missing["team_scheduler"]
                .as_object_mut()
                .expect("team policy object")
                .remove(field);
            assert!(serde_json::from_value::<StageSpec>(missing).is_err());

            let mut zero = source.clone();
            zero["team_scheduler"][field] = serde_json::json!(0);
            assert!(serde_json::from_value::<StageSpec>(zero).is_err());
        }
    }

    #[test]
    fn verification_v2_depends_on_candidate_and_never_uses_general_team_concurrency() {
        const VERIFICATION_JSON: &str =
            include_str!("../../../../../resources/harness/stages/verification/spec.json");
        let spec = load_stage_spec_from_json(VERIFICATION_JSON).expect("parse");

        assert_eq!(spec.v2_requires_stages, vec![StageKind::AttackCandidate]);
        assert_eq!(
            spec.required_stages_for_contract(true),
            &[StageKind::AttackCandidate]
        );
        assert_eq!(
            spec.required_stages_for_contract(false),
            &[StageKind::VulnTriage]
        );
        assert!(spec.team_scheduler.is_none());
    }

    #[test]
    fn candidate_analysis_team_is_closed_bounded_and_uses_plan_a_authority() {
        use golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1;

        let spec = load_stage_spec_from_json(ATTACK_CANDIDATE_JSON).expect("parse Candidate spec");
        let team = spec
            .candidate_analysis_team
            .expect("Candidate analysis team policy");

        assert_eq!(team.schema_version, 1);
        assert_eq!(team.controller_role, "candidate_hypothesis_controller");
        assert_eq!(team.analyst_role, "candidate_hypothesis_analyst");
        assert_eq!(team.critic_role, "merge_conflict_critic");
        assert_eq!(team.final_submitter_role, team.controller_role);
        assert_ne!(team.controller_role, team.analyst_role);
        assert_ne!(team.controller_role, team.critic_role);
        assert_ne!(team.analyst_role, team.critic_role);
        assert!((2..=8).contains(&team.min_live_analysis_lanes));
        assert!((team.min_live_analysis_lanes..=8).contains(&team.max_live_analysis_lanes));
        assert!(team.single_lane_input_limit > 0);

        for version in [
            team.chunking_contract_version,
            team.redaction_contract_version,
            team.attack_class_checklist_contract_version,
            team.trust_boundary_checklist_contract_version,
            team.coverage_partition_contract_version,
            team.coverage_synthesis_contract_version,
            team.hypothesis_coverage_sampling_contract_version,
            team.knowledge_feed_snapshot_contract_version,
            team.product_version_match_contract_version,
        ] {
            assert!(version > 0);
        }
        for ceiling in [
            team.max_inputs_per_microbatch,
            team.max_chunk_bytes,
            team.max_chunks_per_input,
            team.max_chunks_per_coverage_partition,
            team.max_coverage_subreview_work_items,
            team.max_synthesis_inputs_per_partition,
            team.max_proposals_per_artifact,
            team.max_controller_page_size,
            team.max_attempts_per_work_item,
            team.max_analysis_attempts,
        ] {
            assert!(ceiling > 0);
        }
        assert!(team.max_source_bytes_per_input > 0);
        assert!(team.max_knowledge_feed_age_seconds > 0);
        assert!(team.require_checked_tool_truth_temporal_authority);
        assert!(team.require_signed_knowledge_feeds);
        assert!(team.require_read_only_children);
        assert!(team.require_tool_free_children);
        assert_eq!(
            team.required_tool_truth_root_families,
            ToolTruthRootFamilyV1::ALL
        );

        let raw: serde_json::Value = serde_json::from_str(ATTACK_CANDIDATE_JSON).expect("json");
        let policy = raw["candidate_analysis_team"]
            .as_object()
            .expect("team object");
        for duplicate_authority in [
            "temporal_policy_version",
            "temporal_policy_hash",
            "max_cross_observation_skew_ms",
            "positive_ttl_ms",
            "negative_ttl_ms",
            "refutation_ttl_ms",
        ] {
            assert!(!policy.contains_key(duplicate_authority));
        }
    }

    #[test]
    fn candidate_analysis_team_rejects_invalid_stage_roots_and_bounds() {
        let source =
            serde_json::from_str::<serde_json::Value>(ATTACK_CANDIDATE_JSON).expect("json");

        let mut wrong_stage = source.clone();
        wrong_stage["kind"] = serde_json::json!("reporting");
        assert!(load_stage_spec_from_json(&wrong_stage.to_string()).is_err());

        for (field, invalid) in [
            ("min_live_analysis_lanes", serde_json::json!(1)),
            ("max_live_analysis_lanes", serde_json::json!(9)),
            ("single_lane_input_limit", serde_json::json!(0)),
            ("coverage_partition_contract_version", serde_json::json!(0)),
            ("coverage_synthesis_contract_version", serde_json::json!(0)),
            ("max_coverage_subreview_work_items", serde_json::json!(0)),
            ("max_synthesis_inputs_per_partition", serde_json::json!(0)),
            ("max_analysis_attempts", serde_json::json!(0)),
            ("require_signed_knowledge_feeds", serde_json::json!(false)),
            (
                "require_checked_tool_truth_temporal_authority",
                serde_json::json!(false),
            ),
        ] {
            let mut invalid_spec = source.clone();
            invalid_spec["candidate_analysis_team"][field] = invalid;
            assert!(
                load_stage_spec_from_json(&invalid_spec.to_string()).is_err(),
                "invalid field must fail closed: {field}"
            );
        }

        let mut duplicate_root = source;
        duplicate_root["candidate_analysis_team"]["required_tool_truth_root_families"] =
            serde_json::json!(["ti", "eas", "enum", "enum"]);
        assert!(load_stage_spec_from_json(&duplicate_root.to_string()).is_err());
    }

    #[test]
    fn external_attack_surface_requires_and_next_stages() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert!(s.requires_stages.contains(&StageKind::Scoping));
        assert!(s.requires_stages.contains(&StageKind::TargetIntel));
        assert!(s.allowed_next_stages.contains(&StageKind::Enumeration));
        assert!(s.allowed_next_stages.contains(&StageKind::Reporting));
    }

    #[test]
    fn external_attack_surface_allowed_tool_types() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        // 去上阶段工具（2026-06-10）：EAS 不再做被动 DNS（移交 target_intel，复用继承的
        // dns_a），改用 httpx 一步解析+探测确认存活，杜绝在本阶段重采 dig。
        assert!(!s.allowed_tool_types.contains(&"recon/dns".to_string()));
        assert!(s.allowed_tool_types.contains(&"recon/http".to_string()));
        assert!(s.allowed_tool_types.contains(&"recon/visual".to_string()));
        // 阶段重排 2026-06-09：端口扫描前移到 EAS（先定义攻击面再枚举内容）。
        assert!(s
            .allowed_tool_types
            .contains(&"recon/port-scan".to_string()));
        assert!(!s.allowed_tool_types.contains(&"web/injection".to_string()));
        // 边界重构（按是否接触目标）：被动子域名 / url-history 下沉 target_intel，
        // EAS 不再允许它们（只做接触目标的主动测绘）。
        assert!(!s
            .allowed_tool_types
            .contains(&"recon/subdomain".to_string()));
        assert!(!s
            .allowed_tool_types
            .contains(&"recon/url-history".to_string()));
    }

    #[test]
    fn post_exploit_stages_expose_one_exact_p6b_wrapper_after_cleanup_cutover() {
        let expected = [
            "post_exploit_validate_access",
            "post_exploit_record_internal_observation",
            "post_exploit_build_objective_path",
            "post_exploit_execute_action",
        ];
        for (raw, expected_tool) in POST_EXPLOIT_STAGE_JSON.into_iter().zip(expected) {
            let spec = load_stage_spec_from_json(raw).expect("parse post-exploit spec");
            let policy = spec
                .post_exploit_policy
                .expect("post-exploit stage declares execution policy");
            assert!(!policy.p6a_only, "stage={}", spec.id);
            assert!(policy
                .allowed_side_effect_classes
                .contains(&"none".to_string()));
            assert!(policy.requires_cleanup_kernel_for_side_effects);
            assert!(!policy.work_item_kind.trim().is_empty());
            assert!(!policy.canonical_rows.is_empty());
            assert_eq!(spec.specialist.as_deref(), Some("post_exploit_operator"));
            assert_eq!(spec.allowed_tool_types, [expected_tool]);
            assert!(!spec.findings_allowed);
        }
        let objective = load_stage_spec_from_json(POST_EXPLOIT_STAGE_JSON[3])
            .expect("parse objective simulation spec");
        let policy = objective.post_exploit_policy.expect("objective policy");
        assert!(policy
            .allowed_side_effect_classes
            .contains(&"remote_state_mutation".to_string()));
        assert!(policy
            .allowed_side_effect_classes
            .contains(&"local_artifact_mutation".to_string()));
    }

    #[test]
    fn external_attack_surface_min_invocations() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        // 2026-06-25：EAS now uses DB-truth coverage as the hard floor. Do not
        // force the model to hand-copy `required_checks_done` just to satisfy an
        // http_probe minimum.
        assert_eq!(s.min_invocations.get("dns_resolve"), None);
        assert_eq!(s.min_invocations.get("http_probe"), None);
        assert!(s.min_invocations.is_empty());
        // 边界重构：被动子域名枚举不再钉为 EAS 硬地板（移交 target_intel）。
        assert_eq!(s.min_invocations.get("subdomain_enum_passive"), None);
    }

    #[test]
    fn external_attack_surface_gate_rules_count() {
        // EAS 过关标准 = claim/finding evidence + coverage found/checked_empty evidence
        // + named_check:surface_coverage
        // + coverage_complete(per-asset liveness/port/service with notes for other)
        // + coverage_denominator(full explicit coverage) = 7 条 gate_rules。
        // 2026-07-02 gate capability ledger Phase 0: dropped the vacuous
        // named_check:min_invocations rule (spec.min_invocations is {} → always
        // Pass), so the count fell from 8 to 7.
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.gate_rules.len(), 7);
    }

    // 2026-06-09 verify-first + 阶段重排：EAS = 定义攻击面，必须对每个 in-scope 资产
    // 把「存活 + 端口 + 服务指纹 + Web 指纹」纳入过关标准——声明
    // GOLISH-EAS-{LIVENESS,PORT,SERVICE-FINGERPRINT,WEB-FINGERPRINT}
    // expected techniques + 一条 coverage_complete gate_rule。
    // 端口/服务从 enumeration 前移到此（先扫端口再去枚举内容）；JS/API 移交 enumeration。
    #[test]
    fn external_attack_surface_requires_per_asset_surface_coverage() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        for tech in [
            "GOLISH-EAS-LIVENESS",
            "GOLISH-EAS-PORT",
            "GOLISH-EAS-SERVICE-FINGERPRINT",
            "GOLISH-EAS-WEB-FINGERPRINT",
        ] {
            assert!(
                s.expected_techniques.contains(&tech.to_string()),
                "EAS must declare {tech} as an expected technique"
            );
        }
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    require_note_for_other: true,
                    error_is_terminal: false,
                    ..
                }
            )),
            "EAS gate_rules must require notes for blocked/not_applicable and keep retryable errors unfinished"
        );
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageDenominator {
                    min_sample_ratio_pct: 100,
                    authoritative: true,
                    ..
                }
            )),
            "EAS denominator rule must be authoritative/no-op for DB-truth slim deliverables"
        );
        assert!(
            s.facts_from_db_truth,
            "EAS must opt into DB-truth slim deliverables"
        );
    }

    // 2026-06-09 verify-first + 阶段重排 · enumeration = 内容枚举：在 EAS 摸清的服务上
    // 做 JS/API + 目录 + 参数（声明 GOLISH-ENUM-{DIR,PARAM,JSAPI}）。端口/服务前移到
    // EAS，故本阶段不再含 recon/port-scan。产出的可测单元是 vuln_triage 分母来源。
    #[test]
    fn enumeration_requires_per_asset_content_coverage() {
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::Enumeration)
            .expect("load enumeration spec");
        for tech in ["GOLISH-ENUM-DIR", "GOLISH-ENUM-PARAM", "GOLISH-ENUM-JSAPI"] {
            assert!(
                s.expected_techniques.contains(&tech.to_string()),
                "enumeration must declare {tech} as an expected technique"
            );
        }
        // 阶段重排：端口/服务前移到 EAS，enumeration 不再做端口扫描。
        assert!(
            !s.allowed_tool_types
                .contains(&"recon/port-scan".to_string()),
            "enumeration must NOT allow port-scan after the 2026-06-09 reorder"
        );
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    error_is_terminal: false,
                    authoritative_found: true,
                    require_note_for_other: true,
                    ..
                }
            )),
            "enumeration coverage_complete must require current outcome truth, notes, and keep errors unfinished"
        );
    }

    #[test]
    fn external_attack_surface_human_approval_required_before() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        let ha = s.human_approval.expect("human_approval");
        assert!(ha.required_before.contains(&"active_scan".to_string()));
        assert!(ha
            .required_before
            .contains(&"exploit_validation".to_string()));
    }

    #[test]
    fn external_attack_surface_inherits_evidence_from_target_intel() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.inherits_evidence_from.len(), 1);
        let inh = &s.inherits_evidence_from[0];
        assert_eq!(inh.stage_kind, StageKind::TargetIntel);
        assert!(inh.evidence_kinds.contains(&"dns_a".to_string()));
        assert!(inh.evidence_kinds.contains(&"asn".to_string()));
        assert!(inh.evidence_kinds.contains(&"whois".to_string()));
        // 边界重构：EAS 从 target_intel 继承子域名（host 来源），不再自枚举。
        assert!(inh.evidence_kinds.contains(&"subdomain".to_string()));
    }

    #[test]
    fn verification_inheritance_uses_closed_typed_handoff_kinds() {
        let reporting = crate::harness::resources::load_embedded_stage_spec(StageKind::Reporting)
            .expect("load reporting spec");
        let reporting_verification = reporting
            .inherits_evidence_from
            .iter()
            .find(|inherit| inherit.stage_kind == StageKind::Verification)
            .expect("reporting inherits Verification");
        assert_eq!(
            reporting_verification.evidence_kinds,
            vec![
                "candidate_attempt_terminal".to_string(),
                "verified_candidate_attempt".to_string(),
                "attack_fact_delta_proposal".to_string(),
                "attack_no_candidate_decision".to_string(),
                "vuln_finding".to_string(),
            ]
        );
        let access =
            crate::harness::resources::load_embedded_stage_spec(StageKind::AccessValidation)
                .expect("load access_validation spec");
        let access_verification = access
            .inherits_evidence_from
            .iter()
            .find(|inherit| inherit.stage_kind == StageKind::Verification)
            .expect("access_validation inherits Verification");
        assert_eq!(
            access_verification.evidence_kinds,
            vec![
                "verified_candidate_attempt".to_string(),
                "vuln_finding".to_string(),
            ]
        );
    }

    #[test]
    fn target_intel_keeps_subdomain_coverage_but_blocks_cli_subdomain_tools() {
        let s = load_stage_spec_from_json(TARGET_INTEL_JSON).expect("parse");
        // SUBDOMAIN 仍由 target_intel 覆盖，但 2026-06-23 provider-source
        // boundary 后，target_intel 不再暴露任何 scan-tool selector。阶段只走
        // recon_map_assets / recon_lookup_whois 等 registry 工具；缺 provider/source
        // 时提交 terminal coverage cell，而不是切 CLI fallback。
        assert!(s.allowed_tool_types.is_empty());
        assert!(s
            .expected_techniques
            .contains(&"GOLISH-INTEL-SUBDOMAIN".to_string()));
        assert!(s.coverage_axis.contains(&"SUBDOMAIN".to_string()));
        assert!(s.min_invocations.is_empty());
    }

    // P5（2026-06-11）：target_intel 的 coverage 必须既能从 technique 标注的 claims
    // 派生（derive_from_items），又对 found cell 做反向佐证（coverage_corroborated）。
    // 只断言存在性，不锁 gate_rules 总数（避免与 in-flight 规则增删撞车）。
    #[test]
    fn target_intel_coverage_derives_and_corroborates() {
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::TargetIntel)
            .expect("load target_intel spec");
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    derive_from_items: true,
                    ..
                }
            )),
            "target_intel coverage_complete must enable derive_from_items"
        );
        // PR3 (D-scope 灰度): target_intel 是 evidence 投影的首个灰度阶段。
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    derive_from_evidence: true,
                    ..
                }
            )),
            "target_intel coverage_complete must enable derive_from_evidence (PR3 gray rollout)"
        );
        // Phase 0 (2026-06-12-redteam-phase0): target_intel 是 authoritative_found
        // 的首个灰度阶段——found 对已落点的 4 类技术只认 DB/账本真值。
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    authoritative_found: true,
                    ..
                }
            )),
            "target_intel coverage_complete must enable authoritative_found (Phase 0 gray rollout)"
        );
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageCorroborated { .. }
            )),
            "target_intel must declare a coverage_corroborated rule"
        );
    }

    // Phase 1 (2026-06-12-redteam-phase1 §5): active stages coverage 必须开
    // derive_from_evidence，让 coverage_truth 的主动维度（PORT/SERVICE/DIR/PARAM/
    // JSAPI…）DB 投影能补格。2026-06-25: EAS found coverage has been promoted to
    // authoritative DB truth; 2026-07-10 promotes Enumeration too, using only
    // current exact-origin technique_outcomes rather than business-table rows.
    #[test]
    fn active_stages_derive_from_evidence_with_authoritative_outcomes() {
        for kind in [StageKind::ExternalAttackSurface, StageKind::Enumeration] {
            let s = crate::harness::resources::load_embedded_stage_spec(kind)
                .unwrap_or_else(|_| panic!("load {kind:?} spec"));
            assert!(
                s.gate_rules.iter().any(|r| matches!(
                    r,
                    crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                        derive_from_evidence: true,
                        ..
                    }
                )),
                "{kind:?} coverage_complete must enable derive_from_evidence (Phase 1 DB-truth 补格)"
            );
            let has_authoritative = s.gate_rules.iter().any(|r| {
                matches!(
                    r,
                    crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                        authoritative_found: true,
                        ..
                    }
                )
            });
            assert!(
                has_authoritative,
                "EAS and Enumeration must use authoritative completion truth"
            );
        }
    }

    // Host-aware coverage 2b flip (design 2026-06-15-host-aware-coverage §3.2/§3.3):
    // EAS + enumeration enable host_aware_coverage so coverage_complete holds each
    // in-scope asset only to the techniques that apply to its class (EAS: a bare URL
    // endpoint drops PORT/SERVICE-FINGERPRINT; enumeration: a bare IP/CIDR drops all
    // content techniques). The per-asset matrix landed inert (commit e12a7638); this
    // guards the stage flag stays on so the relaxation is actually active.
    #[test]
    fn eas_and_enumeration_enable_host_aware_coverage() {
        use crate::harness::resources::load_embedded_stage_spec;
        for kind in [StageKind::ExternalAttackSurface, StageKind::Enumeration] {
            let s = load_embedded_stage_spec(kind).unwrap_or_else(|_| panic!("load {kind:?}"));
            assert!(
                s.host_aware_coverage,
                "{kind:?} must enable host_aware_coverage (2b flip)"
            );
        }
        // target_intel (2a) was already on; sanity-check the flip did not regress it.
        let ti = load_embedded_stage_spec(StageKind::TargetIntel).expect("load target_intel");
        assert!(
            ti.host_aware_coverage,
            "target_intel (2a) host_aware_coverage stays on"
        );
    }

    #[test]
    fn enumeration_enables_ip_web_coverage_only() {
        let enumeration =
            crate::harness::resources::load_embedded_stage_spec(StageKind::Enumeration)
                .expect("load enumeration spec");
        assert!(
            enumeration.enum_ip_web_coverage,
            "enumeration must include EAS/httpx-proven IP web roots"
        );

        for kind in [StageKind::TargetIntel, StageKind::ExternalAttackSurface] {
            let spec =
                crate::harness::resources::load_embedded_stage_spec(kind).expect("load spec");
            assert!(
                !spec.enum_ip_web_coverage,
                "{kind:?} should not opt into enumeration IP-web coverage"
            );
        }
    }

    // Phase 2 (2026-06-12-redteam-phase2): scoping 子公司 gate 的零回归静态前提。
    // 1. 静态 expected_techniques 必须为空——SUBSIDIARY 只由 execute.rs hook 在
    //    `--include-subsidiaries` 时动态注入; 不带 flag → coverage_complete 看不到
    //    期望技术 → no-op (旧 scoping 行为逐字节不变)。
    // 2. coverage_complete 必须 authoritative 且收紧范围锁定 SUBSIDIARY——found
    //    只认 DB 真值 (organizations.parent_id 有 child org), 自报不算。
    // 3. allowed_tool_types 含 recon/osint——子公司发现 (ENScan, 工商数据 OSINT,
    //    非 probing) 可以在 L0 scoping 阶段跑。
    #[test]
    fn scoping_subsidiary_gate_static_premises() {
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::Scoping)
            .expect("load scoping spec");
        assert!(
            s.expected_techniques.is_empty(),
            "scoping expected_techniques must stay empty (SUBSIDIARY is hook-injected only; \
             a static entry would force the subsidiary gate on EVERY engagement)"
        );
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                crate::harness::gate::rule_engine::GateRule::CoverageComplete {
                    derive_from_evidence: true,
                    authoritative_found: true,
                    authoritative_techniques: Some(ref t),
                    ..
                } if t == &vec!["GOLISH-INTEL-SUBSIDIARY".to_string()]
            )),
            "scoping coverage_complete must be authoritative for exactly GOLISH-INTEL-SUBSIDIARY \
             (found = DB-landed child org, not self-report)"
        );
        assert!(
            s.allowed_tool_types.iter().any(|t| t == "recon/osint"),
            "scoping must allow recon/osint so subsidiary discovery (ENScan) can run at L0"
        );
    }

    #[test]
    fn external_attack_surface_agent_continuity_single_session() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.agent_continuity, AgentContinuity::SingleSession);
    }

    #[test]
    fn external_attack_surface_max_other_skips() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.max_other_skips, Some(2));
    }

    #[test]
    fn gate_rules_default_empty_and_parses() {
        // 缺省：未写 gate_rules 的 spec 解出空数组（向后兼容）。用最小内联 spec
        // （eas.json 现已迁移到 gate_rules，不再是“无 gate_rules”的样例）。
        let minimal = r#"{"id":"scoping","kind":"scoping","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(s.gate_rules.is_empty());

        // 能解析内联 gate_rules。
        let with_rules = r#"{
            "id":"verification","kind":"verification","risk_level":"critical",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "gate_rules":[
              { "op":"for_all","over":"findings",
                "where":{"pred":"severity_at_least","min":"high"},
                "require":{"pred":"non_empty","field":"evidence_refs"},
                "on_fail":{"reason":"high+ finding needs evidence"} }
            ]
        }"#;
        let s2 = load_stage_spec_from_json(with_rules).expect("parse with rules");
        assert_eq!(s2.gate_rules.len(), 1);
    }

    // 设计 2026-06-12-unified-refiner (PR-R2)：投影兜底 opt-in 字段已删除——旧 spec
    // 若仍带该键，serde 默认行为（未知字段忽略）必须保证解析不被破坏。
    #[test]
    fn legacy_projection_flag_in_json_is_ignored_not_fatal() {
        let with_legacy_key = r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "synthesize_from_evidence_when_missing":true}"#;
        let s = load_stage_spec_from_json(with_legacy_key)
            .expect("legacy key must be ignored, not a parse error");
        assert_eq!(s.id, "target_intel");
    }

    // stage_run fan-out (2026-06-13-stage-run-fanout §3.2): target_intel declares
    // its per-org specialist (recon) + the display coverage axis; specs that omit
    // these stay None / empty (back-compat — stage_run simply does not apply).
    #[test]
    fn target_intel_declares_stage_run_specialist_and_axis() {
        let s = load_stage_spec_from_json(TARGET_INTEL_JSON).expect("parse");
        assert_eq!(s.specialist.as_deref(), Some("recon"));
        assert_eq!(
            s.coverage_axis,
            vec!["DNS", "WHOIS", "ASN", "CT", "SUBDOMAIN", "OSINT"]
        );
    }

    // stage_run fan-out (2026-06-13-stage-run-fanout §3.2 · EAS rollout): EAS
    // declares its per-org specialist (`prober`, the active surface-mapper split
    // from Pentester, mirroring how `recon` was split for target_intel) + the
    // display coverage axis (liveness / port / service-fingerprint /
    // web-fingerprint). Without this, the chat `stage_run` tool refuses EAS
    // ("stage has no `specialist` configured") and EAS cannot fan out per org.
    #[test]
    fn external_attack_surface_declares_stage_run_specialist_and_axis() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert_eq!(s.specialist.as_deref(), Some("prober"));
        assert_eq!(s.coverage_axis, vec!["LIVENESS", "PORT", "SERVICE", "WEB"]);
    }

    #[test]
    fn four_specialist_stages_declare_the_exact_runtime_memory_v2_contract() {
        use crate::harness::{RuntimeScopeSource, RuntimeUnitIdentity};

        let expected = StageRuntimeContract {
            schema_version: 2,
            unit_identity: RuntimeUnitIdentity::StageExecutionOrganization,
            scope_source: RuntimeScopeSource::FrozenOperationSnapshot,
            requires_worker_lease: true,
            publishes_handoff_after_final_seal: true,
        };
        for stage in [
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
            StageKind::VulnTriage,
        ] {
            let spec = crate::harness::resources::load_embedded_stage_spec(stage)
                .unwrap_or_else(|error| panic!("load {} spec: {error}", stage.as_str()));
            assert_eq!(
                spec.runtime_memory,
                Some(expected),
                "{} must declare the exact Runtime Memory V2 specialist contract",
                stage.as_str()
            );
            assert!(
                spec.specialist
                    .as_deref()
                    .is_some_and(|specialist| !specialist.trim().is_empty()),
                "{} must keep a concrete specialist",
                stage.as_str()
            );
        }
    }

    #[test]
    fn root_only_stages_do_not_invent_a_specialist_runtime_contract() {
        for stage in [StageKind::Scoping, StageKind::Reporting] {
            let spec = crate::harness::resources::load_embedded_stage_spec(stage)
                .unwrap_or_else(|error| panic!("load {} spec: {error}", stage.as_str()));
            assert!(spec.runtime_memory.is_none(), "stage={}", stage.as_str());
        }
    }

    #[test]
    fn reporting_declares_current_validated_revision_gate() {
        let spec = load_stage_spec_from_json(REPORTING_JSON).expect("parse reporting spec");
        assert_eq!(spec.kind, StageKind::Reporting);
        assert!(spec.allowed_tool_types.is_empty());
        assert!(matches!(
            spec.gate_rules.as_slice(),
            [crate::harness::gate::rule_engine::GateRule::ReportRevisionValidated { .. }]
        ));
    }

    #[test]
    fn external_attack_surface_enables_asset_wave_barrier_only() {
        let eas =
            crate::harness::resources::load_embedded_stage_spec(StageKind::ExternalAttackSurface)
                .expect("load external_attack_surface spec");
        assert!(
            eas.asset_wave_barrier,
            "EAS must freeze its current-wave coverage denominator"
        );

        let target_intel =
            crate::harness::resources::load_embedded_stage_spec(StageKind::TargetIntel)
                .expect("load target_intel spec");
        assert!(
            !target_intel.asset_wave_barrier,
            "passive intel keeps its anchor-only denominator, not the active wave barrier"
        );
    }

    #[test]
    fn skip_dead_assets_only_on_downstream_of_eas() {
        // Dead-asset P3 (design 2026-07-02-dead-asset-liveness-state §5.2):
        // enumeration + vuln_triage drop confirmed-dead assets from the coverage
        // denominator; EAS must NOT (it is the stage that judges liveness — it
        // needs its full denominator to probe).
        let eas =
            crate::harness::resources::load_embedded_stage_spec(StageKind::ExternalAttackSurface)
                .expect("load external_attack_surface spec");
        assert!(
            !eas.skip_dead_assets,
            "EAS must keep dead-candidate assets in its denominator (it judges liveness)"
        );

        for kind in [StageKind::Enumeration, StageKind::VulnTriage] {
            let s = crate::harness::resources::load_embedded_stage_spec(kind)
                .unwrap_or_else(|e| panic!("load {} spec: {e}", kind.as_str()));
            assert!(
                s.skip_dead_assets,
                "{} must exclude confirmed-dead assets from its denominator",
                kind.as_str()
            );
        }
    }

    // stage_run fan-out (2026-06-13-stage-run-fanout §3.2 · enumeration rollout): enumeration
    // declares its per-org specialist (`enumerator`, the active content-enumeration mapper
    // split from Pentester, mirroring how `prober` was split for EAS) + the display coverage
    // axis (its 4 expected techniques: js / dir / param / js-api). Without this, the chat
    // `stage_run` tool refuses enumeration ("stage has no `specialist` configured").
    #[test]
    fn enumeration_declares_stage_run_specialist_and_axis() {
        use crate::harness::gate::rule_engine::GateRule;

        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::Enumeration)
            .expect("load enumeration spec");
        assert_eq!(s.specialist.as_deref(), Some("enumerator"));
        assert_eq!(s.coverage_axis, vec!["JS", "DIR", "PARAM", "JSAPI"]);
        assert!(s.gate_rules.iter().any(|rule| matches!(
            rule,
            GateRule::CoverageComplete {
                authoritative_found: true,
                require_note_for_other: true,
                error_is_terminal: false,
                ..
            }
        )));
    }

    // Attack-stage split (design 2026-07-02 §3.2, P3 Task3.1): vuln_triage becomes
    // the formulaic-scan stage — declare its per-org specialist (`vuln_scanner`),
    // a display coverage axis, and opt into the DB-truth matrix paradigm
    // (facts_from_db_truth + freshness_window) so a slim scan deliverable is not
    // treated as vacuous.
    #[test]
    fn vuln_triage_declares_specialist_and_axis() {
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::VulnTriage)
            .expect("load vuln_triage spec");
        assert_eq!(s.specialist.as_deref(), Some("vuln_scanner"));
        assert!(!s.coverage_axis.is_empty());
        assert!(s.facts_from_db_truth);
        assert!(s.freshness_window);
        assert!(
            !s.asset_wave_barrier,
            "VulnTriage inherits the final-sealed Enumeration exact-origin surface; it must not seed the generic raw-target asset wave"
        );
        assert_eq!(
            s.allowed_tool_types,
            vec![
                "vuln_nuclei_general".to_string(),
                "vuln_nuclei_fingerprint_targeted".to_string(),
                "vuln_probe_anonymous_access".to_string(),
            ]
        );
        for wrapper in [
            "vuln_nuclei_general",
            "vuln_nuclei_fingerprint_targeted",
            "vuln_probe_anonymous_access",
        ] {
            assert!(crate::harness::stage_allows(
                wrapper,
                &serde_json::json!({}),
                &s.allowed_tool_types,
            ));
        }
        assert!(!crate::harness::stage_allows(
            "nuclei",
            &serde_json::json!({}),
            &s.allowed_tool_types,
        ));
        assert!(!crate::harness::stage_allows(
            "pentest_run",
            &serde_json::json!({"tool_name": "nuclei"}),
            &s.allowed_tool_types,
        ));
    }

    // Attack-stage split (design 2026-07-02 §3.9, P3 Task3.2): vuln_triage is
    // narrowed to the 10 formulaic technique classes and its coverage_complete
    // derives only from the three trusted wrapper families' DB truth. True IDOR
    // plus the other reasoning-heavy classes move to attack_candidate; bounded
    // anonymous-access observation remains here. All formulaic classes are
    // authoritative, so a hand-written deliverable cell cannot close the gate.
    #[test]
    fn vuln_triage_narrowed_to_formulaic_and_derives_from_evidence() {
        use crate::harness::gate::rule_engine::GateRule;
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::VulnTriage)
            .expect("load vuln_triage spec");
        assert_eq!(s.expected_techniques.len(), 10);
        assert!(s.expected_techniques.contains(&"GOLISH-NDAY".to_string()));
        assert!(s.expected_techniques.contains(&"WSTG-ATHN-04".to_string()));
        assert!(!s.expected_techniques.contains(&"WSTG-ATHZ-04".to_string()));
        assert_eq!(
            s.allowed_tool_types,
            vec![
                "vuln_nuclei_general",
                "vuln_nuclei_fingerprint_targeted",
                "vuln_probe_anonymous_access",
            ]
        );
        for moved in ["WSTG-BUSL", "WSTG-INPV-19", "WSTG-INPV-18", "WSTG-ATHZ-04"] {
            assert!(
                !s.expected_techniques.contains(&moved.to_string()),
                "{moved} must move out of vuln_triage"
            );
        }
        assert!(
            s.gate_rules.iter().any(|r| matches!(
                r,
                GateRule::CoverageComplete {
                    derive_from_evidence: true,
                    ..
                }
            )),
            "vuln_triage coverage_complete must enable derive_from_evidence"
        );
    }

    // The three foreground wrapper families self-land every formulaic class, so
    // all ten cells are authoritative. Legacy shell/source rows and hand-written
    // deliverable coverage cannot close this stage.
    #[test]
    fn vuln_triage_all_formulaic_classes_are_authoritative() {
        use crate::harness::gate::rule_engine::GateRule;
        let s = crate::harness::resources::load_embedded_stage_spec(StageKind::VulnTriage)
            .expect("load vuln_triage spec");
        let scoped = s.gate_rules.iter().find_map(|r| match r {
            GateRule::CoverageComplete {
                authoritative_found: true,
                authoritative_techniques: Some(t),
                ..
            } => Some(t.clone()),
            _ => None,
        });
        let techs = scoped.expect(
            "vuln_triage coverage_complete must be authoritative for a scoped technique list",
        );
        let mut sorted = techs;
        sorted.sort();
        let mut expected = s.expected_techniques.clone();
        expected.sort();
        assert_eq!(
            sorted, expected,
            "authoritative scope must cover every wrapper-owned formulaic class"
        );
    }

    #[test]
    fn specialist_and_coverage_axis_default_when_absent() {
        let minimal = r#"{"id":"scoping","kind":"scoping","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(s.specialist.is_none());
        assert!(s.coverage_axis.is_empty());
    }

    #[test]
    fn expected_techniques_default_empty_and_parses() {
        // 缺省：未写 expected_techniques 的 spec 解出空数组（coverage_complete no-op）。
        let minimal = r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(s.expected_techniques.is_empty());

        // 能解析 WSTG / ATT&CK id 字符串数组。
        let with = r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "expected_techniques":["WSTG-INPV-05","WSTG-ATHZ-04","T1190"]}"#;
        let s2 = load_stage_spec_from_json(with).expect("parse expected_techniques");
        assert_eq!(s2.expected_techniques.len(), 3);
        assert_eq!(s2.expected_techniques[0], "WSTG-INPV-05");
    }

    // 2026-06-15-recon-stage-findings-suppression: the new findings_allowed flag
    // defaults true (back-compat) and parses an explicit false.
    #[test]
    fn findings_allowed_defaults_true_and_parses_false() {
        let minimal = r#"{"id":"vuln_triage","kind":"vuln_triage","risk_level":"high",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#;
        let s = load_stage_spec_from_json(minimal).expect("parse");
        assert!(
            s.findings_allowed,
            "omitted findings_allowed must default to true (back-compat)"
        );

        let with = r#"{"id":"target_intel","kind":"target_intel","risk_level":"low",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "findings_allowed":false}"#;
        let s2 = load_stage_spec_from_json(with).expect("parse findings_allowed");
        assert!(!s2.findings_allowed);
    }

    // VulnTriage and AttackCandidate are observation/reasoning stages for every
    // runtime contract. Only Verification retains its persisted legacy finding
    // compatibility; V2 findings still come from the Attempt terminalizer.
    #[test]
    fn stage_findings_policy_preserves_candidate_v2_and_legacy_verification_boundaries() {
        use crate::harness::resources::load_embedded_stage_spec;
        for kind in [
            StageKind::Scoping,
            StageKind::TargetIntel,
            StageKind::ExternalAttackSurface,
            StageKind::Enumeration,
        ] {
            let s = load_embedded_stage_spec(kind).unwrap_or_else(|_| panic!("load {kind:?}"));
            assert!(
                !s.findings_allowed,
                "recon stage {kind:?} must set findings_allowed=false"
            );
        }
        for kind in [StageKind::VulnTriage, StageKind::AttackCandidate] {
            let spec = load_embedded_stage_spec(kind).unwrap_or_else(|_| panic!("load {kind:?}"));
            assert!(!spec.findings_allowed, "{kind:?} must not author findings");
        }
        let verification =
            load_embedded_stage_spec(StageKind::Verification).expect("load Verification spec");
        assert!(verification.findings_allowed);
    }

    #[test]
    fn candidate_v2_stage_resources_define_the_durable_wave_contract() {
        let vuln_triage = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/vuln_triage/spec.json"
        ));
        let attack_candidate = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/attack_candidate/spec.json"
        ));
        let verification = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/verification/spec.json"
        ));
        let phases = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/graph/phases.json"
        ));
        let vuln_triage_methodology = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/vuln_triage/methodology.md"
        ));
        let attack_candidate_methodology = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/attack_candidate/methodology.md"
        ));
        let verification_methodology = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/stages/verification/methodology.md"
        ));

        for required in [
            "initial Wave only",
            "follow-on Wave entry",
            "FactDelta consolidation",
        ] {
            assert!(
                vuln_triage.contains(required),
                "missing vuln_triage contract: {required}"
            );
        }
        for required in [
            "vuln_triage_handoff",
            "fact_delta_consolidation",
            "zero-input",
            "durable review",
        ] {
            assert!(
                attack_candidate.contains(required),
                "missing attack_candidate contract: {required}"
            );
        }
        for required in [
            "exact Candidate plan approval",
            "FactDelta proposal",
            "opened_next_wave",
            "closed_no_delta",
            "exhausted",
            "residual risk",
        ] {
            assert!(
                verification.contains(required),
                "missing Verification contract: {required}"
            );
        }
        for required in [
            "V2Only",
            "exact Candidate plan approval",
            "no static back-edge",
        ] {
            assert!(
                phases.contains(required),
                "missing phase contract: {required}"
            );
        }
        for required in [
            "initial Wave only",
            "follow-on Wave entry",
            "accepted FactDelta",
        ] {
            assert!(
                vuln_triage_methodology.contains(required),
                "missing vuln_triage methodology contract: {required}"
            );
        }
        for required in [
            "vuln_triage_handoff",
            "fact_delta_consolidation",
            "zero-input organization",
            "durable Candidate review",
            "opened_next_wave",
            "closed_no_delta",
            "exhausted",
            "residual risk",
        ] {
            assert!(
                attack_candidate_methodology.contains(required),
                "missing attack_candidate methodology contract: {required}"
            );
        }
        for required in [
            "exact Candidate plan approval",
            "FactDelta proposals",
            "opened_next_wave",
            "closed_no_delta",
            "exhausted",
            "residual risk",
            "static operation-graph back-edge",
        ] {
            assert!(
                verification_methodology.contains(required),
                "missing Verification methodology contract: {required}"
            );
        }

        let graph: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../resources/harness/graph/operation_graph.json"
        )))
        .expect("operation graph parses");
        let has_static_wave_back_edge = graph["edges"].as_array().is_some_and(|edges| {
            edges
                .iter()
                .any(|edge| edge["from"] == "verification" && edge["to"] == "attack_candidate")
        });
        assert!(
            !has_static_wave_back_edge,
            "durable Candidate waves must not add a static operation-graph back-edge"
        );
    }
}
