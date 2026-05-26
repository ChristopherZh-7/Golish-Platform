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
//! ├── nl_slice.rs              终态 4 字段 NlSlice (Doc 3 §6)
//! ├── intent_classifier.rs     deterministic 词库 classifier (Doc 3 §6.1, Task 1c.3 完整)
//! ├── pre_action_authorizer.rs 每 tool call 前 authz 检查
//! ├── stage_harness.rs         主入口 · StageHarness::for_stage + validate_gate
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

pub mod gate;
pub mod intent_classifier;
pub mod nl_slice;
pub mod pre_action_authorizer;
pub mod profile;
pub mod sprint_contract;
pub mod stage_harness;
pub mod stage_spec;
pub mod types;

pub use gate::{
    validate_external_attack_surface_gate, GateCheckOutcome, GateResult,
};
pub use intent_classifier::{IntentClassifier, IntentClassifierConfig};
pub use nl_slice::NlSlice;
pub use pre_action_authorizer::{AuthorizationError, PreActionAuthorizer};
pub use profile::{
    load_profile_from_json, ApprovalPolicy, AuthorizationLevel, Profile, ProfileLoadError,
};
pub use sprint_contract::{
    ExpectedFinding, SprintContract, SprintContractGenerator, SprintSkeleton,
};
pub use stage_harness::StageHarness;
pub use stage_spec::{
    load_stage_spec_from_json, HumanApprovalPolicy, InheritsEvidenceFrom, StageSpec,
    StageSpecLoadError,
};
pub use types::{
    AgentContinuity, ExternalAttackSurfaceDeliverable, Finding, FindingSeverity,
    HarnessRecoveryActions, HarnessStageHint, IntentAxis, RiskLevel, SkippedCheckRecord,
    StageClaim, StageKind,
};

/// Feature flag: 启用 stage_mode 路径 (Task 1c.7 完整).
///
/// Phase 1 默认 OFF · 旧 task_orchestrator 路径继续工作; harness 路径与之并行
/// 等 Phase 2+ 跑数据稳定后再 flip on.
pub fn stage_mode_enabled() -> bool {
    false
}
