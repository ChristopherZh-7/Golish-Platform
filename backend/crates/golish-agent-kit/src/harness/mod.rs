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

pub mod eval;
pub mod gate;
pub mod graph_engine;
pub mod guardrail;
pub mod intent_classifier;
pub mod nl_slice;
pub mod operation_flow;
pub mod operation_graph;
pub mod operation_mermaid;
pub mod phase;
pub mod phase_flow;
pub mod pre_action_authorizer;
pub mod profile;
pub mod rag_prior;
pub mod resources;
pub mod sprint_contract;
pub mod stage_harness;
pub mod stage_spec;
pub mod stage_transition;
pub mod surface_mapping;
pub mod tool_taxonomy;
pub mod types;

#[cfg(test)]
mod e2e_tests;

pub use gate::freshness_check::freshness_age_reasons;
pub use gate::{
    validate_external_attack_surface_gate, validate_stage_gate, validate_stage_gate_with_skeleton,
    GateCheckOutcome, GateResult,
};
pub use intent_classifier::{IntentClassifier, IntentClassifierConfig};
pub use nl_slice::NlSlice;
pub use operation_graph::{
    base_operation_graph, load_operation_graph_from_json, AllowedDag, OperationGraph,
    OperationGraphError, StageEdge,
};
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
    load_embedded_stage_spec, profile_json, sprint_skeleton_json, stage_spec_json,
    EMBEDDED_PROFILE_IDS,
};
pub use sprint_contract::{
    DefaultSprintContractGenerator, ExpectedFinding, SprintContract, SprintContractGenerator,
    SprintSkeleton, StageSkeleton,
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
pub use tool_taxonomy::{is_scan_invocation, stage_allows, tool_category, underlying_tool_name};
pub use types::{
    AgentContinuity, ExternalAttackSurfaceDeliverable, FindingSeverity, HarnessFinding,
    HarnessRecoveryActions, HarnessStageHint, IntentAxis, RiskLevel, SkippedCheckRecord,
    StageClaim, StageDeliverable, StageKind,
};

/// Feature flag: 启用 stage_mode 路径 (Operation Harness).
///
/// **默认 ON** (2026-06-01 起): task 模式默认走 Operation Harness
/// (Profile → Operation DAG → Stage gate → Evidence Ledger).
///   - 逃生阀 (kill switch): 设环境变量 `GOLISH_HARNESS_STAGE_MODE=false`
///     (或 `0` / `off` / `no`, 大小写不敏感) 即回退到旧 task_orchestrator 路径,
///     无需改代码即可线上快速回退.
///   - 其它任何值 (未设 / `true` / `1` / 乱填) = 开.
///   - 启动时 LazyLock 缓存一次, 避免每次 subtask 都查 env.
///   - Phase 2 计划: 从 settings.toml 的 `harness.stage_mode_enabled` 读取,
///     与 LangFuse / proxy 等 settings 同源 (env 作为覆盖).
///
/// 历史: 此前默认 OFF, 作为 harness MVP 与旧路径并行的灰度过渡; 多 stage / 多 profile
/// 运行时 (Phase A/B/C) 落地后按设计终态翻为默认 ON.
pub fn stage_mode_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(read_env_flag);
    *ENABLED
}

/// Feature flag: 启用 `submit_stage_deliverable` 工具（确定性交付通道）。
///
/// **默认 ON**。Kill switch: 设环境变量 `GOLISH_HARNESS_SUBMIT_TOOL=false`
/// (或 `0`/`off`/`no`) 回退到「在最终消息打印 ```json」的文本路径。复用
/// [`parse_stage_mode_flag`] 的「仅显式 falsey 关闭」语义；首次读 LazyLock 缓存。
pub fn submit_tool_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_stage_mode_flag(std::env::var("GOLISH_HARNESS_SUBMIT_TOOL").ok().as_deref())
    });
    *ENABLED
}

/// Feature flag: 启用 sprint-skeleton 驱动的 **per-target gate 强校验**
/// (`expected_count_range` 每类 finding 数量区间 + skeleton 的 `min_tool_invocations`)。
///
/// **默认 OFF**（opt-in 灰度）：energize 后 gate 会按 profile 的 sprint_skeleton 强制
/// 每个目标的 finding 数量区间，可能让此前结构性 PASS 的 stage 转 BLOCK，需要先在真机
/// 上观察。设 `GOLISH_HARNESS_SPRINT_SKELETON=1`（或 `true`/`on`/`yes`）开启；首次读
/// LazyLock 缓存。
pub fn sprint_skeleton_enforcement_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_truthy_flag(
            std::env::var("GOLISH_HARNESS_SPRINT_SKELETON")
                .ok()
                .as_deref(),
        )
    });
    *ENABLED
}

/// Feature flag: 启用 evidence「新鲜度」回查（P0 Task 6 收口）。
///
/// **默认 OFF**（opt-in）：开启后 gate 收口阶段会按 evidence 真实 age 拦截**硬过期**
/// 证据（age ≥ 2×max_age → BLOCK；软陈旧只 warn 不阻断）。默认关，因长跑 / 跨 resume
/// 的 operation 证据可能合法变旧，需先在真机观察。设
/// `GOLISH_HARNESS_EVIDENCE_FRESHNESS=1`（或 `true`/`on`/`yes`）开启；首次读 LazyLock 缓存。
pub fn evidence_freshness_enforcement_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_truthy_flag(
            std::env::var("GOLISH_HARNESS_EVIDENCE_FRESHNESS")
                .ok()
                .as_deref(),
        )
    });
    *ENABLED
}

