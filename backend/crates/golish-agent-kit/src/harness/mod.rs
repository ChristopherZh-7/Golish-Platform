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
pub mod operation_graph;
pub mod operation_mermaid;
pub mod pre_action_authorizer;
pub mod profile;
pub mod resources;
pub mod sprint_contract;
pub mod stage_harness;
pub mod stage_spec;
pub mod stage_transition;
pub mod surface_mapping;
pub mod types;

#[cfg(test)]
mod e2e_tests;

pub use gate::{
    validate_external_attack_surface_gate, validate_stage_gate, GateCheckOutcome, GateResult,
};
pub use intent_classifier::{IntentClassifier, IntentClassifierConfig};
pub use nl_slice::NlSlice;
pub use operation_graph::{
    base_operation_graph, load_operation_graph_from_json, AllowedDag, OperationGraph,
    OperationGraphError, StageEdge,
};
pub use pre_action_authorizer::{AuthorizationError, HarnessAuthz, PreActionAuthorizer};
pub use profile::{
    load_profile_from_json, ApprovalPolicy, AuthorizationLevel, Profile, ProfileLoadError,
};
pub use resources::{
    load_embedded_profile, load_embedded_stage_spec, profile_json, stage_spec_json,
    EMBEDDED_PROFILE_IDS,
};
pub use sprint_contract::{
    DefaultSprintContractGenerator, ExpectedFinding, SprintContract, SprintContractGenerator,
    SprintSkeleton,
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
