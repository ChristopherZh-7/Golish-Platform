//! Operation Harness · Phase 1c skeleton.
//!
//! 把 chat panel 的 task 模式重构为 1-stage harness (external_attack_surface).
//! 来源:
//!   - `docs/design/2026-05-26-stage-harness-mvp-external-attack-surface.md` (Doc 3)
//!   - `docs/design/2026-05-26-evidence-ledger-on-existing-audit-log.md` (Doc 1)
//!   - `docs/design/2026-05-26-mcp-resource-evidence-summary.md` (Doc 2)
//!
//! 子模块分层 (Doc 3 §5.2):
//!
//! ```text
//! harness/
//! ├── mod.rs                   re-export + module 注释 (本文件)
//! ├── types.rs                 共享 DTO (StageKind / AuthorizationLevel / IntentAxis / GateResult / ...)
//! ├── profile.rs               Profile (assessment / pentest / ...) + JSON loader
//! ├── stage_spec.rs            StageSpec (external_attack_surface / ...) + JSON loader
//! ├── operation_graph.rs       Base Operation DAG loader + profile 投影 + next_stages (Doc 3 §3)
//! ├── nl_slice.rs              终态 4 字段 NlSlice (Doc 3 §6)
//! ├── intent_classifier.rs     deterministic 词库 classifier (Doc 3 §6.1, Task 1c.3 完整)
//! ├── pre_action_authorizer.rs 每 tool call 前 authz 检查
//! ├── stage_harness.rs         主入口 · StageHarness::for_stage + validate_gate
//! ├── stage_transition.rs      gate 结果 → 下一 stage 决策 (Doc 3 §6.2)
//! ├── sprint_contract.rs       Sprint Contract DTO + Generator (Task 1c.4)
//! └── gate/
//!     ├── mod.rs               6 个 check 调度
//!     ├── schema_check.rs      deliverable schema 完整性
//!     ├── scope_check.rs       evidence_refs[*] 当前 label = InScope
//!     ├── contract_check.rs    findings 数量在 Sprint Contract range 内
//!     ├── vacuous_check.rs     vacuous detector (no_tool / fake / skip pattern)
//!     └── freshness_check.rs   evidence as_of_timestamp + max_age 比较
//! ```
//!
//! Phase 1c.2 阶段交付 = skeleton (types + 主流程函数签名 + stub return).
//! Phase 1c.3 + 1c.4 + 1c.5 在此基础上填实 IntentClassifier 词库 / SprintContract /
//! gate check.

pub mod chain_wave;
pub mod eval;
pub mod evidence_facts;
pub mod feature_flags;
pub mod gate;
pub mod graph_engine;
pub mod guardrail;
pub mod intent_classifier;
pub mod nl_slice;
pub mod operation_continuity;
pub mod operation_flow;
pub mod operation_graph;
pub mod operation_mermaid;
pub mod org_gate;
pub mod phase;
pub mod phase_flow;
pub mod pre_action_authorizer;
pub mod profile;
pub mod rag_prior;
pub mod resources;
pub mod slice;
pub mod sprint_contract;
pub mod stage_capability;
pub mod stage_fanout;
pub mod stage_harness;
pub mod stage_spec;
pub mod stage_transition;
pub mod surface_mapping;
pub mod technique_resolver;
pub mod technique_taxonomy;
pub mod tool_taxonomy;
pub mod types;
pub mod wstg_mapping;

#[cfg(test)]
mod e2e_tests;