/// Feature flag: 启用**类别白名单**的 per-stage 工具边界（deny-by-default）。
///
/// 工具边界从每关 `StageSpec.allowed_tool_types` 的类别选择器（`recon` /
/// `recon/dns` / 具体工具名）强制——**只对扫描工具生效**（agent/meta 工具豁免），
/// 不在白名单里的扫描类工具一律拦（主 agent 路 + 子代理路），见
/// [`tool_taxonomy::stage_allows`] / [`tool_taxonomy::is_scan_invocation`]。
///
/// **默认 ON**（旧的 `forbidden_tools` 黑名单已删，这是唯一的工具边界）。Kill switch:
/// 设 `GOLISH_HARNESS_TOOL_WHITELIST=false`（或 `0`/`off`/`no`）关闭 per-stage 扫描
/// 限制（紧急绕过；`GOLISH_HARNESS_STAGE_MODE=false` 是更彻底的总开关）。其它任何值
/// （未设 / `true` / `1`）= 开。首次读 LazyLock 缓存。
pub fn tool_whitelist_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_stage_mode_flag(
            std::env::var("GOLISH_HARNESS_TOOL_WHITELIST")
                .ok()
                .as_deref(),
        )
    });
    *ENABLED
}

/// Feature flag: 启用两级阶段模型（大阶段 Phase × 小阶段 Stage）的 phase-aware
/// 流转（设计 `docs/design/2026-06-03-two-level-phase-stage-model.md`）。
///
/// **默认 OFF**（opt-in 灰度）：开启后，人工审批从「每 stage」收敛到「跨大阶段边界」
/// （`active_scan`＝prep→active_recon、`exploit_validation`＝active_recon→vuln，de-dup），
/// 大阶段内小阶段不再各自卡审批。关闭 = 现状 per-stage 审批，零回归。设
/// `GOLISH_HARNESS_TWO_LEVEL=1`（或 `true`/`on`/`yes`）开启；首次读 LazyLock 缓存。
pub fn two_level_enabled() -> bool {
    use std::sync::LazyLock;
    static ENABLED: LazyLock<bool> = LazyLock::new(|| {
        parse_truthy_flag(std::env::var("GOLISH_HARNESS_TWO_LEVEL").ok().as_deref())
    });
    *ENABLED
}

fn read_env_flag() -> bool {
    parse_stage_mode_flag(std::env::var("GOLISH_HARNESS_STAGE_MODE").ok().as_deref())
}

/// Pure parser (env-independent → 可单测全分支). 默认 ON: 仅显式 falsey 值关闭;
/// 未设 / 未知 / truthy 值一律开.
fn parse_stage_mode_flag(value: Option<&str>) -> bool {
    !matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("false" | "0" | "off" | "no")
    )
}

/// Pure parser (env-independent → 可单测全分支). 默认 OFF: 仅显式 truthy 值开启;
/// 未设 / 未知 / falsey 值一律关. 用于 opt-in 灰度 flag.
fn parse_truthy_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("true" | "1" | "on" | "yes")
    )
}

/// Operation profile selection for stage_mode (Phase C).
///
/// Default `assessment`. Override via `GOLISH_HARNESS_PROFILE=<id>` where `<id>`
/// is one of the embedded profile ids (`assessment` / `pentest` / `red_team` /
/// `bug_bounty` / `cloud_assessment`). Cached once at first read (LazyLock), same
/// as [`stage_mode_enabled`]. The id is NOT validated here; operation startup
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
    fn stage_mode_enabled_default_on() {
        // 默认 ON. 若运行环境显式设了 GOLISH_HARNESS_STAGE_MODE 则跳过 (避免 env 干扰).
        if std::env::var("GOLISH_HARNESS_STAGE_MODE").is_ok() {
            return;
        }
        assert!(stage_mode_enabled(), "default must be on");
    }

    #[test]
    fn parse_stage_mode_flag_unset_defaults_on() {
        assert!(parse_stage_mode_flag(None), "unset must default ON");
    }

    #[test]
    fn parse_stage_mode_flag_explicit_falsey_disables() {
        for off in ["false", "0", "off", "no", "FALSE", " False ", "OFF", "No"] {
            assert!(!parse_stage_mode_flag(Some(off)), "'{off}' must disable");
        }
    }

    #[test]
    fn parse_stage_mode_flag_truthy_or_unknown_enables() {
        for on in ["true", "1", "TRUE", "yes", "on", "garbage", ""] {
            assert!(parse_stage_mode_flag(Some(on)), "'{on}' must enable");
        }
    }

    #[test]
    fn parse_truthy_flag_default_off() {
        assert!(!parse_truthy_flag(None), "unset must default OFF");
        for off in ["false", "0", "off", "no", "garbage", ""] {
            assert!(!parse_truthy_flag(Some(off)), "'{off}' must stay OFF");
        }
    }

    #[test]
    fn parse_truthy_flag_explicit_truthy_enables() {
        for on in ["true", "1", "on", "yes", "TRUE", " On ", "YES"] {
            assert!(parse_truthy_flag(Some(on)), "'{on}' must enable");
        }
    }

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