pub use gate::context_builder::GateContextBuilder;
pub use gate::freshness_check::freshness_age_reasons;
pub use gate::rule_engine::{EvidenceFact, EvidenceOutcome, GateContext, SourceQueryFact};
pub use gate::{
    validate_external_attack_surface_gate, validate_stage_gate, validate_stage_gate_with_context,
    validate_stage_gate_with_skeleton, GateCheckOutcome, GateResult,
};
pub use intent_classifier::{IntentClassifier, IntentClassifierConfig};
pub use nl_slice::NlSlice;
pub use operation_continuity::{
    plan_adoption_cursor, ContinuityAdoptionPlan, ContinuityDecision, ContinuitySnapshot,
    StageReuseStatus, StageReuseSummary,
};
pub use operation_graph::{
    base_operation_graph, load_operation_graph_from_json, AllowedDag, OperationGraph,
    OperationGraphError, StageEdge,
};
pub use org_gate::{evaluate_org_stage_gate, OrgVerdict};
pub use phase::{load_phase_map_from_json, Phase, PhaseMap, PhaseMapError};
pub use phase_flow::{
    decide_phase_step, next_phase, pending_phase_approval, phase_is_complete, PhaseStep,
};
pub use pre_action_authorizer::{AuthorizationError, HarnessAuthz, PreActionAuthorizer};
pub use profile::{
    load_profile_from_json, ApprovalPolicy, AuthorizationLevel, Profile, ProfileLoadError,
};
pub use resources::{
    load_embedded_phase_map, load_embedded_profile, load_embedded_sprint_skeleton,
    load_embedded_stage_spec, profile_json, sprint_skeleton_json, stage_methodology_md,
    stage_spec_json, EMBEDDED_PROFILE_IDS,
};
pub use slice::resolve_slice;
pub use sprint_contract::{
    expected_techniques_for_target_types, DefaultSprintContractGenerator, ExpectedFinding,
    SprintContract, SprintContractGenerator, SprintSkeleton, StageSkeleton,
};
pub use stage_capability::{
    capabilities_for_stage, capabilities_for_technique, capability_by_id, stage_for_technique,
    suggested_capabilities_for_any_technique, suggested_capabilities_for_technique,
    suggested_tools_for_any_technique, suggested_tools_for_technique, tools_from_suggestions,
    CapabilityRisk, CapabilityRunnerKind, StageCapabilitySpec, StageCapabilitySuggestion,
};
pub use stage_harness::StageHarness;
pub use stage_spec::{
    load_stage_spec_from_json, HumanApprovalPolicy, InheritsEvidenceFrom, StageSpec,
    StageSpecLoadError,
};
pub use stage_transition::{
    decide_from_gate, decide_transition, stage_entry_requires_approval, TransitionDecision,
};
pub use surface_mapping::{
    missing_required_categories, SurfaceCategory, SurfaceCoverage, D2_REQUIRED_CATEGORIES,
    D2_SOFT_CATEGORIES,
};
pub use technique_taxonomy::{
    is_recognized as is_recognized_technique, load_technique_taxonomy, lookup as lookup_technique,
    TechniqueMeta,
};
pub use tool_taxonomy::{
    allowed_tool_names, is_offensive_sub_agent, is_scan_invocation, is_scan_tool_name,
    stage_allows, tool_category, underlying_tool_name,
};
pub use types::{
    AgentContinuity, AttackCandidate, CandidateDisposition, CandidatePriority, CoverageCell,
    CoverageGapAction, CoverageStatus, ExternalAttackSurfaceDeliverable, FindingSeverity,
    HarnessFinding, HarnessRecoveryActions, HarnessStageHint, IntentAxis, RiskLevel,
    SkippedCheckRecord, StageClaim, StageDeliverable, StageKind,
};

/// Operation profile selection for stage_mode (Phase C).
///
/// Default `assessment`. Override via `GOLISH_HARNESS_PROFILE=<id>` where `<id>`
/// is one of the embedded profile ids (`assessment` / `pentest` / `red_team` /
/// `bug_bounty` / `cloud_assessment`). Cached once at first read (LazyLock). The
/// id is NOT validated here; operation startup
/// validates it against the embedded registry and falls back to `assessment` so a
/// typo cannot wedge the cursor with an unknown profile.
pub fn active_profile_id() -> &'static str {
    use std::sync::LazyLock;
    static PROFILE: LazyLock<String> = LazyLock::new(read_env_profile);
    PROFILE.as_str()
}

fn read_env_profile() -> String {
    std::env::var("GOLISH_HARNESS_PROFILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "assessment".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_env_profile_defaults_to_assessment_when_unset() {
        if std::env::var("GOLISH_HARNESS_PROFILE").is_ok() {
            return;
        }
        assert_eq!(read_env_profile(), "assessment");
    }

    #[test]
    fn all_selectable_profiles_load_from_registry() {
        // Startup invariant: every id a user can pick via GOLISH_HARNESS_PROFILE
        // must resolve in the embedded registry, and the assessment fallback must
        // always exist.
        for id in [
            "assessment",
            "pentest",
            "red_team",
            "bug_bounty",
            "cloud_assessment",
        ] {
            assert!(
                load_embedded_profile(id).unwrap().is_some(),
                "selectable profile '{id}' must load from embedded registry"
            );
        }
        // active_profile_id() either resolves directly or the startup falls back
        // to assessment (proven loadable above).
        let active = active_profile_id();
        assert!(
            load_embedded_profile(active).unwrap().is_some()
                || load_embedded_profile("assessment").unwrap().is_some()
        );
    }
}
