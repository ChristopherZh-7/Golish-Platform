# Hypothesis Registry 与 Candidate Analysis Team 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在不改变历史 operation authority 的前提下，交付 operation-frozen 五态 rollout、canonical Hypothesis Registry、每公司两波只读 Candidate Analysis Team、确定性 Gate、旧 Candidate/NoCandidate 兼容投影，以及 Plan D 前可审计 Registry 的最小只读 UI。

**架构：** Registry 是唯一 canonical hypothesis ledger，旧 `attack_candidates` 只在冻结 rollout 允许时保持 legacy authority；新权威模式的 legacy Candidate/Attempt compatibility view 只能在 canonical transaction 提交 immutable outbox 后由独立 projector 异步派生，投影失败不回滚 canonical truth，旧 consumer 必须 fail closed。Candidate 使用一个 company-scoped Controller、2–8 条 live 只读 analyst/critic lane，以及server-owned multi-root Checked Tool Truth bundle、temporal-validity/signed-feed snapshot、input chunks、H1、分区coverage subreview/synthesis与H2 exact censuses；LLM 只提交 typed artifact，semantic key、root/revision、claim components、VerificationContract、HypothesisVerificationPlan、merge、readiness、generation seal 和 projection 都由服务端 reducer/Gate 决定。Candidate writer 不能产生 `verified/refuted`，`invalid` 也只能由 server validator 写入；Plan C 以后只能通过 revision-level `HypothesisRevisionAdjudication` + exact B-owned `HypothesisVerificationPlanV1` objective/claim-component outcomes + transition receipt 接管验证终态，单个 Campaign terminal 从来不是充分 authority。Plan B 不实现或调用 Verification Campaign、Prepared Action、typed oracle 或主动 FactDelta 循环；在新权威 mode 下暂以 `plan_c_verification_unavailable` residual 收口并转 Reporting。

**技术栈：** Rust 2021、Tokio、SQLx/PostgreSQL、Tauri 2、ts-rs、React 19、TypeScript 6、Vitest、Biome。

**规格来源：** `docs/design/2026-07-29-tool-truth-hypothesis-verification-loop.md` §5、§6、§10、§12、§14.1、§16 Plan B、§17.2、§17.5。

---

## 实施前硬边界与授权暂停点

1. 当前仓库只能有一个 `feature_list.json` 条目处于 `in_progress`。执行本计划前，先按 `AGENTS.md` 把当前功能完成、阻塞或退回 `not_started`，再登记并选中 Plan B；不要在实现中制造第二个 `in_progress`。
2. **PAUSE A — schema 授权：** 设计批准不等于 migration 授权。开始 Task 2、创建或应用 `20260729000006_hypothesis_registry.sql` 前，必须再次取得用户对 DB schema/migration 的明确确认。未确认则停在 Task 1，不创建 migration。
3. **PAUSE B — generated IPC 授权：** 开始 Task 11 的 ts-rs 导出、生成 `frontend/lib/generated/` 文件或改变已发布 IPC 类型链前，必须再次取得用户明确确认；这明确包括 `ProjectionEntityKind.ts`、`ProjectionInvalidationReason.ts`、`TimelineEventKind.ts`、`ProjectionSourceTimeStatusV1.ts` 四个由 Plan B core enum 生成的 bindings/golden files。未确认时后端 DTO 与四个 enum 可以停留在未导出分支或只运行内存 `TS::decl()` golden test，不能写出或手改 generated 文件。
4. 本计划不授权 rollout promotion。`investigation_rollout` 始终以 `legacy_candidate_v1 + legacy_only` 为初始值；Plan B 不提供对外 promotion command，也不修改已有 operation 的冻结 mode。
5. 本计划不实现、不调度、也不伪造 Campaign、Prepared Action、authorization packet、action execution 或 oracle。`shadow_registry` / `dual_read_compare` 不能授权新执行；`registry_authoritative_legacy_projection` / `new_only` 在 Registry seal 后写显式 residual，不得回落调用旧 verifier 作为 authority。
6. 每个 Task 下的 **Future Commit** 只是未来实施时的原子边界。本轮编写计划不执行 `git add`、`git commit`、`git push`，也不运行任何测试。
7. 实施阶段每批 Cargo build/test/clippy 前先运行 `just space-guard`；未经用户明确授权，不运行 `./init.sh`、`just precommit`、`just check`、全 workspace test 或真实目标/provider 调用。

## 冻结 rollout 矩阵

| rollout mode | operation contract | canonical Candidate writer / Gate | Registry 行为 | legacy mutation | compatibility 行为 | Plan B 后续 |
|---|---|---|---|---|---|---|
| `legacy_only` | `legacy_candidate_v1` | legacy Candidate/NoCandidate | 不写 Registry | 允许 | API 只读映射 legacy；缺字段为 `legacy_unavailable` | 维持旧 Verification |
| `shadow_registry` | `hypothesis_registry_v1` | legacy Candidate/NoCandidate | legacy source transaction 只写 outbox；projector 异步构建 shadow Registry | 允许 | divergence 仅审计并阻止未来 promotion | 维持旧 Verification；Registry 不授权 |
| `dual_read_compare` | `hypothesis_registry_v1` | legacy Candidate/NoCandidate | 完整记录 exact compare；禁止 field fallback | 允许 | mismatch/incomplete 仅审计并阻止未来 promotion | 维持旧 Verification；Registry 不授权 |
| `registry_authoritative_legacy_projection` | `hypothesis_registry_v1` | Registry Candidate Gate | Registry canonical transaction只写typed outbox header/member及outbox-owned immutable source blob | 禁止 | projector 异步派生 Candidate/Attempt version；失败不回滚 canonical，旧 consumer fail closed | 写 `plan_c_verification_unavailable` residual，转 Reporting |
| `new_only` | `hypothesis_registry_v1` | Registry Candidate Gate | Registry canonical transaction只写typed outbox header/member及outbox-owned immutable source blob | 禁止 | 不生成新 legacy compatibility versions，只读历史 projection | 写 `plan_c_verification_unavailable` residual，转 Reporting |

唯一 final policy 的 `allow_prepared_action_jit` 在前三态为 `false`、后两态为 `true`；Plan B 只负责提前冻结该最终语义，不能因为尚未实现 Campaign 而把它改成临时矩阵。由于 Plan B 本身没有 Campaign scheduler/executor，Plan B runtime 的 `authorizes_hypothesis_execution` 仍始终为 `false`，并在后两态写 `plan_c_verification_unavailable` residual；Plan C 落地后才由同一 policy + joint admission guard 接管新 authoritative operation。

---

## 文件结构

### 新建文件

- `backend/crates/golish-core/src/investigation_contract.rs`：contract/mode、精确 rollout policy 和稳定错误码。
- `backend/crates/golish-core/src/verification_contract.rs`：Plan B/C唯一`VerificationContractV1`、combinator/binding闭集、canonical serializer/hash与host-only constructor inputs。
- `backend/crates/golish-core/src/hypothesis_verification.rs`：Plan B/C唯一`HypothesisClaimComponentV1`、full-claim `HypothesisVerificationPlanV1` objective/path/falsifier exact sets、revision-level `HypothesisRevisionAdjudication`、transition receipt binding与validators。
- `backend/crates/golish-core/src/investigation_comparison.rs`：唯一 `comparison_record.v1`、canonical serializer 与 golden hash。
- `backend/crates/golish-core/src/investigation_projection.rs`：`ProjectionEntityKind / ProjectionChangeKind / TimelineEventKind / ProjectionInvalidationReason / ProjectionSourceTimeStatusV1` 闭集、字符串映射、四个 ts-rs binding ownership 与 exhaustive catalog/golden tests。
- `backend/crates/golish-db/migrations/20260729000006_hypothesis_registry.sql`：Plan B 唯一 forward-only migration。
- `backend/crates/golish-db/src/repo/investigation_rollout.rs`：只读 deployment default；Plan B 无 promote API。
- `backend/crates/golish-db/src/repo/operation_rollout.rs`：Tool Truth + Investigation 合法 pair/rank、operation creation/fork admission 与统一 adoption receipt；无 default mutation API。
- `backend/crates/golish-db/src/repo/hypothesis_registry.rs`：root/revision/event/generation 的锁、CAS 与原子 apply。
- `backend/crates/golish-db/src/repo/candidate_analysis.rs`：multi-root Checked bundle/temporal/signed-feed snapshot、分页 receipt、work item、artifact、H1/H2、coverage partition/subreview/synthesis与conflict component censuses。
- `backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs`：projector transaction 内异步派生 append-only Candidate/Attempt compatibility entity versions，并构造 canonical/legacy 两侧完整 comparison record；只调用 Plan B 唯一 comparator，不拥有 compare 算法或 hash authority。
- `backend/crates/golish-db/src/repo/investigation_projection/mod.rs`：repeatable-read snapshot 编排与对外 repo 入口；Plan D 在此原位扩展。
- `backend/crates/golish-db/src/repo/investigation_projection/projector.rs`：按 outbox batch exact set 原子物化全部 entity versions/change/timeline/invalidation，并一次 CAS operation head。
- `backend/crates/golish-db/src/repo/investigation_projection/types.rs`：projection rows、page keys 与 sealed read authority。
- `backend/crates/golish-db/src/repo/investigation_projection/summary.rs`：operation/generation/count summary 查询。
- `backend/crates/golish-db/src/repo/investigation_projection/hypotheses.rs`：stable list/detail 与 lineage/source 聚合。
- `backend/crates/golish-db/src/repo/investigation_projection/legacy.rs`：legacy Candidate/NoCandidate 只读映射与 `legacy_unavailable`。
- `backend/crates/golish-db/src/repo/investigation_projection/comparison.rs`：唯一 per-record compare writer/reader；Plan D 原位扩展 aggregate query。
- `backend/crates/golish-agent-kit/src/db_traits/hypothesis_registry.rs`：runtime 使用的窄 DB port 与 typed views。
- `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/{mod.rs,types.rs,semantic_key.rs,verification_contract_compiler.rs,verification_plan_compiler.rs,reducer.rs,candidate_gate.rs,rollout.rs}`：消费golish-core唯一VerificationContract/HypothesisVerificationPlan的host compiler wrapper、identity、reducer和Gate；不重定义contract/combinator/plan/path/hash/validator。
- `backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs`：Controller/analyst/critic typed schemas、runner 和 runtime trait。
- `backend/crates/golish-agent-app/src/ai/db_bridge/hypothesis_registry.rs`：kit DB port 到 golish-db repo 的转换。
- `backend/crates/golish-agent-app/src/ai/candidate_analysis_projection.rs`：snapshot-pinned、bounded、`instruction_authority=false` 的模型输入投影。
- `backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs`：authorities 重载、纯 Gate 调用和原子 finalizer。
- `backend/crates/golish-agent-app/src/ai/candidate_analysis_runtime.rs`：两波调度与 recovery。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_analysis_agent_runner.rs`：tool-free bound subagent 执行器。
- `backend/crates/golish-sub-agents/src/defaults/prompts/hypothesis_analysis.rs`：三个静态 agent 的闭合 prompt。
- `backend/crates/golish-agent-app/src/ai/commands/investigation/{mod.rs,dto.rs,cursor.rs}`：三个 Plan-B-owned command、DTO 与 cursor；Plan D 在此目录原位扩展。
- `backend/crates/golish/src/commands_facade/investigation.rs`：Tauri command facade。
- `backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs`：semantic identity/reducer/Gate 测试。
- `backend/crates/golish-db/tests/hypothesis_registry.rs`：migration、immutability、atomicity、projection 测试。
- `backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs`：两波、2–8 live lane、recovery 测试。
- `backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs`：IDOR、scope、cursor、deleted-target 测试。
- `frontend/lib/api/investigation.ts`：三个只读 API wrapper。
- `frontend/components/Engagement/HypothesisRegistryAudit.tsx`：Plan D 前最小审计面板。
- `frontend/components/Engagement/HypothesisRegistryAudit.test.tsx`：loading/error/empty/stale/mode/residual 回归。

### 修改文件

- Core/DB：`backend/crates/golish-core/src/lib.rs`、`backend/crates/golish-db/src/repo/{mod.rs,operation_state.rs,runtime_memory_tx.rs,attack_candidates.rs,attack_candidate_approvals.rs,candidate_attempts.rs,stage_teams.rs}`。
- Kit：`backend/crates/golish-agent-kit/src/{db_traits/mod.rs,db_traits/runtime_memory.rs,db_traits/types.rs,harness/mod.rs,harness/stage_spec.rs,task_orchestrator/mod.rs}`。
- App/runtime/bridge：`backend/crates/golish-agent-app/src/ai/{mod.rs,db_bridge/mod.rs,db_bridge/runtime_memory.rs,commands/mod.rs,commands/attack.rs,commands/bridge_config.rs,tracking_bridge/chain.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,tool_execution/direct/mod.rs,tool_execution/direct/stage_run_call.rs,tool_execution/direct/stage_team_scheduler.rs}`、`backend/crates/golish-agent-runtime/src/{test_utils/context.rs,eval_support/single_turn.rs,eval_support/multi_turn.rs}`、`backend/crates/golish-agent-bridge/src/{agent_bridge/mod.rs,agent_bridge/config.rs,agent_bridge/prepare.rs,agent_bridge/constructors/mod.rs,bridge_executor/trait_impl.rs}`。
- Subagents/spec：`backend/crates/golish-sub-agents/src/defaults/{prompts/mod.rs,builder/mod.rs,builder/registry.rs,tests.rs}`、`backend/crates/golish-sub-agents/src/executor/{tool_setup.rs,prompt_assembly.rs,response_parsing.rs,stream_processing.rs,inner.rs}`、`resources/harness/stages/attack_candidate/spec.json`。
- IPC/UI：`backend/Cargo.toml`、`backend/crates/golish-agent-app/Cargo.toml`、`backend/crates/golish/src/commands_facade/mod.rs`、`backend/crates/golish/src/commands_registry.rs`、`frontend/lib/api/{index.ts,error-codes.ts}`、`frontend/components/Engagement/index.ts`、`frontend/components/ToolCallDetailView/{ToolCallDetailView.tsx,ToolCallDetailView.candidate.test.tsx}`。
- 收尾文档：`docs/modules/backend/golish-core.md`、`docs/modules/backend/golish-db.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-kit.md`、`docs/modules/backend/golish-agent-kit/{harness.md,task_orchestrator.md,db_traits.md}`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-bridge/{agent_bridge.md,bridge_executor.md}`、`docs/modules/backend/golish-sub-agents/{defaults.md,executor.md}`、`docs/modules/frontend/{lib.md,components.md}`、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`。

### 生成文件（PAUSE B 后由 ts-rs 生成，禁止手改）

- `frontend/lib/generated/InvestigationScopeRequest.ts`
- `frontend/lib/generated/InvestigationHypothesisListRequest.ts`
- `frontend/lib/generated/InvestigationHypothesisGetRequest.ts`
- `frontend/lib/generated/InvestigationSummaryView.ts`
- `frontend/lib/generated/InvestigationHypothesisListView.ts`
- `frontend/lib/generated/InvestigationHypothesisListItemView.ts`
- `frontend/lib/generated/InvestigationHypothesisDetailView.ts`
- `frontend/lib/generated/InvestigationTemporalSnapshotView.ts`
- `frontend/lib/generated/InvestigationProjectionEnvelope.ts`
- `frontend/lib/generated/InvestigationModePolicyView.ts`
- `frontend/lib/generated/InvestigationCommandError.ts`
- `frontend/lib/generated/ProjectionEntityKind.ts`
- `frontend/lib/generated/ProjectionInvalidationReason.ts`
- `frontend/lib/generated/TimelineEventKind.ts`
- `frontend/lib/generated/ProjectionSourceTimeStatusV1.ts`

---

## Task 1：建立纯 investigation contract 与五态矩阵

**文件：**

- 创建：`backend/crates/golish-core/src/investigation_contract.rs`
- 修改：`backend/crates/golish-core/src/lib.rs`

### Step 1：先写 RED matrix tests

在新文件底部先写测试，暂不定义被引用类型：

```rust
#[cfg(test)]
mod tests {
    use super::{
        CampaignWritePolicy, ComparePolicy, InvestigationAuthority,
        InvestigationContractVersion, InvestigationErrorCode, InvestigationRolloutMode,
        LegacyProjectionPolicy,
    };

    #[test]
    fn investigation_rollout_matrix_is_the_single_final_policy() {
        use CampaignWritePolicy::{Canonical, CompareOnly, Off, ShadowAudit};
        use ComparePolicy::{AuditOnly, Off as CompareOff, PromotionBlocking, WholeRecordExact};
        use InvestigationAuthority::{Legacy, Registry};
        use LegacyProjectionPolicy::{CanonicalDerivedFailClosed, HistoricalReadOnly, Native};
        let expected = [
            (InvestigationRolloutMode::LegacyOnly, Legacy, true, false, Off, false, CompareOff, Native),
            (InvestigationRolloutMode::ShadowRegistry, Legacy, true, true, ShadowAudit, false, PromotionBlocking, Native),
            (InvestigationRolloutMode::DualReadCompare, Legacy, true, true, CompareOnly, false, WholeRecordExact, Native),
            (InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection, Registry, false, false, Canonical, true, AuditOnly, CanonicalDerivedFailClosed),
            (InvestigationRolloutMode::NewOnly, Registry, false, false, Canonical, true, CompareOff, HistoricalReadOnly),
        ];
        for (mode, authority, legacy_mutation, shadow, campaign, jit, compare, projection) in expected {
            let policy = mode.policy();
            assert_eq!(policy.canonical_writer, authority);
            assert_eq!(policy.gate_authority, authority);
            assert_eq!(policy.allow_legacy_mutation, legacy_mutation);
            assert_eq!(policy.write_registry_shadow, shadow);
            assert_eq!(policy.campaign_write_policy, campaign);
            assert_eq!(policy.allow_prepared_action_jit, jit);
            assert_eq!(policy.compare_policy, compare);
            assert_eq!(policy.legacy_projection, projection);
        }
    }

    #[test]
    fn legal_contract_mode_pairs_are_closed() {
        assert!(InvestigationContractVersion::LegacyCandidateV1
            .allows(InvestigationRolloutMode::LegacyOnly));
        for mode in [
            InvestigationRolloutMode::ShadowRegistry,
            InvestigationRolloutMode::DualReadCompare,
            InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection,
            InvestigationRolloutMode::NewOnly,
        ] {
            assert!(InvestigationContractVersion::HypothesisRegistryV1.allows(mode));
            assert!(!InvestigationContractVersion::LegacyCandidateV1.allows(mode));
        }
        assert!(!InvestigationContractVersion::HypothesisRegistryV1
            .allows(InvestigationRolloutMode::LegacyOnly));
    }

    #[test]
    fn investigation_error_codes_are_stable_and_closed() {
        assert_eq!(
            InvestigationErrorCode::ALL.map(InvestigationErrorCode::as_str),
            [
                "INVESTIGATION_FORBIDDEN",
                "INVESTIGATION_INVALID_ID",
                "INVESTIGATION_INVALID_ARGUMENT",
                "INVESTIGATION_CURSOR_INVALID",
                "INVESTIGATION_PROJECTION_STALE",
                "INVESTIGATION_AUTHORITY_CORRUPT",
                "INVESTIGATION_DATABASE",
                "INVESTIGATION_LEGACY_PROJECTION_DIVERGED",
            ],
        );
    }
}
```

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_rollout_matrix) | test(legal_contract_mode_pairs)')
```

Expected：编译失败，错误明确指出 `InvestigationContractVersion` / `InvestigationRolloutMode` 未定义；不能因为 filter 没匹配到测试而显示绿。

### Step 3：实现最小 contract

实现以下 closed enums 与 policy，不读取环境变量或数据库：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationContractVersion {
    #[default]
    LegacyCandidateV1,
    HypothesisRegistryV1,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationRolloutMode {
    #[default]
    LegacyOnly,
    ShadowRegistry,
    DualReadCompare,
    RegistryAuthoritativeLegacyProjection,
    NewOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationAuthority { Legacy, Registry }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparePolicy { Off, PromotionBlocking, WholeRecordExact, AuditOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignWritePolicy { Off, ShadowAudit, CompareOnly, Canonical }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyProjectionPolicy {
    Native,
    CanonicalDerivedFailClosed,
    HistoricalReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationErrorCode {
    Forbidden,
    InvalidId,
    InvalidArgument,
    CursorInvalid,
    ProjectionStale,
    AuthorityCorrupt,
    Database,
    LegacyProjectionDiverged,
}

impl InvestigationErrorCode {
    pub const ALL: [Self; 8] = [
        Self::Forbidden,
        Self::InvalidId,
        Self::InvalidArgument,
        Self::CursorInvalid,
        Self::ProjectionStale,
        Self::AuthorityCorrupt,
        Self::Database,
        Self::LegacyProjectionDiverged,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forbidden => "INVESTIGATION_FORBIDDEN",
            Self::InvalidId => "INVESTIGATION_INVALID_ID",
            Self::InvalidArgument => "INVESTIGATION_INVALID_ARGUMENT",
            Self::CursorInvalid => "INVESTIGATION_CURSOR_INVALID",
            Self::ProjectionStale => "INVESTIGATION_PROJECTION_STALE",
            Self::AuthorityCorrupt => "INVESTIGATION_AUTHORITY_CORRUPT",
            Self::Database => "INVESTIGATION_DATABASE",
            Self::LegacyProjectionDiverged => "INVESTIGATION_LEGACY_PROJECTION_DIVERGED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvestigationModePolicy {
    pub canonical_writer: InvestigationAuthority,
    pub gate_authority: InvestigationAuthority,
    pub allow_legacy_mutation: bool,
    pub write_registry_shadow: bool,
    pub campaign_write_policy: CampaignWritePolicy,
    pub allow_prepared_action_jit: bool,
    pub compare_policy: ComparePolicy,
    pub legacy_projection: LegacyProjectionPolicy,
}

impl InvestigationContractVersion {
    pub const ALL: [Self; 2] = [Self::LegacyCandidateV1, Self::HypothesisRegistryV1];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyCandidateV1 => "legacy_candidate_v1",
            Self::HypothesisRegistryV1 => "hypothesis_registry_v1",
        }
    }

    pub const fn allows(self, mode: InvestigationRolloutMode) -> bool {
        matches!(
            (self, mode),
            (Self::LegacyCandidateV1, InvestigationRolloutMode::LegacyOnly)
                | (Self::HypothesisRegistryV1, InvestigationRolloutMode::ShadowRegistry)
                | (Self::HypothesisRegistryV1, InvestigationRolloutMode::DualReadCompare)
                | (
                    Self::HypothesisRegistryV1,
                    InvestigationRolloutMode::RegistryAuthoritativeLegacyProjection
                )
                | (Self::HypothesisRegistryV1, InvestigationRolloutMode::NewOnly)
        )
    }
}

impl InvestigationRolloutMode {
    pub const ALL: [Self; 5] = [
        Self::LegacyOnly,
        Self::ShadowRegistry,
        Self::DualReadCompare,
        Self::RegistryAuthoritativeLegacyProjection,
        Self::NewOnly,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyOnly => "legacy_only",
            Self::ShadowRegistry => "shadow_registry",
            Self::DualReadCompare => "dual_read_compare",
            Self::RegistryAuthoritativeLegacyProjection => {
                "registry_authoritative_legacy_projection"
            }
            Self::NewOnly => "new_only",
        }
    }

    pub const fn mode_rank(self) -> i16 {
        match self {
            Self::LegacyOnly => 0,
            Self::ShadowRegistry => 1,
            Self::DualReadCompare => 2,
            Self::RegistryAuthoritativeLegacyProjection => 3,
            Self::NewOnly => 4,
        }
    }

    pub const fn policy(self) -> InvestigationModePolicy {
        use ComparePolicy::{AuditOnly, Off, PromotionBlocking, WholeRecordExact};
        use CampaignWritePolicy::{Canonical, CompareOnly, Off as CampaignOff, ShadowAudit};
        use InvestigationAuthority::{Legacy, Registry};
        use LegacyProjectionPolicy::{CanonicalDerivedFailClosed, HistoricalReadOnly, Native};

        match self {
            Self::LegacyOnly => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: false,
                campaign_write_policy: CampaignOff,
                allow_prepared_action_jit: false,
                compare_policy: Off,
                legacy_projection: Native,
            },
            Self::ShadowRegistry => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: true,
                campaign_write_policy: ShadowAudit,
                allow_prepared_action_jit: false,
                compare_policy: PromotionBlocking,
                legacy_projection: Native,
            },
            Self::DualReadCompare => InvestigationModePolicy {
                canonical_writer: Legacy,
                gate_authority: Legacy,
                allow_legacy_mutation: true,
                write_registry_shadow: true,
                campaign_write_policy: CompareOnly,
                allow_prepared_action_jit: false,
                compare_policy: WholeRecordExact,
                legacy_projection: Native,
            },
            Self::RegistryAuthoritativeLegacyProjection => InvestigationModePolicy {
                canonical_writer: Registry,
                gate_authority: Registry,
                allow_legacy_mutation: false,
                write_registry_shadow: false,
                campaign_write_policy: Canonical,
                allow_prepared_action_jit: true,
                compare_policy: AuditOnly,
                legacy_projection: CanonicalDerivedFailClosed,
            },
            Self::NewOnly => InvestigationModePolicy {
                canonical_writer: Registry,
                gate_authority: Registry,
                allow_legacy_mutation: false,
                write_registry_shadow: false,
                campaign_write_policy: Canonical,
                allow_prepared_action_jit: true,
                compare_policy: Off,
                legacy_projection: HistoricalReadOnly,
            },
        }
    }
}
```

同时实现严格 `TryFrom<&str>`；未知值返回 typed parse error，禁止 fallback。由 `lib.rs` 显式 `pub use`。这是 B/C/D 唯一 rollout policy；Plan B 虽然冻结后两态的最终 policy，但在 Plan C repository/component 不存在时只写 `plan_c_verification_unavailable`，不得另造“Plan B policy”把同一 mode 解释成另一组布尔值。

### Step 4：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_) | test(legal_contract_mode_pairs)')
```

Expected：相关 tests 全部 `PASS`，unknown value test 证明不会解析成 legacy。

### Future Commit

```bash
git add backend/crates/golish-core/src/investigation_contract.rs backend/crates/golish-core/src/lib.rs
git commit -m "feat(investigation): define frozen rollout contract"
```

---

## Task 2：在唯一 migration 中建立完整 Plan B schema

> **2026-07-30 implementation correction (repository facts):** The referenced
> `application_model_operation_contract.rs` fixture does not exist, so the RED/GREEN
> suite uses the existing `capability_execution_receipts.rs` embedded-Postgres fixture
> and the `runtime_memory_rollout_attestation.rs` migration-subset upgrade pattern.
> Plan A rejects a missing or cross-organization required root before invoking the
> checked-bundle callback; Plan B therefore leaves no snapshot/attempt in that case.
> A complete four-root census that is semantic/temporal non-fresh may create only a
> `blocked_authority_bundle` snapshot inside the callback transaction. Plan B adds
> compound candidate keys to Plan A tables in `00006` and references them, but does
> not modify `00005` or reimplement freshness. The bundle header freezes the policy
> **set** hash; per-root/per-receipt policy identities remain in census members because
> Plan A intentionally permits distinct policies across roots.

**PAUSE A：没有本轮明确 schema/migration 授权时，不执行本 Task。**

**文件：**

- 创建：`backend/crates/golish-db/migrations/20260729000006_hypothesis_registry.sql`
- 创建：`backend/crates/golish-db/tests/hypothesis_registry.rs`

### Step 1：先写 migration RED

沿用 `application_model_operation_contract.rs` 的 embedded Postgres fixture，先断言：

```rust
#[tokio::test]
#[serial_test::serial]
async fn migrated_database_defaults_old_operations_to_legacy_and_has_registry_constraints() {
    let database = HypothesisRegistryDb::start("schema-default").await;
    let defaults: (String, String) = sqlx::query_as(
        "SELECT contract_version, rollout_mode FROM investigation_rollout WHERE singleton=TRUE",
    )
    .fetch_one(database.db.pool())
    .await
    .expect("read investigation rollout singleton");
    assert_eq!(defaults.0, "legacy_candidate_v1");
    assert_eq!(defaults.1, "legacy_only");

    let operation_id = database.insert_historical_operation().await;
    let frozen: (String, String) = sqlx::query_as(
        "SELECT investigation_contract_version, investigation_rollout_mode FROM operation_state WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_one(database.db.pool())
    .await
    .expect("read migrated historical operation");
    assert_eq!(frozen.0, "legacy_candidate_v1");
    assert_eq!(frozen.1, "legacy_only");
}
```

再添加独立 tests：冻结字段不可更新、DB direct INSERT 七态之外任意 Tool Truth+Investigation pair失败、同 root 只能一个 current revision、同 operation/org/current key 只能一个 root、append-only table 拒绝 update/delete、`server_phase_transition` 在非 Candidate 或 census 未 sealed 时拒绝；每revision verification plan/objective/contract exact set漏重或hash漂移拒绝；Plan B阶段伪造Campaign terminal、revision adjudication或transition receipt都不能写`verified/refuted`。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry)
```

Expected：fresh DB 启动后 test 因 `investigation_rollout` / Registry tables 不存在而失败；不是 compile-only 绿色。

### Step 3：添加 rollout、operation freeze 与 Stage Team 扩展 SQL

migration 起始部分使用精确值：

```sql
CREATE TABLE investigation_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    contract_version TEXT NOT NULL CHECK (
        contract_version IN ('legacy_candidate_v1', 'hypothesis_registry_v1')
    ),
    rollout_mode TEXT NOT NULL CHECK (
        rollout_mode IN (
            'legacy_only',
            'shadow_registry',
            'dual_read_compare',
            'registry_authoritative_legacy_projection',
            'new_only'
        )
    ),
    mode_rank SMALLINT NOT NULL CHECK (mode_rank BETWEEN 0 AND 4),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        (contract_version='legacy_candidate_v1' AND rollout_mode='legacy_only' AND mode_rank=0)
        OR
        (contract_version='hypothesis_registry_v1' AND rollout_mode='shadow_registry' AND mode_rank=1)
        OR
        (contract_version='hypothesis_registry_v1' AND rollout_mode='dual_read_compare' AND mode_rank=2)
        OR
        (contract_version='hypothesis_registry_v1' AND rollout_mode='registry_authoritative_legacy_projection' AND mode_rank=3)
        OR
        (contract_version='hypothesis_registry_v1' AND rollout_mode='new_only' AND mode_rank=4)
    )
);

INSERT INTO investigation_rollout(singleton,contract_version,rollout_mode,mode_rank)
VALUES(TRUE,'legacy_candidate_v1','legacy_only',0);

CREATE FUNCTION operation_joint_contract_rank(
    tool_truth TEXT,
    investigation_contract TEXT,
    investigation_mode TEXT
) RETURNS SMALLINT
LANGUAGE SQL IMMUTABLE STRICT AS $$
    SELECT CASE
        WHEN tool_truth='legacy_v1' AND investigation_contract='legacy_candidate_v1' AND investigation_mode='legacy_only' THEN 0
        WHEN tool_truth='shadow_v1' AND investigation_contract='legacy_candidate_v1' AND investigation_mode='legacy_only' THEN 1
        WHEN tool_truth='shadow_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='shadow_registry' THEN 2
        WHEN tool_truth='shadow_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='dual_read_compare' THEN 3
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='dual_read_compare' THEN 4
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='registry_authoritative_legacy_projection' THEN 5
        WHEN tool_truth='receipt_v1' AND investigation_contract='hypothesis_registry_v1' AND investigation_mode='new_only' THEN 6
        ELSE NULL
    END
$$;

ALTER TABLE operation_state
    ADD COLUMN investigation_contract_version TEXT NOT NULL DEFAULT 'legacy_candidate_v1',
    ADD COLUMN investigation_rollout_mode TEXT NOT NULL DEFAULT 'legacy_only',
    ADD CONSTRAINT operation_state_joint_contract_pair_check CHECK (
        operation_joint_contract_rank(
            tool_truth_contract,
            investigation_contract_version,
            investigation_rollout_mode
        ) IS NOT NULL
    );

CREATE FUNCTION enforce_operation_investigation_contract_immutable()
RETURNS trigger AS $$
BEGIN
    IF ROW(NEW.investigation_contract_version,NEW.investigation_rollout_mode)
       IS DISTINCT FROM
       ROW(OLD.investigation_contract_version,OLD.investigation_rollout_mode)
    THEN
        RAISE EXCEPTION 'OPERATION_INVESTIGATION_CONTRACT_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_state_investigation_contract_immutable
BEFORE UPDATE OF investigation_contract_version,investigation_rollout_mode ON operation_state
FOR EACH ROW EXECUTE FUNCTION enforce_operation_investigation_contract_immutable();
```

`investigation_rollout.mode_rank`只表示Investigation自身五态的0–4顺序；它不等于、也不得代替由三个冻结字段派生的0–6 `joint_contract_rank`。operation admission、fork和Plan D promotion一律调用`operation_joint_contract_rank(...)`，不能从`mode_rank`推断联合合法性。

新增统一`operation_contract_adoptions`，绑定source/target operation、source/target Tool Truth contract、Investigation contract+mode、source/target joint rank、source final-seal hash、adoption-set hash、request id、receipt hash、创建时间。DB CHECK必须调用同一个`operation_joint_contract_rank`验证两侧并要求`target_joint_rank = source_joint_rank + 1`；receipt安装append-only trigger。无receipt的fork完整继承source pair；有receipt也只能前进一个joint rank。合法相邻边可以只改变其中一轴，但request/receipt始终记录完整两轴target，且不能通过独立setter制造非法组合。migration test直接SQL插入非法pair、跳级、UPDATE/DELETE receipt并断言失败。

`operation_rollout.rs`把以上七个合法pair映射成rank 0–6，并永久拥有共享validation/creation/fork-admission函数。operation creation在同一transaction、固定锁序中读取`tool_truth_rollout`和`investigation_rollout`，先验证pair再一次INSERT；并发promotion只能看到完整旧pair或完整新pair。Plan B不提供default setter；Plan D新建唯一`operation_default_rollout.rs` mutation coordinator，复用这里的rank/validation，不把promotion写进`operation_rollout.rs`或另造第二套mapping。

替换现有自动命名 check：

```sql
ALTER TABLE stage_work_items
    DROP CONSTRAINT stage_work_items_created_by_check,
    ADD CONSTRAINT stage_work_items_created_by_check CHECK (
        created_by IN (
            'server_seed','accepted_worker_request','gate_repair','server_phase_transition'
        )
    );

ALTER TABLE stage_worker_outputs
    DROP CONSTRAINT stage_worker_outputs_business_disposition_check,
    ADD CONSTRAINT stage_worker_outputs_business_disposition_check CHECK (
        business_disposition IN ('found','checked_empty','blocked','artifact_recorded')
    );
```

`artifact_recorded` 只是 control receipt，必须有 dedicated `candidate_analysis_artifacts` 引用；不能携带 checked-empty、安全结论或 evidence 冒充分析事实。

### Step 4：建立 canonical Registry tables

创建以下 exact tables：

```sql
CREATE TABLE attack_hypotheses (
    root_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
    root_kind TEXT NOT NULL CHECK (root_kind IN ('initial','split','merge','derive')),
    identity_ingredients JSONB NOT NULL CHECK (jsonb_typeof(identity_ingredients)='object'),
    identity_ingredients_hash TEXT NOT NULL CHECK (identity_ingredients_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(root_id,operation_id,organization_id),
    UNIQUE(operation_id,organization_id,identity_ingredients_hash)
);

CREATE TABLE attack_hypothesis_revisions (
    revision_id UUID PRIMARY KEY,
    root_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    predecessor_revision_id UUID,
    revision_ordinal INTEGER NOT NULL CHECK (revision_ordinal >= 0),
    semantic_key JSONB NOT NULL CHECK (jsonb_typeof(semantic_key)='object'),
    semantic_key_hash TEXT NOT NULL CHECK (semantic_key_hash ~ '^sha256:[0-9a-f]{64}$'),
    subject_kind TEXT NOT NULL CHECK (btrim(subject_kind)<>''),
    subject_identity_hash TEXT NOT NULL CHECK (subject_identity_hash ~ '^sha256:[0-9a-f]{64}$'),
    target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    target_type_at_time TEXT NOT NULL CHECK (btrim(target_type_at_time)<>''),
    target_value_at_time TEXT NOT NULL CHECK (btrim(target_value_at_time)<>''),
    predicate_schema TEXT NOT NULL CHECK (btrim(predicate_schema)<>''),
    predicate_version INTEGER NOT NULL CHECK (predicate_version > 0),
    normalized_arguments JSONB NOT NULL CHECK (jsonb_typeof(normalized_arguments)='object'),
    trust_boundary TEXT NOT NULL CHECK (btrim(trust_boundary)<>''),
    polarity TEXT NOT NULL CHECK (polarity IN ('positive','negative')),
    epistemic_state TEXT NOT NULL CHECK (epistemic_state IN (
        'proposed','supported','contested','verified','refuted','inconclusive','invalid'
    )),
    lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN ('current','superseded','closed')),
    planning_readiness TEXT NOT NULL CHECK (planning_readiness IN (
        'ready_for_strategy','needs_enrichment','deferred','out_of_scope','unsafe'
    )),
    structured_claim JSONB NOT NULL CHECK (jsonb_typeof(structured_claim)='object'),
    assumptions JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(assumptions)='array'),
    missing_facts JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(missing_facts)='array'),
    priority INTEGER NOT NULL,
    risk_impact JSONB NOT NULL CHECK (jsonb_typeof(risk_impact)='object'),
    revision_hash TEXT NOT NULL CHECK (revision_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY(root_id,operation_id,organization_id)
        REFERENCES attack_hypotheses(root_id,operation_id,organization_id) ON DELETE RESTRICT,
    FOREIGN KEY(predecessor_revision_id)
        REFERENCES attack_hypothesis_revisions(revision_id) ON DELETE RESTRICT,
    UNIQUE(root_id,revision_ordinal),
    CHECK (
        (epistemic_state IN ('verified','refuted','invalid') AND lifecycle_state='closed')
        OR epistemic_state NOT IN ('verified','refuted','invalid')
    ),
    CHECK (lifecycle_state<>'superseded' OR planning_readiness<>'ready_for_strategy')
);

CREATE UNIQUE INDEX attack_hypothesis_one_current_revision
ON attack_hypothesis_revisions(root_id)
WHERE lifecycle_state='current';

CREATE UNIQUE INDEX attack_hypothesis_one_current_semantic_key
ON attack_hypothesis_revisions(operation_id,organization_id,semantic_key_hash)
WHERE lifecycle_state='current';
```

同一 migration 继续创建：

- `attack_hypothesis_revision_sources`：`source_role IN ('support','contradiction','application_context','knowledge_signal','gap')`；AU ref、knowledge-feed signal与evidence proof ref不共用role。`knowledge_signal`可以触发/排序hypothesis，但永远不能满足VerificationContract predicate/control、Finding/refutation lineage或terminal proof。
- `attack_hypothesis_verification_objectives`：只保存 Controller 提交、server校验后的 objective intent、stopping criteria 与 objective hash；不接受模型提交 contract id/hash、predicate component 或 required control。
- `attack_hypothesis_claim_components`：host从revision的typed predicate/structured claim确定性派生`HypothesisClaimComponentV1` exact set，闭集kind为`claim_clause|impact_qualifier|trust_boundary_condition|identity_condition`，保存canonical fragment/condition hash、required flag、ordinal/member hash与derivation contract version/digest；模型/Controller不能删组件、降required或自报set hash。
- `attack_hypothesis_verification_contracts`：host compiler为 revision+objective 生成 immutable `verification_contract.v1`，保存 contract schema/version、closed combinator、predicate/control member count+set hash、compiler/rule/policy snapshot digest、contract hash；同一 `(revision,objective,contract_version,policy_snapshot)` 唯一。
- `attack_hypothesis_verification_objective_claim_components`：每objective/contract绑定其覆盖的claim-component subset exact set与member hashes；空subset、orphan/stale component或caller-provided binding拒绝。
- `attack_hypothesis_verification_predicate_components`：contract 内 canonical-sorted predicate component exact set，保存 component semantic key、predicate schema/version、normalized argument hash、expected polarity、prerequisite hash；ordinal与member hash唯一，不能由模型或 command request指定。
- `attack_hypothesis_verification_required_controls`：contract 内 canonical-sorted required-control exact set，保存 control id/version、control contract hash、ordinal/member hash；空集合必须由host compiler显式标记 `no_required_control`，不能用缺行表达“没有控制”。
- `attack_hypothesis_verification_pair_bindings`：仅`paired_differential` contract使用；每个pair exact绑定baseline component、variant component、required control id/version/contract hash/member hash与versioned comparator rule/digest，control必须exact-one引用当前contract required-control member，component不得跨pair或缺role。
- `attack_hypothesis_verification_ordered_steps`：仅`ordered_sequence` contract使用；保存连续step ordinal、component id、同session binding key schema/version、前驱step、interleaving/reset policy与step hash，禁止跳号、重复、跨session拼接。
- `attack_hypothesis_verification_plans`：Plan B host compiler按revision冻结exact-one immutable `HypothesisVerificationPlanV1`，保存plan schema/version、revision ingredients hash、required claim-component count/set hash、objective count/set hash、ordered proof-path count/set hash、outer aggregation policy version/digest与plan hash；模型、Controller、Campaign或command不能提交plan id/hash/count/set hash。
- `attack_hypothesis_verification_plan_objectives`：plan内canonical-sorted objective exact set，每member exact绑定当前revision的objective id/hash、唯一`VerificationContractV1` id/version/hash、claim-component subset count/set hash、stopping criteria hash、outcome requirement与member hash；漏/重/orphan/stale contract均拒绝。
- `attack_hypothesis_verification_plan_paths`：plan内server-sealed ordered proof paths，保存连续path ordinal、path key、member count/set hash与path hash；至少一条path，顺序/identity/hash均由host compiler确定。
- `attack_hypothesis_verification_plan_path_members`：每path的连续member ordinal exact set，exact引用一个plan objective/contract member及其claim-component subset hash，并冻结`required_proof|required_proof_and_path_falsifier` closed role；falsifier role必须另外exact绑定其可falsify的一个或多个claim-component member hashes。每个objective至少被一个path引用，每path至少一个显式`path_falsifier`，且**每条path全部member的component union都必须exact-equal revision全部required claim components**；禁止prose或Campaign运行时改变路径/分母。outer aggregation唯一语义为：任一全覆盖path全部member及其components得到valid `satisfied` outcome才可`verified`；只有每一path都至少一个显式falsifier在其绑定component上得到valid `refuted` outcome才可`refuted`；其余一律nonterminal。若Controller objectives不能覆盖required component exact set，plan不得seal；host可以显式创建只包含covered components的narrow successor claim/revision并保留原claim非终态，但绝不能用较窄plan验证/反驳较宽原claim。Plan C只FK消费B-owned claim/plan/path/member exact sets并为每个plan objective/component给出typed outcome，不能创建第二套plan表，也不能用“只有一个Campaign结束了”替代整plan。
- `attack_hypothesis_relations`：`support|contradict|refine|split|merge|derive|duplicate|supersede` 与 source/target revision/root。
- `hypothesis_server_validation_receipts`：append-only deterministic validator authority，绑定operation/org/root、validated revision ingredients hash、validator contract version/digest、typed invalid reason与receipt hash；不接受模型/command提供receipt id/hash，且只可支撑`invalid`。
- `attack_hypothesis_state_events`：event kind、origin authority、predecessor/successor revision、successor epistemic state、authority receipt kind/id/hash、event hash、server decision id。`candidate_analysis` origin只能写 `proposed|supported|contested|inconclusive`；`invalid`只允许`server_validator`与typed validation receipt；`verified/refuted`只允许未来 Plan C 的`hypothesis_revision_adjudication` seam。
- `hypothesis_generations`：operation/org/generation ordinal/snapshot id/hash/previous generation。
- `hypothesis_generation_members`：generation + revision exact membership。
- `hypothesis_generation_transitions`：每个 previous member exact-one `unchanged|terminal|successor` disposition。
- `hypothesis_generation_transition_successors`：split/merge 的零到多 successor edges。
- `hypothesis_generation_seals`：member set hash、event set hash、open obligation hash、Controller worker、sealed time。
- `verification_capability_assessments`明确不由Plan B/`00006`创建：它由Plan C `00007`与`verification_capability_assessments.rs`的Action Compiler/policy authority拥有。Plan B只冻结objective/VerificationContract以及projection catalog中的future entity kind；comparison在Plan C前使用typed `not_available_plan_c`，不能建占位表抢写authority。
- `hypothesis_residual_risks`：revision、reason code、owner、affected inputs、next action；Plan B 新权威 mode 使用 `plan_c_verification_unavailable`。

revision ordinal 连续性、predecessor 同 root、root identity collision 和 generation transition exact-set 由 repo 在同事务锁下重算；DB unique/FK 是最后防线，不能用 trigger 中的非锁定查询代替 reducer。

`00006`还必须在revision与state-event两表上安装`DEFERRABLE INITIALLY DEFERRED` constraint trigger：commit时每个新revision必须有exact-one creating event，event successor/root/operation/org/state必须与revision一致，并按origin闭集验证acyclic authority decision。`candidate_analysis`映射四个非终态；`server_validator`只映射`invalid`；`hypothesis_revision_adjudication`在Plan B阶段固定为`PLAN_C_REVISION_ADJUDICATION_AUTHORITY_NOT_INSTALLED`。Plan C `00007`只能用compound FK/trigger扩展该origin：server-loaded adjudication必须exact绑定同revision持久化的B plan、全部typed objective/component outcome refs/count/set hashes、unresolved exact set、live AllFresh binding、transition decision id/hash及最终transition receipt id/hash；creating event authority引用pre-event transition decision，final receipt再绑定已生成的revision/event，禁止hash反向依赖。每个objective/component outcome再按contract绑定适用的Campaign terminal、完整oracle census与Finding/refutation lineage。单个Campaign terminal receipt即使对应唯一objective也不能直接授权revision终态。Plan B `00006`不得提前创建Plan C-owned adjudication/transition receipt tables，只冻结core DTO、plan tables、origin词汇与拒绝路径。普通INSERT、legacy adapter、model/controller字符串或通用repo都不能仅靠把origin字段写成某个值绕过。direct-SQL tests分别伪造`verified/refuted/invalid` revision、只伪造Campaign terminal、伪造adjudication但漏plan/outcome/unresolved/AllFresh/decision/receipt、漏creating event、重复event、state mismatch或制造revision-event-receipt hash环，必须在commit失败且不留revision/event。

### Step 5：建立 snapshot、两波 artifact 与 census tables

创建：

- `candidate_analysis_snapshots`：operation/org/wave/scope、genesis flag、previous generation seal、source-set hash、FactDelta watermark、capability/policy/credential revisions与`sealed_ready|blocked_authority_bundle`状态；并持久化Plan A multi-root bundle seal id、server-derived relevant-root count/set hash、bundle member/receipt count/set hash、denominator-graph/semantic/freshness/temporal bundle hashes、stable consumer request id，以及`EvidenceTemporalValidityPolicyV1` id/hash、target-state epoch set hash、observation-window hash、temporal decision count/set hash、managed-feed catalog/policy seal、required feed-source/member exact-set、signature-algorithm/trust-store/key-revocation epoch hashes、knowledge-feed snapshot/product-version/feed-match census hashes、stale residual/revalidation/enrichment obligation set hashes和组合`candidate_snapshot_authority_hash`。snapshot row只能在同request `with_checked_tool_truth_authority_bundle` guard callback、同一DB transaction内从opaque `CheckedToolTruthAuthorityBundle<'guard>`及host-managed signed feed authority复制这些字段并写入，不能接收caller root/feed list/token/hash/time/epoch set或在独立`REPEATABLE READ`中仅查询旧`consistent` rows。bundle或expected feed denominator不是all-fresh/current时仍原子落blocked snapshot+census+residual，但不得创建analysis attempt或Gate pass。
- `candidate_analysis_snapshot_authority_bundle_members`：与Plan A bundle root-member exact set逐ordinal相等；每行冻结stage/source family、root denominator id、organization id、root authority-set seal id/hash、denominator graph hash、semantic authority-set hash、freshness attestation set hash、temporal decision set hash、receipt count/set hash、Plan A exact `consistent_fresh|semantic_invalid|expired|mixed_epoch|skew_exceeded` member status与member hash。TI/EAS/Enum/Vuln等server-required roots不得由caller预过滤；漏任一stage root、cross-org root、root census drift、同root member漏重或任一root tamper均使bundle非all-fresh/fail closed。
- Plan B `00006`只以FK/compound identity消费Plan A先行提供的`tool_truth_authority_bundle_seals/members`（及per-root set/temporal seals）与二级opaque guard API，不创建镜像bundle truth或第二套freshness reducer；若Plan A migration/API尚未落地，Task 2/5必须PAUSE/blocked，不能退回单root set或旧consistent查询。
- `candidate_analysis_attempts`：每个snapshot/org的append-only immutable attempt header，保存server-issued id、从0连续ordinal、predecessor attempt、attempt input hash、host-frozen attack-class checklist version/digest、trust-boundary checklist version/digest、coverage-sampling contract version/digest与bounded retry policy。
- `candidate_analysis_attempt_state_events`：append-only `opened|superseded_missed_hypothesis|sealed|blocked`事件；每attempt exact-one `opened`、至多一个terminal event，operation/snapshot lock下计算“没有terminal的latest ordinal”为唯一active attempt。历史attempt header/event不更新/删除。
- `candidate_analysis_snapshot_source_sets`：每个 required source kind 的 exact ids/count/hash；non-genesis 的 previous generation/events/relations/open obligations 与 expected/unconsumed/consumed delta sets 都必须显式有行。
- `candidate_analysis_temporal_validity_censuses`、`candidate_analysis_temporal_validity_census_members`：每snapshot exact-one host-owned copy/verification census，必须与Checked bundle中全部roots/receipts的Plan A temporal/set decisions exact-equal，caller不能先过滤stale；member冻结root/member identity、evidence class、server receipt observed_at/valid_until、source/current target-state epoch、policy id/hash、observation-window/max-skew、Plan A `fresh|expired|mixed_epoch|skew_exceeded` temporal status、semantic reconciliation status与decision hash。B可以派生typed residual reason，但不能改名后丢失原status。negative/refutation TTL必须各自严格短于positive TTL，caller/model时间不参与。
- `candidate_analysis_stale_evidence_residuals`、`candidate_analysis_revalidation_obligations`：每个非authoritative bundle member exact-one typed stale residual和exact-one revalidation obligation，绑定snapshot/root/evidence/target-state epoch identity/reason/required capability与hash；它们进入snapshot authority hash和后续handoff，但对应fact/evidence body绝不能进入source/input/chunk set，也不能被当作gap refutation、checked-empty或negative proof。只要bundle的Plan A temporal status为任一`expired|mixed_epoch|skew_exceeded`、其独立semantic reconciliation axis为invalid/orphan、或server-required root不全，整个snapshot为`blocked_authority_bundle`，不能把其余fresh roots拼成“部分authoritative”Candidate分析。
- `candidate_analysis_knowledge_feed_denominators`、`candidate_analysis_knowledge_feed_denominator_members`：host先从operation-frozen managed-feed catalog/trust policy派生Candidate required `cve|cpe|kev|vendor_advisory|detection_rule` source/member exact set，保存catalog/policy id/version/hash、signature algorithm allowlist hash、trust-store version/hash、key-revocation epoch/hash、required source/member count/set hash和denominator seal。它独立于“本地store当前有什么”，所以整源缺失也必须以expected member `unavailable`闭合，不能由空store/部分store自证exact。
- `candidate_analysis_knowledge_feed_snapshots`：host针对上述expected denominator逐member从本地managed feed store冻结；每member保存expected-member ref、feed/source id、schema/version、published_at、host-ingested-at、content hash、signed manifest hash、signer/key id与signature-verification receipt、provenance、age-policy version/digest、computed age及`current|stale|signature_invalid|signer_revoked|unavailable` disposition。snapshot header绑定denominator seal、trust-policy/keyring/revocation epoch与member exact-set hash；Candidate transaction只读已验证feed，不联网更新。更新/撤销signer key属于独立运维流程和显式外部授权，agent/critic不得临时browse补authority；但Gate必须用DB clock和current trust store重新判定age/signature/key revocation，不能只相信freeze-time disposition。
- `candidate_analysis_product_version_censuses`：从all-fresh Application Model/tool-truth bundle按at-time subject冻结product identity/CPE候选、observed version与`known|unknown|conflicting` disposition exact set；caller/模型不能补版本。
- `candidate_analysis_feed_match_censuses`、`candidate_analysis_feed_match_census_members`：host以versioned CPE/range/rule matcher对`product-version × feed snapshot`运行deterministic exact reducer，保存matcher contract version/digest、input product/feed count/set hash、每product-feed `matched|no_match|unknown_product_version|feed_stale|feed_invalid` closure、匹配CVE/KEV/advisory/rule id/version/range/entry hash及最终member count/set hash。只有current+signature-valid feed与known product version的match body可作为`knowledge_signal` Candidate input；它只能产生signal/hypothesis，不能成为proof/refutation。stale/invalid feed、unknown/conflicting product version或matcher不支持必须写typed residual + feed-refresh/product-version-enrichment obligation，并使相应checklist member/coverage review blocked，而不是宣称无漏洞。
- `candidate_analysis_snapshot_inputs`：stable input key、typed source ref/hash、完整source content hash/byte count、at-time subject和server chunking disposition；`instruction_authority BOOLEAN NOT NULL DEFAULT FALSE CHECK (NOT instruction_authority)`。本表不保存“截断后的完整输入”假象。
- `candidate_analysis_input_chunk_censuses`：每个snapshot input exact-one，保存host-owned `chunking_contract_version`与`redaction_contract_version`、source content hash/size、`complete|source_empty|blocked_oversize|blocked_unrepresentable` disposition、chunk count/member-set hash和sealed time。
- `candidate_analysis_input_chunk_census_members`：每个可分析 input 的 canonical chunk exact set，保存ordinal、source byte/record range、immutable server-redacted typed envelope body或content-addressed blob id、body/blob hash、chunking/redaction version与chunk hash；`source_empty`以显式零成员census表示。只存range+hash不合格，因为source后续可变/删除时无法重放；模型不能提交chunk boundary、body/blob id、count/hash或`read_complete`。
- `candidate_analysis_page_receipts`：绑定analysis attempt、server cursor、first/last key、returned count、page hash、consumer worker；child 不可自行提交。
- `candidate_analysis_work_items`：1:1 `stage_work_items`，绑定analysis attempt，phase 仅 `proposal|critic|controller`，保存 capability、microbatch/component/page authority。
- `candidate_analysis_artifacts`：绑定analysis attempt、typed schema、worker、artifact kind/hash/body；append-only。artifact kind闭集包含`hypothesis_proposal.v1|proposal_conflict_review.v1|hypothesis_coverage_subreview.v1|hypothesis_coverage_synthesis.v1|hypothesis_coverage_review.v1|controller_decision.v1`；subreview/synthesis只能经对应dedicated writer落库，最终coverage review只能由host reducer写入，不能接收模型自报body/hash。
- `hypothesis_proposals`、`hypothesis_proposal_refs`：每条proposal绑定analysis attempt，不能跨attempt复用artifact identity。
- `candidate_analysis_proposal_censuses`、`candidate_analysis_proposal_census_members`：每analysis attempt的H1 exact set。
- `candidate_analysis_input_proposal_dispositions`：每`(analysis_attempt_id,snapshot_input_id)` exact-one `has_proposal|zero_proposal|blocked`，由server从同attempt H1 proposal refs与chunk closure派生，模型不能自报“没有 hypothesis”。该disposition只描述H1输出数量，不能代替coverage review。
- `candidate_analysis_conflict_components`、`candidate_analysis_conflict_component_members`：绑定analysis attempt；该attempt每个 proposal 恰属一个 component，singleton 也必须有 component。
- `candidate_analysis_hypothesis_coverage_checklist_members`：host按input kind、at-time subject、all-fresh Application Model与trust boundaries，从versioned base registries + current signed feed-match census冻结attack-class × trust-boundary exact set；保存两份checklist contract version/digest、feed snapshot/matcher/member refs exact hash、ordinal、attack class id/version、boundary identity/hash、applicability basis与member hash。Agent不能增删、抽样或自报member-set hash；feed stale/invalid或product version unknown/conflicting时，version-dependent member必须显式blocked并关联refresh/enrichment obligation，不能从checklist静默消失。
- `candidate_analysis_hypothesis_coverage_chunk_partitions`：host把每input完整chunk census切成bounded、canonical、连续且无重叠/空洞的partition exact set，保存partition ordinal、first/last chunk ordinal、chunk count/set hash、bounded context budget、partition hash；任何单critic都不允许宣称一次读完超出context上限的大input。
- `candidate_analysis_hypothesis_coverage_subreview_censuses`、`candidate_analysis_hypothesis_coverage_subreview_census_members`：每input的member exact set必须等于`checklist-member × chunk-partition`笛卡尔积；member保存两侧ordinal/hash、server-issued work item、`required|sampling_omitted` disposition与member hash。正常full mode全部为required；deterministic sampling只能把未执行格子显式记为`sampling_omitted`并最终blocked/degraded，不能缩小或伪造census。
- `candidate_analysis_hypothesis_coverage_subreviews`：每个required `(input,checklist-member,chunk-partition)` exact-one immutable typed subreview，绑定designated chunk exact set/read receipts、该input全部H1 proposal ref exact set/hash、primary analyst与不同的map critic worker、context-budget/truncation attestation、`no_local_miss|missed_hypothesis|blocked` outcome、typed missed refs与subreview hash。page/read receipt只证明bytes交付，不能证明模型理解；provider context truncation、漏chunk、漏H1 ref、worker自证或不匹配tuple均只能blocked。
- `candidate_analysis_hypothesis_coverage_synthesis_censuses`、`candidate_analysis_hypothesis_coverage_synthesis_census_members`、`candidate_analysis_hypothesis_coverage_synthesis_reviews`：host冻结bounded recursive semantic reduction tree。叶层`cross_chunk`按input/checklist-member消费全部partition subreviews；`cross_input_partition`按attack-class×trust-boundary消费bounded input partition；若同dimension仍有多个partition，host以固定fan-in构造零到多层`cross_input_reduce(level)`直到每attack-class×boundary exact-one root；再构造零到多层`cross_dimension_reduce(level)`消费全部dimension roots及server-frozen relationship cross-index，最终每org/snapshot/attempt exact-one `global_semantic_root`，从而发现跨input partition、跨attack-class及跨trust-boundary组合。每node保存level/partition ordinal、covered input/checklist exact sets、child receipt count/set hash、relationship-index hash、descendant-worker count/set hash与node hash；partition顺序变化不能改变canonical root/outcome。任一parent synthesis worker必须不同于全部primary analysts及其**全部transitive descendant** map/synthesis workers，而不只是直接child。漏任一child/dimension/global root、context truncation、worker复用或只完成局部partition时不得finalize。
- `candidate_analysis_hypothesis_coverage_global_reviews`：每analysis attempt exact-one immutable host-reduced review，绑定recursive tree census/root receipt、全部dimension root exact set、relationship cross-index、worker-separation proof与`adequate|missed_hypothesis|blocked` outcome。只有global semantic root sealed且未发现组合遗漏时，per-input review才可能`adequate`；global miss使当前attempt整体supersede，不能只给某个input打勾。
- `candidate_analysis_critic_censuses`、`candidate_analysis_critic_census_members`：每analysis attempt的H2 exact-set reducer；member kind闭集为`proposal_conflict_component|hypothesis_coverage_subreview|hypothesis_coverage_synthesis|hypothesis_coverage_input_review|hypothesis_coverage_global_review`，必须分别与conflict components、subreview笛卡尔积、server-frozen recursive synthesis tree、全部snapshot inputs及exact-one global review exact-equal；zero-proposal只是proposal ref count为0的普通input review。
- `candidate_analysis_hypothesis_coverage_reviews`：每`(analysis_attempt_id,snapshot_input_id)` exact-one immutable host-reduced `hypothesis_coverage_review.v1`，保存attempt ordinal、完整chunk census/partition/subreview census及read receipt set hash、该input全部H1 proposal ref exact set/hash（允许显式空集）、attack-class与trust-boundary checklist version/digest/member-set hash、recursive synthesis census/global-root receipt set hash、coverage-sampling contract version/digest、全部worker separation set hash、`full|deterministic_sample` review mode、`adequate|missed_hypothesis|blocked` outcome、逐checklist-member disposition与typed missed attack-class/trust-boundary refs、review hash。只有全部required subreviews、完整recursive tree与global review exact闭合、无truncation/omission且host reducer复算一致时才能写`adequate`；即使H1已有一个proposal，也必须检查第二/第三个独立或组合hypothesis是否遗漏。`adequate`仅表示在冻结checklist与完整输入/综合census下未发现遗漏，不是漏洞不存在或全域coverage证明；deterministic sampling永远不能写`adequate`，只能`blocked`并写`candidate_hypothesis_coverage_sampling_degraded` residual。`missed_hypothesis`以append-only state event把当前attempt终结为`superseded_missed_hypothesis`并创建后继attempt；critic不得直接创建proposal，也不得覆盖旧review。重试耗尽只能写blocked residual，不能写refuted/checked-empty/analysis-coverage-complete。
- `hypothesis_proposal_relations`、`hypothesis_merge_decisions`。
- `input_processing_dispositions`：每 snapshot input 恰一行，值仅 `analyzed|informational|duplicate_input|not_security_relevant|gap|blocked`。
- `input_hypothesis_relations`：零到多 `creates_hypothesis|supports_existing|contradicts_existing|qualifies_existing`。

`server_phase_transition` trigger 规则固定为：只有`sealed_ready`且bundle root/member/receipt exact set全fresh的snapshot可由`server_seed`创建proposal work items；`blocked_authority_bundle`或stale/invalid feed导致的blocked checklist member不能被caller略过。conflict critic与coverage map work items只能在同analysis attempt的H1 census sealed 后创建；每个synthesis node只在其child exact set闭合后创建，cross-input reduction必须归并到每dimension exact-one root，再归并到exact-one org/snapshot global semantic root；controller只能在同attempt H2 exact census、global review及全部per-input host-reduced coverage reviews sealed且没有`missed_hypothesis`后创建。新attempt必须直接引用被miss supersede的前驱并重开H1，任何旧attempt artifact/subreview/synthesis/review不能满足新attempt census。任一 identity/attempt/mode/stage 不匹配均抛稳定 SQL code。

### Step 6：建立 outbox、legacy projection、compare 与 read head

创建：

```sql
CREATE TABLE investigation_projection_source_heads (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    last_source_batch_seq BIGINT NOT NULL DEFAULT 0 CHECK (last_source_batch_seq>=0),
    last_source_batch_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE investigation_projection_heads (
    operation_id UUID PRIMARY KEY REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    projection_schema_version INTEGER NOT NULL DEFAULT 1 CHECK (projection_schema_version=1),
    change_seq BIGINT NOT NULL DEFAULT 0 CHECK (change_seq>=0),
    last_projected_batch_id UUID,
    cursor_salt BYTEA NOT NULL DEFAULT gen_random_bytes(32)
        CHECK (OCTET_LENGTH(cursor_salt)=32),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE FUNCTION projection_timeline_mapping_is_valid(
    p_entity_kind TEXT,
    p_change_kind TEXT,
    p_timeline_event_kind TEXT
) RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE STRICT AS $$
    SELECT EXISTS (
        SELECT 1
        FROM (VALUES
            ('generation','insert','generation_sealed'),
            ('hypothesis','insert','hypothesis_inserted'),
            ('hypothesis','supersede','hypothesis_superseded'),
            ('hypothesis','close','hypothesis_closed'),
            ('hypothesis','invalidate','hypothesis_invalidated'),
            ('hypothesis_verification_plan','close','hypothesis_verification_plan_sealed'),
            ('hypothesis_verification_objective_outcome','close','hypothesis_verification_objective_outcome_closed'),
            ('hypothesis_verification_objective_outcome','invalidate','hypothesis_verification_objective_outcome_invalidated'),
            ('hypothesis_revision_adjudication','close','hypothesis_revision_adjudication_closed'),
            ('hypothesis_revision_adjudication','invalidate','hypothesis_revision_adjudication_invalidated'),
            ('hypothesis_revision_terminal_decision','close','hypothesis_revision_terminal_decision_closed'),
            ('hypothesis_revision_terminal_decision','invalidate','hypothesis_revision_terminal_decision_invalidated'),
            ('hypothesis_state_event','insert','hypothesis_state_event_inserted'),
            ('hypothesis_state_event','invalidate','hypothesis_state_event_invalidated'),
            ('finding','insert','finding_inserted'),
            ('finding','invalidate','finding_invalidated'),
            ('refutation','insert','refutation_inserted'),
            ('refutation','invalidate','refutation_invalidated'),
            ('relation','insert','relation_inserted'),
            ('relation','invalidate','relation_invalidated'),
            ('residual','insert','residual_inserted'),
            ('residual','close','residual_closed'),
            ('residual','invalidate','residual_invalidated'),
            ('capability_assessment','insert','capability_assessment_inserted'),
            ('capability_assessment','invalidate','capability_assessment_invalidated'),
            ('capability_assessment_set','close','capability_assessment_set_sealed'),
            ('legacy_candidate_projection','insert','legacy_candidate_projection_materialized'),
            ('legacy_candidate_projection','invalidate','legacy_candidate_projection_invalidated'),
            ('legacy_attempt_projection','insert','legacy_attempt_projection_materialized'),
            ('legacy_attempt_projection','invalidate','legacy_attempt_projection_invalidated'),
            ('shadow_comparison','compare','shadow_comparison_recorded'),
            ('campaign','insert','campaign_inserted'),
            ('campaign','supersede','campaign_superseded'),
            ('campaign','close','campaign_closed'),
            ('campaign_round','insert','campaign_round_inserted'),
            ('campaign_round','close','campaign_round_closed'),
            ('consult','insert','consult_inserted'),
            ('consult','close','consult_closed'),
            ('strategy','insert','strategy_inserted'),
            ('strategy_obligation','insert','strategy_obligation_inserted'),
            ('prepared_action','insert','prepared_action_inserted'),
            ('prepared_action','supersede','prepared_action_superseded'),
            ('authorization','insert','authorization_inserted'),
            ('action_execution','insert','action_execution_inserted'),
            ('action_execution','close','action_execution_closed'),
            ('conflict_lease','insert','conflict_lease_acquired'),
            ('conflict_lease','supersede','conflict_lease_recovery_held'),
            ('conflict_lease','close','conflict_lease_released'),
            ('budget_ledger_entry','insert','budget_ledger_entry_recorded'),
            ('cleanup_obligation','insert','cleanup_obligation_inserted'),
            ('cleanup_obligation','close','cleanup_obligation_closed'),
            ('callback_obligation','insert','callback_obligation_inserted'),
            ('callback_obligation','close','callback_obligation_closed'),
            ('oracle','insert','oracle_inserted'),
            ('oracle','invalidate','oracle_invalidated'),
            ('oracle_census','close','oracle_census_sealed'),
            ('adjudication','insert','adjudication_inserted'),
            ('campaign_terminal','close','campaign_terminal_closed'),
            ('campaign_terminal','invalidate','campaign_terminal_invalidated'),
            ('fact_delta','insert','fact_delta_inserted'),
            ('fact_delta','invalidate','fact_delta_invalidated'),
            ('fact_delta_consumption','insert','fact_delta_consumed'),
            ('fact_delta_consumption','close','fact_delta_consumption_closed'),
            ('hypothesis_evolution_proposal','insert','hypothesis_evolution_proposed'),
            ('hypothesis_evolution_decision','insert','hypothesis_evolution_decided'),
            ('consolidation','close','consolidation_closed'),
            ('fixed_point','close','fixed_point_closed'),
            ('enrichment_obligation','insert','enrichment_obligation_inserted'),
            ('enrichment_obligation','close','enrichment_obligation_closed'),
            ('application_fact_refinement_obligation','insert','application_fact_refinement_obligation_inserted'),
            ('application_fact_refinement_obligation','close','application_fact_refinement_obligation_closed'),
            ('coverage','insert','coverage_denominator_sealed'),
            ('coverage','supersede','coverage_result_recorded'),
            ('coverage','close','coverage_closed'),
            ('coverage','invalidate','coverage_invalidated'),
            ('report','insert','report_inserted'),
            ('report','close','report_closed'),
            ('report','supersede','report_superseded')
        ) AS allowed(entity_kind,change_kind,timeline_event_kind)
        WHERE allowed.entity_kind=p_entity_kind
          AND allowed.change_kind=p_change_kind
          AND allowed.timeline_event_kind=p_timeline_event_kind
    )
$$;

CREATE TABLE investigation_projection_changes (
    operation_id UUID NOT NULL,
    change_seq BIGINT NOT NULL CHECK (change_seq>0),
    event_id UUID NOT NULL UNIQUE,
    batch_id UUID NOT NULL,
    source_batch_seq BIGINT NOT NULL CHECK (source_batch_seq>0),
    entity_kind TEXT NOT NULL CHECK (entity_kind IN (
        'generation','hypothesis','hypothesis_verification_plan',
        'hypothesis_verification_objective_outcome','hypothesis_revision_adjudication',
        'hypothesis_revision_terminal_decision','hypothesis_state_event','finding','refutation',
        'relation','residual','capability_assessment',
        'capability_assessment_set',
        'legacy_candidate_projection','legacy_attempt_projection','shadow_comparison',
        'campaign','campaign_round','consult','strategy','strategy_obligation',
        'prepared_action','authorization','action_execution','conflict_lease',
        'budget_ledger_entry','cleanup_obligation','callback_obligation',
        'oracle','oracle_census','adjudication','campaign_terminal','fact_delta',
        'fact_delta_consumption','hypothesis_evolution_proposal',
        'hypothesis_evolution_decision','consolidation','fixed_point',
        'enrichment_obligation','application_fact_refinement_obligation',
        'coverage','report'
    )),
    entity_id TEXT NOT NULL CHECK (btrim(entity_id)<>''),
    entity_version BIGINT NOT NULL CHECK (entity_version>0),
    change_kind TEXT NOT NULL CHECK (
        change_kind IN ('insert','supersede','close','compare','invalidate')
    ),
    timeline_event_kind TEXT NOT NULL CHECK (timeline_event_kind IN (
        'generation_sealed',
        'hypothesis_inserted','hypothesis_superseded','hypothesis_closed','hypothesis_invalidated',
        'hypothesis_verification_plan_sealed',
        'hypothesis_verification_objective_outcome_closed',
        'hypothesis_verification_objective_outcome_invalidated',
        'hypothesis_revision_adjudication_closed','hypothesis_revision_adjudication_invalidated',
        'hypothesis_revision_terminal_decision_closed',
        'hypothesis_revision_terminal_decision_invalidated',
        'hypothesis_state_event_inserted','hypothesis_state_event_invalidated',
        'finding_inserted','finding_invalidated','refutation_inserted','refutation_invalidated',
        'relation_inserted','relation_invalidated',
        'residual_inserted','residual_closed','residual_invalidated',
        'capability_assessment_inserted','capability_assessment_invalidated',
        'capability_assessment_set_sealed',
        'legacy_candidate_projection_materialized','legacy_candidate_projection_invalidated',
        'legacy_attempt_projection_materialized','legacy_attempt_projection_invalidated',
        'shadow_comparison_recorded',
        'campaign_inserted','campaign_superseded','campaign_closed','campaign_round_inserted',
        'campaign_round_closed','consult_inserted','consult_closed','strategy_inserted',
        'strategy_obligation_inserted','prepared_action_inserted','prepared_action_superseded',
        'authorization_inserted','action_execution_inserted','action_execution_closed',
        'conflict_lease_acquired','conflict_lease_recovery_held','conflict_lease_released',
        'budget_ledger_entry_recorded',
        'cleanup_obligation_inserted','cleanup_obligation_closed',
        'callback_obligation_inserted','callback_obligation_closed',
        'oracle_inserted','oracle_invalidated','oracle_census_sealed','adjudication_inserted',
        'campaign_terminal_closed','campaign_terminal_invalidated',
        'fact_delta_inserted','fact_delta_invalidated',
        'fact_delta_consumed','fact_delta_consumption_closed',
        'hypothesis_evolution_proposed','hypothesis_evolution_decided',
        'consolidation_closed','fixed_point_closed','enrichment_obligation_inserted',
        'enrichment_obligation_closed','application_fact_refinement_obligation_inserted',
        'application_fact_refinement_obligation_closed','coverage_denominator_sealed',
        'coverage_result_recorded','coverage_closed','coverage_invalidated',
        'report_inserted','report_closed',
        'report_superseded'
    )),
    invalidation_reason TEXT CHECK (invalidation_reason IS NULL OR invalidation_reason IN (
        'source_superseded','source_quarantined','authority_stale','source_deleted',
        'legacy_projection_unsupported','legacy_projection_derivation_failed',
        'legacy_projection_diverged','contract_unsupported'
    )),
    change_hash TEXT NOT NULL CHECK (change_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_occurred_at TIMESTAMPTZ,
    source_time_status TEXT NOT NULL CHECK (
        source_time_status IN ('known','historical_unknown')
    ),
    projected_at TIMESTAMPTZ NOT NULL,
    CHECK ((source_time_status='known') = (source_occurred_at IS NOT NULL)),
    CHECK ((change_kind='invalidate') = (invalidation_reason IS NOT NULL)),
    CHECK (projection_timeline_mapping_is_valid(
        entity_kind, change_kind, timeline_event_kind
    )),
    PRIMARY KEY(operation_id,change_seq),
    FOREIGN KEY(operation_id) REFERENCES investigation_projection_heads(operation_id)
        ON DELETE RESTRICT
);

INSERT INTO investigation_projection_heads(operation_id)
SELECT operation_id FROM operation_state
ON CONFLICT (operation_id) DO NOTHING;

INSERT INTO investigation_projection_source_heads(operation_id)
SELECT operation_id FROM operation_state
ON CONFLICT (operation_id) DO NOTHING;

CREATE FUNCTION initialize_investigation_projection_head()
RETURNS trigger AS $$
BEGIN
    INSERT INTO investigation_projection_source_heads(operation_id)
    VALUES (NEW.operation_id)
    ON CONFLICT (operation_id) DO NOTHING;
    INSERT INTO investigation_projection_heads(operation_id)
    VALUES (NEW.operation_id)
    ON CONFLICT (operation_id) DO NOTHING;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operation_state_initialize_investigation_projection_head
AFTER INSERT ON operation_state
FOR EACH ROW EXECUTE FUNCTION initialize_investigation_projection_head();

CREATE FUNCTION enforce_investigation_projection_head_identity_immutable()
RETURNS trigger AS $$
BEGIN
    IF ROW(NEW.projection_schema_version, NEW.cursor_salt)
       IS DISTINCT FROM ROW(OLD.projection_schema_version, OLD.cursor_salt)
    THEN
        RAISE EXCEPTION 'INVESTIGATION_PROJECTION_HEAD_IDENTITY_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER investigation_projection_head_identity_immutable
BEFORE UPDATE OF projection_schema_version,cursor_salt
ON investigation_projection_heads
FOR EACH ROW EXECUTE FUNCTION enforce_investigation_projection_head_identity_immutable();
```

`investigation_projection_source_heads`只由canonical outbox helper在已经锁定operation的source transaction中CAS，用于分配immutable、无空洞的`source_batch_seq`并冻结`predecessor_batch_id`；它不是read-model head，projector不能修改。`last_projected_batch_id`只能与`change_seq/updated_at`由projector在同一CAS更新；普通repo没有setter。projector锁住head并为完整batch分配连续operation-local`change_seq`区间后，用operation UUIDv5 namespace和`projection-change.v1`域分隔的canonical length-prefixed tuple `(change_seq, entity_kind, entity_id, entity_version, change_kind, change_hash)`派生`event_id`；response-loss/rebuild在相同source batch order下必须得到同一UUID。Timeline cursor/DTO只使用该持久event id，Plan D不得临时生成随机id或把TEXT `entity_id`解析成UUID。

以上五个closed catalog由`golish-core::investigation_projection`唯一拥有，DB TEXT只是在migration中的精确镜像；repo/outbox/projector/Timeline不得接收裸字符串（其中`ProjectionChangeKind`仅为内部catalog，其余四个同时是Plan D public wire enums）：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../frontend/lib/generated/")]
pub enum ProjectionEntityKind {
    Generation, Hypothesis, HypothesisVerificationPlan,
    HypothesisVerificationObjectiveOutcome,
    HypothesisRevisionAdjudication, HypothesisRevisionTerminalDecision,
    HypothesisStateEvent, Finding, Refutation, Relation, Residual, CapabilityAssessment,
    CapabilityAssessmentSet,
    LegacyCandidateProjection, LegacyAttemptProjection, ShadowComparison,
    Campaign, CampaignRound, Consult, Strategy, StrategyObligation,
    PreparedAction, Authorization, ActionExecution, ConflictLease,
    BudgetLedgerEntry, CleanupObligation, CallbackObligation,
    Oracle, OracleCensus, Adjudication, CampaignTerminal, FactDelta,
    FactDeltaConsumption, HypothesisEvolutionProposal, HypothesisEvolutionDecision,
    Consolidation, FixedPoint, EnrichmentObligation,
    ApplicationFactRefinementObligation, Coverage, Report,
}

pub enum ProjectionChangeKind { Insert, Supersede, Close, Compare, Invalidate }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../frontend/lib/generated/")]
pub enum ProjectionInvalidationReason {
    SourceSuperseded,
    SourceQuarantined,
    AuthorityStale,
    SourceDeleted,
    LegacyProjectionUnsupported,
    LegacyProjectionDerivationFailed,
    LegacyProjectionDiverged,
    ContractUnsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../frontend/lib/generated/")]
pub enum TimelineEventKind {
    GenerationSealed,
    HypothesisInserted, HypothesisSuperseded, HypothesisClosed, HypothesisInvalidated,
    HypothesisVerificationPlanSealed,
    HypothesisVerificationObjectiveOutcomeClosed,
    HypothesisVerificationObjectiveOutcomeInvalidated,
    HypothesisRevisionAdjudicationClosed, HypothesisRevisionAdjudicationInvalidated,
    HypothesisRevisionTerminalDecisionClosed, HypothesisRevisionTerminalDecisionInvalidated,
    HypothesisStateEventInserted, HypothesisStateEventInvalidated,
    FindingInserted, FindingInvalidated, RefutationInserted, RefutationInvalidated,
    RelationInserted, RelationInvalidated,
    ResidualInserted, ResidualClosed, ResidualInvalidated,
    CapabilityAssessmentInserted, CapabilityAssessmentInvalidated, CapabilityAssessmentSetSealed,
    LegacyCandidateProjectionMaterialized, LegacyCandidateProjectionInvalidated,
    LegacyAttemptProjectionMaterialized, LegacyAttemptProjectionInvalidated,
    ShadowComparisonRecorded,
    CampaignInserted, CampaignSuperseded, CampaignClosed,
    CampaignRoundInserted, CampaignRoundClosed, ConsultInserted, ConsultClosed,
    StrategyInserted, StrategyObligationInserted,
    PreparedActionInserted, PreparedActionSuperseded, AuthorizationInserted,
    ActionExecutionInserted, ActionExecutionClosed,
    ConflictLeaseAcquired, ConflictLeaseRecoveryHeld, ConflictLeaseReleased,
    BudgetLedgerEntryRecorded,
    CleanupObligationInserted, CleanupObligationClosed,
    CallbackObligationInserted, CallbackObligationClosed,
    OracleInserted, OracleInvalidated, OracleCensusSealed, AdjudicationInserted,
    CampaignTerminalClosed, CampaignTerminalInvalidated,
    FactDeltaInserted, FactDeltaInvalidated, FactDeltaConsumed, FactDeltaConsumptionClosed,
    HypothesisEvolutionProposed, HypothesisEvolutionDecided,
    ConsolidationClosed, FixedPointClosed,
    EnrichmentObligationInserted, EnrichmentObligationClosed,
    ApplicationFactRefinementObligationInserted,
    ApplicationFactRefinementObligationClosed,
    CoverageDenominatorSealed, CoverageResultRecorded, CoverageClosed, CoverageInvalidated,
    ReportInserted, ReportClosed, ReportSuperseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../frontend/lib/generated/")]
pub enum ProjectionSourceTimeStatusV1 { Known, HistoricalUnknown }
```

每个enum提供`ALL/as_str/TryFrom<&str>`；上述四个会进入Plan D Timeline IPC的closed enum同时在`golish-core`唯一derive `serde + ts_rs::TS`并生成golden bindings，外部crate不得定义mirror enum或自由字符串。unknown值一律typed error，不能fallback为generic update。migration中的immutable SQL函数`projection_timeline_mapping_is_valid`与Rust matcher使用同一张人工审阅的closed mapping表；catalog test必须逐项比较Rust`ALL`与migration CHECK，并exhaustive验证允许的`(entity_kind,change_kind)->timeline_event_kind`映射及每种`Invalidate`必带closed invalidation reason。

Plan B Candidate canonical compound必须把每个sealed `HypothesisVerificationPlanV1` exact-one路由为`HypothesisVerificationPlan Close/HypothesisVerificationPlanSealed`；漏plan、重复plan或plan hash与revision outbox source不一致时canonical transaction回滚。plan是revision终态authority的冻结输入，不得投影成generic hypothesis prose或Campaign leaf。

`PlanCProjectionMutationRouteV1`也是core-owned exhaustive static catalog，而不是Plan C可扩展的字符串map。它必须逐表冻结：assessment row/set seal → `CapabilityAssessment/CapabilityAssessmentSet`；Campaign/round/consult/strategy/strategy-obligation → 同名kind；Prepared Action/authorization/execution/conflict lease/budget entry/cleanup/callback → 各专属kind；oracle assessment/census/adjudication/Campaign terminal → `Oracle/OracleCensus/Adjudication/CampaignTerminal`；每个plan objective outcome → `HypothesisVerificationObjectiveOutcome`；revision-level adjudication、terminal decision、Finding或Refutation、creating state event与successor revision分别exact路由到`HypothesisRevisionAdjudication/HypothesisRevisionTerminalDecision/Finding|Refutation/HypothesisStateEvent/Hypothesis`，不能复用Campaign terminal、oracle adjudication或折叠source members；FactDelta/consumption/evolution proposal/evolution decision → 各专属kind；consolidation/fixed-point/enrichment/application-fact refinement → 各专属kind；wave/campaign denominator、member、result、receipt与unassigned-result mutations → 同一typed `Coverage` aggregate的`Insert/Supersede/Close`版本。Plan C terminal canonical manifest因此是exact-five：adjudication、terminal decision、Finding-or-Refutation、state event、successor Hypothesis revision/version；缺Hypothesis close/insert会使materialized list/detail停在旧状态并必须回滚。authority quarantine header/member exact set必须路由为同batch中所有实际受影响的`CampaignTerminal/HypothesisVerificationObjectiveOutcome/HypothesisRevisionAdjudication/HypothesisRevisionTerminalDecision/Finding|Refutation/HypothesisStateEvent/Hypothesis/Coverage/FactDelta/Report Invalidate|Supersede`，实际集合由dependency graph exact证明，不能用静态子集遗漏Hypothesis或Report；correction bundle路由typed `FactDelta Insert` subtype，correction consumption路由`FactDeltaConsumption Insert/Close`；conflict recovery-hold路由`ConflictLease Supersede`并产生`ConflictLeaseRecoveryHeld`。单个Campaign terminal close/invalidate仍只是leaf事实，不能直接写Hypothesis终态，也不能独自关闭/推翻revision adjudication。每个Plan C canonical repo compound必须经该catalog把其实际mutation exact set映射成同一source batch member exact set；没有route、漏member、额外member、折叠terminal source members或错误parent aggregate identity时source transaction rollback。Plan C `00007`须同步采用该exact-five manifest与quarantine dependency routes，不能修改Plan B `00006` CHECK或另建catalog。

Plan B的`projection_plan_b_verification_plan_route_`测试先证明每revision plan seal exact-one路由且单个Campaign terminal不能冒充该entity。`projection_plan_c_route_catalog_`再对core-owned fixture逐项核对route exact-one，并覆盖objective outcome close/invalidate、revision terminal exact-five（adjudication/terminal-decision/Finding-or-Refutation/state-event/successor-Hypothesis）、Campaign terminal leaf、其余Plan C route及quarantine dependency graph中Hypothesis/Report等全部实际affected entities；测试特意漏掉、折叠或多加terminal五member任一项必须在source commit前失败，并断言投影后的Hypothesis list/detail显示terminal successor。同时构造unknown mutation，断言`PROJECTION_MUTATION_ROUTE_UNSUPPORTED`且三个heads/outbox均不变。Plan C实施时`projection_plan_c_repo_routes_`必须把`00007`各compound manifest与该core fixture exact-equal；B不反向依赖尚未存在的`00007`。

`ProjectionSourceSnapshotV1`与`ProjectionEntityV1`同样由该core模块定义为按`ProjectionEntityKind` exhaustive match的closed tagged unions，constructor只接收server repo typed DTO并执行redaction/bounds/canonical hash；不得以`serde_json::Value`、caller-provided type tag或live row locator充当逃生口。Plan C/D新增payload字段必须升级对应schema/version并更新B-ownedcatalog测试，不能只让DB TEXT CHECK接受新字符串。

migration test必须证明全部历史operation backfill exact-one source head+projection head；migration后的普通create/fork在同一transaction由DB trigger同时得到两者，rollback不留orphan；`projection_schema_version/cursor_salt`不可变，canonical outbox helper只能CAS推进source batch seq，合法projector transaction只能CAS推进`change_seq/last_projected_batch_id/updated_at`。禁止lazy read-time initializer，否则首次并发read会改变只读snapshot。

并创建：

- `investigation_projection_outbox_batches`：source canonical transaction写exact-one immutable batch header，保存operation/scope、operation-local `source_batch_seq`、exact predecessor batch id、request/source transaction identity、member count/set hash、server-derived `source_occurred_at`及source-time status；`UNIQUE(operation_id,source_batch_seq)`且seq必须由source head分配，caller不能自报顺序或逐event绕过batch。
- `investigation_projection_source_blobs`：可选content-addressed immutable redacted payload store，identity为schema/version/content hash，保存bounded bytes与redaction metadata；append-only且不能保存live path/loader locator。
- `investigation_projection_outbox`：batch member ordinal、typed entity/change kind、source entity version/id/hash、member-level `source_occurred_at/source_time_status`、`projection_source_snapshot.v1` schema/version/hash、immutable server-redacted typed source body或content-addressed blob id/hash、typed timeline event intent与optional invalidation reason；canonical side只能在同一事务插入完整batch，不能推进head或写materialized projection。新canonical compound的member source time均来自同一DB transaction timestamp；historical backfill逐member保存可信原时间或`historical_unknown`，不能拿batch/projector时间代替。blob store row同样append-only并由outbox FK `ON DELETE RESTRICT`保留。projector构造projection时只能读该outbox snapshot/blob，type graph中不得持有live canonical source loader；source随后修改/删除也必须重放相同bytes/hash。
- `investigation_projection_entity_versions`：append-only materialized tagged projection union，identity为`operation+entity kind+entity id+entity version`，保存source hash、projection hash、batch/change sequence、source occurred time、projected time与typed invalidation。每个version还保存`predecessor_entity_version/predecessor_projection_hash`：version 1必须显式`predecessor_absent=true`且二者为空；version N>1必须引用同operation/kind/id的N-1及exact hash，并有self compound FK/unique约束。payload只能由server `ProjectionEntityV1` tagged enum序列化并在读取时fail-closed反序列化，禁止caller arbitrary JSON。
- `investigation_projection_batch_receipts`：每batch exact-one，保存source batch seq/predecessor、first/last change seq、entity-version/change/timeline manifest hash、projected time；receipt本身就是整个outbox exact set唯一processed证明，不存在per-member marker/update，并与全部entity versions、changes、legacy compatibility versions及head CAS在同一projector transaction提交。
- `hypothesis_legacy_candidate_projection_versions`与`hypothesis_legacy_attempt_projection_versions`：append-only、read-only、canonical-derived compatibility entity versions；每行绑定source generation/revision/contract hash、projection hash、batch/change seq与typed `ready|unsupported|invalidated`状态。它们不是旧authority row，不写旧Candidate/Attempt mutation tables。

每个source transaction必须在operation/source-head锁下分配下一个source batch seq，再canonical-sort完整outbox member set并冻结batch count/hash；projector只能claim该operation“最小尚未完成且predecessor receipt已存在”的连续完整batch，不能越过较早batch或逐row标记processed。它在一个短transaction中锁operation head、重读batch exact set/hash与predecessor receipt、验证每个entity version的直接predecessor/version+1/change legality，派生所有materialized entity versions与typed timeline changes、派生适用的Candidate/Attempt compatibility versions或typed invalidation、写batch receipt，最后一次CAS把head从旧seq推进到batch末seq。较晚batch worker先抢到时，其外部future不得成功或发布；`PROJECTION_PREDECESSOR_PENDING`只允许作为projector内部等待/重试状态，scheduler必须持续等待或requeue，直到前驱receipt提交后再处理该batch。任一步失败时entity version/change/compatibility/receipt/head全部rollback，但source canonical transaction保持已提交并可安全重试projector。reader先在`REPEATABLE READ READ ONLY` snapshot捕获head，只读取`change_seq <= captured_head.change_seq`的materialized versions；因此并发reader只看见完整旧batch或完整新batch，绝不能看到半批entity、已推进head但缺version，或canonical表与projection临时拼接的混合视图。rebuild同样严格按source batch seq重放，因此source order、change seq、entity version、event UUID及排除时间字段的canonical manifest hash稳定；新的`projected_at`允许不同且不参与receipt manifest/hash。

`source_occurred_at`表示source authority事实发生时间：新source row只能取canonical DB transaction timestamp，不能取模型、command或caller时间；`projected_at`取projector transaction timestamp。两者都不参与semantic/projection hash或event identity，Timeline按`change_seq,event_id`排序而非按时钟排序。历史row有可信created/terminal时间时原样保留并标`known`；没有可信时间时`source_occurred_at=NULL, source_time_status=historical_unknown`，禁止把migration/projected time伪装成source发生时间。

- `investigation_projection_compare_samples`：A–D 唯一 per-record comparison ledger。Plan B 的 `investigation_comparison.rs` + `investigation_projection/comparison.rs` 拥有 canonical serializer/hash writer；`hypothesis_legacy_projection.rs`只构造两侧完整 record。Plan C/D 只提供新增 canonical fields，不创建第二张 sample 表或第二个 comparator truth。schema 从一开始冻结为：

```sql
CREATE TABLE investigation_projection_compare_samples (
    comparison_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    project_scope_id UUID NOT NULL,
    organization_id UUID,
    projection_schema_version INTEGER NOT NULL DEFAULT 1
        CHECK (projection_schema_version = 1),
    as_of_change_seq BIGINT NOT NULL CHECK (as_of_change_seq >= 0),
    comparison_contract_version TEXT NOT NULL DEFAULT 'comparison_record.v1'
        CHECK (comparison_contract_version = 'comparison_record.v1'),
    tool_truth_contract TEXT NOT NULL CHECK (
        tool_truth_contract IN ('legacy_v1','shadow_v1','receipt_v1')
    ),
    investigation_contract_version TEXT NOT NULL CHECK (
        investigation_contract_version IN ('legacy_candidate_v1','hypothesis_registry_v1')
    ),
    investigation_rollout_mode TEXT NOT NULL CHECK (
        investigation_rollout_mode IN (
            'legacy_only','shadow_registry','dual_read_compare',
            'registry_authoritative_legacy_projection','new_only'
        )
    ),
    CHECK (operation_joint_contract_rank(
        tool_truth_contract,
        investigation_contract_version,
        investigation_rollout_mode
    ) IS NOT NULL),
    record_kind TEXT NOT NULL CHECK (BTRIM(record_kind) <> ''),
    record_key TEXT NOT NULL CHECK (BTRIM(record_key) <> ''),
    legacy_hash TEXT CHECK (legacy_hash IS NULL OR legacy_hash ~ '^sha256:[0-9a-f]{64}$'),
    registry_hash TEXT CHECK (registry_hash IS NULL OR registry_hash ~ '^sha256:[0-9a-f]{64}$'),
    comparison_state TEXT NOT NULL CHECK (comparison_state IN (
        'match','mismatch','registry_missing','legacy_projection_missing',
        'incomplete','authority_corrupt'
    )),
    diff_summary JSONB NOT NULL DEFAULT '{}'::JSONB,
    compared_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (operation_id, as_of_change_seq, record_kind, record_key),
    FOREIGN KEY (operation_id, project_scope_id)
        REFERENCES operation_state(operation_id, project_scope_id) ON DELETE RESTRICT
);
```

sample writer必须从同一locked `operation_state`复制三项frozen contract并用`operation_rollout::validate_joint_pair`验证，caller不能传入。`diff_summary`只保存differing path名、reason code和两侧member counts，不保存具体值、raw payload或secret。任一side必须先形成complete record或明确missing/incomplete/corrupt；禁止per-field fallback。

给 `attack_candidate_work_items` 和 `attack_candidates` 只加 nullable `hypothesis_revision_id`、`legacy_projection_id` link，用于legacy-source correlation；保留所有旧 ID、hash 与 compound FK。canonical-derived projector不得回写这些legacy source rows，异步兼容数据只写独立append-only Candidate/Attempt projection-version tables。

对 ledger/artifact/census/event/outbox/source-blob/entity-version/receipt/change 表安装共用 append-only trigger。允许 mutation 的只有显式 CAS heads与work item lifecycle；outbox完成状态由append-only batch receipt证明，不逐row UPDATE processed flag，canonical history不update/delete。

### Step 7：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(hypothesis_registry_schema_) | test(hypothesis_state_authority_schema_) | test(candidate_analysis_attempt_schema_) | test(candidate_snapshot_tool_truth_authority_schema_) | test(candidate_knowledge_feed_schema_) | test(candidate_input_chunk_census_schema_) | test(hypothesis_coverage_review_schema_) | test(verification_contract_schema_) | test(hypothesis_claim_component_schema_) | test(hypothesis_verification_plan_schema_) | test(projection_catalog_schema_) | test(projection_plan_b_verification_plan_route_) | test(projection_plan_c_route_catalog_) | test(projection_source_snapshot_schema_) | test(projection_entity_predecessor_schema_) | test(projection_batch_schema_)')
```

Expected：schema/default/immutability/unique/append-only/phase-transition tests全部`PASS`；snapshot bundle seal/root/member/receipt/graph/semantic/freshness/temporal字段必须FK/identity/hash一致且只能从Plan A Checked guard复制，caller裸root list/seal/hash、cross-org root或旧consistent row不能成ready snapshot，non-all-fresh exact写blocked snapshot+census/residual；signed CVE/CPE/KEV/advisory/rule feed、product-version与deterministic match census exact seal，stale/invalid feed或unknown version只能残差/obligation，knowledge signal不能填proof role；每attempt H1/H2/disposition exact set、每input immutable coverage review/checklist/subreview/synthesis exact set、完整chunk+H1 read binding、sample→blocked residual约束、VerificationContract predicate/control/pair exact set、每revision claim-component与`HypothesisVerificationPlanV1` objective/component/path/member exact set、revision↔creating-event authority及Plan B terminal-origin拒绝、outbox frozen source snapshot、entity direct predecessor、batch/change/head FK与两时间CHECK均由direct SQL正反例证明；catalog CHECK与计划冻结词汇完全一致，Plan B plan seal与Plan C每个canonical mutation都exact-one映射到专属entity或typed Coverage aggregate且unknown mutation在canonical commit前被拒；`to_regclass('verification_capability_assessments') IS NULL`及Plan C adjudication/transition tables不存在证明Plan B migration没有抢占Plan C-owned authority；test输出确认migration文件名恰为`20260729000006_hypothesis_registry.sql`。

### Future Commit

```bash
git add backend/crates/golish-db/migrations/20260729000006_hypothesis_registry.sql backend/crates/golish-db/tests/hypothesis_registry.rs
git commit -m "feat(db): add hypothesis registry schema"
```

---

## Task 3：冻结 operation mode、resume 与 fork adoption

> **2026-07-30 implementation correction (fork inheritance):** `operation_state`
> must exist before the existing `operation_stage_forks` row can reference it, so
> a synchronous insert trigger cannot distinguish an unauthorized non-default pair
> from a no-adoption fork inheriting an older source pair. The sole `00006` migration
> therefore installs a deferred constraint trigger: at commit it first verifies an
> exact adoption receipt or exact source/target fork inheritance, and only then reads
> Tool Truth followed by Investigation deployment defaults for an ordinary fresh
> operation. Plan B still exposes no promotion setter. The adoption set hash excludes
> the not-yet-allocated target operation id and binds source operation/scope, adopted
> stage exact set, source final-seal census and both joint pairs; the final receipt
> additionally binds target operation and stable request id.

**文件：**

- 创建：`backend/crates/golish-db/src/repo/investigation_rollout.rs`
- 创建：`backend/crates/golish-db/src/repo/operation_rollout.rs`
- 修改：Plan A 已创建的 `backend/crates/golish-db/src/repo/tool_truth_rollout.rs`
- 修改：`backend/crates/golish-db/src/repo/{mod.rs,operation_state.rs,runtime_memory_tx.rs}`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/{runtime_memory.rs,types.rs}`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- 测试：`backend/crates/golish-db/tests/hypothesis_registry.rs`

### Step 1：添加 RED tests

覆盖：fresh operation 按固定锁序从 Tool Truth + Investigation 两个 singleton 同事务冻结合法 joint pair；历史 row 为 joint rank 0；same-operation resume 返回完整同 pair；任一 deployment default 变化不改已有 operation；并发 joint promotion只能观察完整旧pair或完整新pair；fork无adoption时精确继承source两轴；显式升级必须写统一receipt且只能`rank + 1`；伪造source seal/adoption hash拒绝。

关键断言：

```rust
assert_eq!(created.investigation_contract_version, InvestigationContractVersion::LegacyCandidateV1);
assert_eq!(created.investigation_rollout_mode, InvestigationRolloutMode::LegacyOnly);
assert_eq!(created.tool_truth_contract, ToolTruthContract::LegacyV1);
assert_eq!(joint_contract_rank(&created), 0);
assert_eq!(resumed.investigation_rollout_mode, created.investigation_rollout_mode);
assert_eq!(resumed.tool_truth_contract, created.tool_truth_contract);
assert_eq!(
    adoption_error.code(),
    "STAGE_FORK_OPERATION_CONTRACT_ADOPTION_RECEIPT_REQUIRED"
);
```

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(operation_) | test(stage_fork_)')
```

Expected：新字段尚未从 repo 映射，tests 编译失败或断言 legacy field missing。

### Step 3：实现两个只读 rollout repo、joint admission 与 operation mapping

`investigation_rollout.rs` 只暴露：

```rust
pub async fn get_for_share(
    connection: &mut sqlx::PgConnection,
) -> crate::Result<InvestigationRolloutRow>;

pub fn parse_frozen_pair(
    contract: &str,
    mode: &str,
) -> crate::Result<(InvestigationContractVersion, InvestigationRolloutMode)>;
```

Plan A `tool_truth_rollout.rs`继续只暴露transaction-bound share read。新`operation_rollout.rs`拥有七个合法Tool Truth + Investigation pair的`joint_contract_rank`、`validate_joint_pair`、operation creation/fork admission与统一adoption receipt writer；不要添加`advance_*`、`promote_*`、环境变量reconciliation或Tauri command。

把 Investigation 两个字段加入并保留 Plan A 已加入的 Tool Truth 字段：

- `OperationStateRow`、`OperationStateView`；
- `INSERT_OPERATION_SQL`、`INSERT_OPERATION_WITH_EXECUTOR_SQL`；
- `OPERATION_STATE_ROW_COLUMNS` 与每个 `SELECT operation_id,profile,current_stage...`；
- `CreatedRuntimeOperationRow` 与 app bridge conversion。

### Step 4：在 operation creation transaction 原子冻结 joint pair

在`create_runtime_operation_inner`中取得既有runtime/attack rollout lock后，固定先`tool_truth_rollout FOR SHARE`、再`investigation_rollout FOR SHARE`；调用`operation_rollout::validate_joint_pair`后一次INSERT operation。与Plan D promotion并发时，只能读到完整旧joint pair或完整新joint pair，禁止先插入再补字段。非fork使用两个singleton；fork只用source完整pair或显式adoption target，不读取singleton决定target。

为`StageForkCreate` / `StageForkCreateRow`增加：

```rust
pub struct OperationContractForkAdoption {
    pub request_id: String,
    pub target_tool_truth_contract: ToolTruthContract,
    pub target_investigation_contract_version: InvestigationContractVersion,
    pub target_investigation_rollout_mode: InvestigationRolloutMode,
    pub source_final_seal_hash: String,
    pub adoption_exact_set_hash: String,
}
```

无adoption时target完整joint pair必须等于source；有adoption时repo锁source operation/final seals，派生source/target rank并只接受`target = source + 1`，重算adoption exact-set/receipt hash，在创建target operation与fork edge的同一事务插入统一`operation_contract_adoptions`。caller不能自报source contract/rank。未知pair、任意导致非法pair的一轴改动、跳级/倒退、source seal drift、response-loss request drift全部fail closed。

### Step 5：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(operation_) | test(stage_fork_)')
```

Expected：全部`PASS`，并证明joint pair无torn read、改变singleton不改变已存在operation、resume/fork精确保留两轴、adoption最多前进一阶。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/tool_truth_rollout.rs backend/crates/golish-db/src/repo/investigation_rollout.rs backend/crates/golish-db/src/repo/operation_rollout.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-kit/src/db_traits/types.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-db/tests/hypothesis_registry.rs
git commit -m "feat(investigation): freeze operation rollout mode"
```

---

## Task 4：实现 semantic key、host-owned VerificationContract、root/revision identity 与 reducer

**文件：**

- 创建：`backend/crates/golish-agent-kit/src/harness/hypothesis_registry/{mod.rs,types.rs,semantic_key.rs,verification_contract_compiler.rs,verification_plan_compiler.rs,reducer.rs,rollout.rs}`
- 创建：`backend/crates/golish-core/src/verification_contract.rs`
- 创建：`backend/crates/golish-core/src/hypothesis_verification.rs`
- 创建：`backend/crates/golish-core/src/investigation_projection.rs`
- 修改：`backend/crates/golish-core/src/lib.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/mod.rs`
- 创建：`backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs`

### Step 1：写 semantic identity RED

测试必须证明 prose/confidence/priority/tags/evidence/proposer 不影响 key；organization、at-time subject、predicate version/args、trust boundary、polarity 任一变化都会改变 key；nested JSON map 顺序不影响 hash。

同一RED批次还要证明：Controller/analyst artifact无法提交contract id/hash、predicate component或required control；host compiler对相同objective/policy输入无论map/member顺序都生成相同`verification_contract.v1` hash；漏/重一个component/control、paired binding control id相同但version/contract hash/member hash不同、改变combinator/version/compiler digest都会改变或拒绝contract；任意caller伪造member count/set hash失败。host从typed claim派生claim-clause/impact/trust-boundary/identity `HypothesisClaimComponentV1` exact set；每revision `HypothesisVerificationPlanV1`必须与sealed claim-components + objectives + contracts exact-equal且canonical-order/hash稳定。property tests生成不同组件/路径排列并证明：每path component union全覆盖才可seal/verified，每path显式falsifier绑定具体component才可refuted；漏required component/objective要么稳定拒绝，要么只创建narrow successor且原宽claim非终态。proof path/member漏重、ordinal跳号、objective未被任一path引用、path无显式falsifier、stale objective、contract substitution、单个Campaign terminal冒充plan outcome set、outcome member未绑定plan/component、revision adjudication漏objective或transition receipt均拒绝。Plan B authority-adapter tests还要证明它只能消费Plan A sealed `EvidenceTemporalValidityPolicyV1`/Checked bundle decisions，expired/target-state epoch/max-skew/orphan fail closed且caller时间/epoch/hash无constructor入口；TTL ordering本身由Plan A policy/schema tests拥有，B不复制规则。Controller若在Candidate mutation中提交`verified`、`refuted`或`invalid`必须分别得到`HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN`或`HYPOTHESIS_INVALID_STATE_SERVER_ONLY`，且mutation set为空。

path-level truth-table RED必须覆盖：winning path全required proof而其他path含`Unassigned|Blocked`仍`Verified`且这些项只进non-decisive limitations；每path各有一个designated valid required-component falsifier而其余member未决仍`Refuted`；任一live path既未全proof也无valid falsifier时才`NonTerminal`并把decision-blocking members exact收入unresolved；optional-only falsifier、漏required component proof、或把limitation挪进decisive set均稳定拒绝。

```rust
#[test]
fn non_identity_fields_do_not_change_semantic_key_or_initial_root() {
    let first = fixture_claim("first prose", 90, vec!["CWE-639"]);
    let second = fixture_claim("second prose", 10, vec!["OWASP-API1"]);
    let first_key = HypothesisSemanticKeyV1::from_claim(&first).unwrap();
    let second_key = HypothesisSemanticKeyV1::from_claim(&second).unwrap();
    assert_eq!(first_key.hash().unwrap(), second_key.hash().unwrap());
    assert_eq!(
        initial_root_id(first.operation_id, first.organization_id, &first_key).unwrap(),
        initial_root_id(second.operation_id, second.organization_id, &second_key).unwrap()
    );
}
```

另写 provider completion order test：对相同 proposal census 的所有排列执行 reducer，最终 root/revision mutation set hash 相同；projection catalog test对四个enum的`ALL/as_str/TryFrom`做exhaustive round-trip，并拒绝unknown string。`projection_ts_decl_golden_`只在内存比较四个core enum的`TS::decl()`与checked-in期望wire union；PAUSE B前绝不调用export或写`frontend/lib/generated/`。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate -E 'test(semantic_) | test(non_identity_) | test(provider_completion_) | test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(hypothesis_revision_adjudication_) | test(candidate_terminal_state_)')
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(hypothesis_revision_adjudication_) | test(investigation_projection_catalog_) | test(projection_plan_c_route_catalog_) | test(projection_ts_decl_golden_)')
```

Expected：semantic/contract/verification-plan/revision-adjudication/Candidate-state与projection catalog/TS declaration模块或类型未定义而编译失败；不能只因semantic tests命中就漏掉本Task其余RED。

### Step 3：实现 canonical key/hash、Candidate state boundary 与 UUIDv5 formulas

核心类型固定为：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HypothesisSemanticKeyV1 {
    pub schema: String,
    pub organization_id: Uuid,
    pub subject: AtTimeSubjectIdentity,
    pub predicate: PredicateIdentity,
    pub trust_boundary: String,
    pub polarity: ClaimPolarity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtTimeSubjectIdentity {
    pub kind: String,
    pub identity_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PredicateIdentity {
    pub schema: String,
    pub version: u32,
    pub normalized_arguments: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateMutationEpistemicState {
    Proposed,
    Supported,
    Contested,
    Inconclusive,
}
```

hash 为 `sha256("hypothesis_semantic_key.v1\0" || canonical_json)`，结果带 `sha256:`。live target UUID 不进入 key。

Candidate proposal、Controller decision与`CandidateGatePass`只能携带`CandidateMutationEpistemicState`，不能复用包含终态的全局`EpistemicState`。`attack_hypothesis_revisions`仍保留`verified/refuted/invalid`以供后续权威transition，但Plan B Candidate repo不暴露写这些值的方法。`invalid`只能由server deterministic contract/authority validator经独立`invalidate_revision_from_server_validation`写入；模型、Controller、legacy projection和任意command都不能选择它。

UUIDv5 公式：

```rust
let namespace = Uuid::new_v5(
    &Uuid::NAMESPACE_URL,
    format!(
        "golish:hypothesis-registry:v1:operation:{operation_id}:organization:{organization_id}"
    )
    .as_bytes(),
);
let initial = Uuid::new_v5(&namespace, format!("initial:{semantic_key_hash}").as_bytes());
let split = Uuid::new_v5(
    &namespace,
    format!("split:{parent_root_id}:{child_semantic_key_hash}").as_bytes(),
);
let merge = Uuid::new_v5(
    &namespace,
    format!("merge:{}:{successor_semantic_key_hash}", sorted_parent_roots.join(",")).as_bytes(),
);
let derive = Uuid::new_v5(
    &namespace,
    format!(
        "derive:{source_root_id}:{source_revision_id}:{derivation_rule_hash}:{child_semantic_key_hash}"
    )
    .as_bytes(),
);
```

revision ID 使用 root namespace：Candidate/nonterminal为`revision:{ordinal}:{semantic_key_hash}:{origin_decision_hash}`；未来terminal为`revision:{ordinal}:{semantic_key_hash}:{adjudication_hash}:{transition_decision_hash}`。ordinal仍必须由锁定predecessor + 1决定；revision ingredients绝不包含creating `state_event_hash`或最终transition receipt hash，event只能在revision identity已确定后计算，避免revision↔event↔receipt hash环。

### Step 3b：实现 host-owned `verification_contract.v1`

`golish-core/src/verification_contract.rs`定义并导出唯一closed DTO、canonical hash与validator；所有字段私有。`golish-agent-kit/.../verification_contract_compiler.rs`只是host compiler/reducer wrapper，从已sealed revision/objective、predicate registry、policy snapshot与control registry构造core builder input并调用core validator，不复制enum/hash规则：

```rust
pub enum ContractCombinatorV1 {
    AllOf,
    AnyOf,
    PairedDifferential,
    OrderedSequence,
}

pub struct PredicateComponentV1 {
    semantic_key: String,
    predicate_schema: String,
    predicate_version: u32,
    normalized_arguments: CanonicalJsonObject,
    expected_polarity: ClaimPolarity,
    prerequisite_hash: String,
}

pub struct VerificationControlV1 {
    control_id: String,
    control_version: u32,
    control_contract_hash: String,
}

pub enum OrderedSessionScopeV1 { SameExecutionSession }
pub enum OrderedInterleavingPolicyV1 { Forbid }
pub enum OrderedResetPolicyV1 { RestartAtStepZero }

pub struct PairedDifferentialBindingV1 {
    pair_key: String,
    baseline_component_key: String,
    variant_component_key: String,
    required_control_id: String,
    required_control_version: u32,
    required_control_contract_hash: String,
    required_control_member_hash: String,
    comparator_rule_id: String,
    comparator_rule_version: u32,
    comparator_rule_digest: String,
}

pub struct OrderedSequenceStepV1 {
    step_ordinal: u32,
    component_key: String,
    predecessor_step_ordinal: Option<u32>,
    session_binding_key_schema: String,
    session_binding_key_version: u32,
    session_scope: OrderedSessionScopeV1,
    interleaving_policy: OrderedInterleavingPolicyV1,
    reset_policy: OrderedResetPolicyV1,
}

pub struct VerificationContractV1 {
    contract_version: u32,
    objective_id: Uuid,
    combinator: ContractCombinatorV1,
    predicate_components: Vec<PredicateComponentV1>,
    predicate_member_set_hash: String,
    required_controls: Vec<VerificationControlV1>,
    required_control_member_set_hash: String,
    explicit_no_required_control: bool,
    paired_differential_bindings: Vec<PairedDifferentialBindingV1>,
    paired_binding_set_hash: String,
    ordered_steps: Vec<OrderedSequenceStepV1>,
    ordered_step_set_hash: String,
    stopping_criteria_hash: String,
    compiler_digest: String,
    policy_snapshot_hash: String,
    contract_hash: String,
}
```

`ContractCombinatorV1::as_str/TryFrom<&str>`与serde wire值精确闭合为`all_of|any_of|paired_differential|ordered_sequence`；unknown值（包括`threshold`）直接typed error，DB CHECK使用同一四值集合。

`CanonicalJsonObject`是core-owned、key canonical-sort且拒绝non-finite/duplicate/unsupported value的private newtype，不是任意`serde_json::Value`写口；上述ordered policy在V1均为单一闭集值，未来若需要允许interleaving或不同reset语义必须升contract version，不能用prose/nullable字段改变V1。

compiler先按semantic key canonical-sort并拒绝duplicate；predicate components必须非空；required controls为空时必须`explicit_no_required_control=true`，非空时必须false。`all_of/any_of`的pair/ordered集合必须都为空；`paired_differential`必须让每个component exact-one出现在baseline或variant role、每pair exact-one绑定当前contract required-control member的id/version/contract hash/member hash与versioned comparator，任何missing/extra/orphan/stale control或同id不同version/hash都拒绝；`ordered_sequence`必须至少两步、ordinal从0连续、每步只引用一个component、全部要求同一versioned session binding，前驱/顺序/interleaving/reset语义闭合，不能跨action/session补齐。count/set hash由host重算，hash使用domain-separated length-prefixed canonical fields，不接受artifact/request中的hash。

真值结构也从V1冻结：`all_of`只有全部component terminal-satisfied才成立；`any_of`任一terminal-satisfied可成立，但只有全部component在完整control下terminal-refuted才可refute；`paired_differential`必须每pair baseline+variant+control+comparator均完整；`ordered_sequence`必须同session按连续step顺序满足，缺步、乱序、跨session、非法interleaving或reset后一律inconclusive。Plan B只冻结结构而不产终态；Plan C必须直接import`golish_core::verification_contract::{VerificationContractV1, ContractCombinatorV1, PredicateComponentV1, VerificationControlV1, PairedDifferentialBindingV1, OrderedSequenceStepV1, OrderedSessionScopeV1, OrderedInterleavingPolicyV1, OrderedResetPolicyV1}`并消费持久exact set，禁止重定义第二个`VerificationContract`、`ContractCombinator`或从prose重建predicate/control。

### Step 3c：冻结 revision-level verification plan 与终态 authority

`golish-core/src/hypothesis_verification.rs`拥有唯一public types；字段私有、constructor只接受server-loaded sealed DTO，所有count/set hash与final hash均由core重算：

```rust
pub enum HypothesisClaimComponentKindV1 {
    ClaimClause,
    ImpactQualifier,
    TrustBoundaryCondition,
    IdentityCondition,
}

pub struct HypothesisClaimComponentV1 {
    revision_id: Uuid,
    revision_hash: String,
    component_ordinal: u32,
    component_key: String,
    kind: HypothesisClaimComponentKindV1,
    canonical_fragment_hash: String,
    canonical_condition_hash: String,
    derivation_contract_version: u32,
    derivation_contract_digest: String,
    required: bool,
    member_hash: String,
}

pub enum HypothesisVerificationObjectiveOutcomeRequirementV1 {
    SatisfyBoundComponents,
    SatisfyOrFalsifyBoundRequiredComponents,
}

pub struct HypothesisVerificationPlanObjectiveV1 {
    objective_id: Uuid,
    objective_hash: String,
    verification_contract_id: Uuid,
    verification_contract_version: u32,
    verification_contract_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    stopping_criteria_hash: String,
    outcome_requirement: HypothesisVerificationObjectiveOutcomeRequirementV1,
    member_hash: String,
}

pub enum HypothesisVerificationPlanPathMemberRoleV1 {
    RequiredProof,
    RequiredProofAndPathFalsifier,
}

pub struct HypothesisVerificationPlanPathMemberV1 {
    member_ordinal: u32,
    plan_objective_member_hash: String,
    verification_contract_hash: String,
    claim_component_set_hash: String,
    role: HypothesisVerificationPlanPathMemberRoleV1,
    falsifier_claim_component_member_hashes: Vec<String>,
    falsifier_claim_component_set_hash: String,
    member_hash: String,
}

pub struct HypothesisVerificationPlanPathV1 {
    path_ordinal: u32,
    path_key: String,
    members: Vec<HypothesisVerificationPlanPathMemberV1>,
    member_count: u32,
    member_set_hash: String,
    path_hash: String,
}

pub struct HypothesisVerificationPlanV1 {
    plan_id: Uuid,
    plan_version: u32,
    revision_id: Uuid,
    revision_hash: String,
    revision_ingredients_hash: String,
    required_claim_components: Vec<HypothesisClaimComponentV1>,
    required_claim_component_count: u32,
    required_claim_component_set_hash: String,
    objectives: Vec<HypothesisVerificationPlanObjectiveV1>,
    objective_count: u32,
    objective_set_hash: String,
    proof_paths: Vec<HypothesisVerificationPlanPathV1>,
    proof_path_count: u32,
    proof_path_set_hash: String,
    outer_aggregation_policy_version: u32,
    outer_aggregation_policy_digest: String,
    plan_hash: String,
}

pub enum HypothesisClaimComponentOutcomeKindV1 {
    Satisfied,
    Refuted,
    Inconclusive,
    Blocked,
    Unassigned,
    Invalidated,
}

pub enum HypothesisVerificationObjectiveOutcomeKindV1 {
    Satisfied,
    Refuted,
    Inconclusive,
    Blocked,
    ExhaustedWithResiduals,
    Unassigned,
    Invalidated,
}

pub struct HypothesisComponentProofRefV1 {
    claim_component_member_hash: String,
    predicate_component_member_hash: String,
    oracle_receipt_id: Uuid,
    oracle_receipt_hash: String,
    coverage_receipt_hash: String,
    fact_delta_consumption_set_hash: String,
    member_hash: String,
}

pub struct HypothesisComponentRefutationRefV1 {
    claim_component_member_hash: String,
    predicate_component_member_hash: String,
    required_control_set_hash: String,
    oracle_receipt_id: Uuid,
    oracle_receipt_hash: String,
    coverage_receipt_hash: String,
    fact_delta_consumption_set_hash: String,
    member_hash: String,
}

pub enum HypothesisClaimComponentOutcomeLineageV1 {
    Satisfied {
        proof_members: Vec<HypothesisComponentProofRefV1>,
        proof_member_count: u32,
        proof_member_set_hash: String,
    },
    Refuted {
        refutation_members: Vec<HypothesisComponentRefutationRefV1>,
        refutation_member_count: u32,
        refutation_member_set_hash: String,
    },
    NonTerminal { residual_risk_set_hash: String },
}

pub struct HypothesisClaimComponentOutcomeV1 {
    claim_component_member_hash: String,
    outcome: HypothesisClaimComponentOutcomeKindV1,
    lineage: HypothesisClaimComponentOutcomeLineageV1,
    outcome_hash: String,
}

pub struct CampaignTerminalReceiptRefV1 {
    receipt_id: Uuid,
    receipt_version: u32,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    all_fresh_authority_binding_hash: String,
    member_hash: String,
}

pub struct OracleCensusReceiptRefV1 {
    receipt_id: Uuid,
    receipt_version: u32,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_member_hashes: Vec<String>,
    claim_component_count: u32,
    claim_component_set_hash: String,
    oracle_member_set_hash: String,
    all_fresh_authority_binding_hash: String,
    member_hash: String,
}

pub struct CampaignCoverageReceiptRefV1 {
    receipt_id: Uuid,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_set_hash: String,
    denominator_member_set_hash: String,
    member_hash: String,
}

pub struct FactDeltaConsumptionReceiptRefV1 {
    receipt_id: Uuid,
    receipt_hash: String,
    plan_objective_member_hash: String,
    claim_component_set_hash: String,
    delta_set_hash: String,
    member_hash: String,
}

pub struct HypothesisVerificationObjectiveOutcomeV1 {
    outcome_receipt_id: Uuid,
    outcome_receipt_version: u32,
    outcome_ordinal: u32,
    predecessor_outcome_receipt_id: Option<Uuid>,
    predecessor_outcome_receipt_hash: Option<String>,
    campaign_head_hash: String,
    plan_objective_member_hash: String,
    verification_contract_hash: String,
    claim_component_outcomes: Vec<HypothesisClaimComponentOutcomeV1>,
    claim_component_outcome_count: u32,
    claim_component_outcome_set_hash: String,
    outcome: HypothesisVerificationObjectiveOutcomeKindV1,
    campaign_terminal_receipts: Vec<CampaignTerminalReceiptRefV1>,
    campaign_terminal_receipt_count: u32,
    campaign_terminal_receipt_set_hash: String,
    oracle_census_receipts: Vec<OracleCensusReceiptRefV1>,
    oracle_census_receipt_count: u32,
    oracle_census_receipt_set_hash: String,
    coverage_receipts: Vec<CampaignCoverageReceiptRefV1>,
    coverage_receipt_count: u32,
    coverage_receipt_set_hash: String,
    fact_delta_consumption_receipts: Vec<FactDeltaConsumptionReceiptRefV1>,
    fact_delta_consumption_receipt_count: u32,
    fact_delta_consumption_receipt_set_hash: String,
    unassigned_residual_risk_set_hash: String,
    outcome_lineage_hash: String,
    outcome_hash: String,
}

pub enum HypothesisRevisionAdjudicationVerdictV1 {
    Verified,
    Refuted,
    NonTerminal,
}

pub enum HypothesisRevisionNonTerminalReasonV1 {
    Inconclusive,
    Blocked,
    ExhaustedWithResiduals,
    Unassigned,
    Invalidated,
}

pub enum HypothesisRevisionAdjudicationLineageV1 {
    Verified { finding_id: Uuid, finding_lineage_hash: String },
    Refuted {
        refutation_receipt_id: Uuid,
        predicate_component_set_hash: String,
        required_control_set_hash: String,
    },
    NonTerminal {
        reason: HypothesisRevisionNonTerminalReasonV1,
        residual_risk_set_hash: String,
    },
}

pub struct HypothesisUnresolvedOutcomeRefV1 {
    plan_objective_member_hash: String,
    claim_component_member_hash: Option<String>,
    outcome_hash: String,
    member_hash: String,
}

pub struct PersistedAllFreshToolTruthAuthorityBindingV1 {
    bundle_seal_id: Uuid,
    relevant_root_set_hash: String,
    bundle_member_set_hash: String,
    receipt_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_hash: String,
    temporal_validity_decision_set_hash: String,
    observation_window_hash: String,
    target_state_epoch_set_hash: String,
    earliest_effective_valid_until: DateTime<Utc>,
    binding_hash: String,
}

pub struct HypothesisRevisionAdjudication {
    adjudication_id: Uuid,
    adjudication_version: u32,
    revision_id: Uuid,
    revision_hash: String,
    verification_plan_id: Uuid,
    verification_plan_hash: String,
    objective_outcomes: Vec<HypothesisVerificationObjectiveOutcomeV1>,
    objective_outcome_count: u32,
    objective_outcome_set_hash: String,
    unresolved_outcomes: Vec<HypothesisUnresolvedOutcomeRefV1>,
    unresolved_outcome_count: u32,
    unresolved_outcome_set_hash: String,
    non_decisive_limitations: Vec<HypothesisUnresolvedOutcomeRefV1>,
    non_decisive_limitation_count: u32,
    non_decisive_limitation_set_hash: String,
    all_fresh_authority_binding: PersistedAllFreshToolTruthAuthorityBindingV1,
    verdict: HypothesisRevisionAdjudicationVerdictV1,
    adjudication_lineage: HypothesisRevisionAdjudicationLineageV1,
    adjudication_hash: String,
}

pub struct HypothesisRevisionTransitionDecisionV1 {
    decision_id: Uuid,
    predecessor_revision_id: Uuid,
    successor_epistemic_state: HypothesisTerminalEpistemicStateV1,
    verification_plan_id: Uuid,
    verification_plan_hash: String,
    adjudication_id: Uuid,
    adjudication_hash: String,
    objective_outcome_set_hash: String,
    all_fresh_authority_binding_hash: String,
    decision_hash: String,
}

pub struct HypothesisRevisionTransitionReceiptV1 {
    receipt_id: Uuid,
    transition_decision_id: Uuid,
    transition_decision_hash: String,
    successor_revision_id: Uuid,
    successor_revision_hash: String,
    state_event_hash: String,
    receipt_hash: String,
}

pub struct HypothesisRevisionAdjudicationAuthorityV1 {
    verification_plan: HypothesisVerificationPlanV1,
    adjudication: HypothesisRevisionAdjudication,
    transition_decision: HypothesisRevisionTransitionDecisionV1,
    transition_receipt: HypothesisRevisionTransitionReceiptV1,
}
```

host先从typed predicate/structured claim按versioned derivation rule生成`HypothesisClaimComponentV1` exact set；每member绑定revision id/hash、连续ordinal、canonical fragment/condition hashes及derivation version/digest，覆盖claim clauses、impact qualifiers、trust-boundary与identity conditions，禁止跨revision或旧derivation substitution。plan objective集合必须与该revision全部sealed objective + host-owned contract exact-equal，每objective冻结closed outcome requirement并绑定一个非空component subset，不能由Controller删掉难验证的objective/component或让一个Campaign隐式代表多个objective。host compiler还必须生成至少一条ordinal从0连续的sealed proof path；每path成员连续且只引用plan objective exact set，每objective至少出现在一条path。每个falsifier set必须是`objective component subset ∩ revision required-component exact set`的非空子集（`RequiredProof`则必须显式空falsifier set）；仅optional qualifier被refuted只能触发qualifier/narrow-successor处理，绝不能计入宽claim outer refutation。每条path的component union必须exact覆盖全部required claim components。path/member/component count/set hash均不可由caller提供。outer aggregation使用显式全称/存在量词：`Verified := ∃ path, ∀ required member/component in path, valid Satisfied`；`Refuted := ∀ path, ∃ explicit path-falsifier whose bound required component has valid Refuted`；否则无论多少Campaign已terminal都只能typed `NonTerminal`，并携带`Inconclusive|Blocked|ExhaustedWithResiduals|Unassigned|Invalidated` reason与unresolved objective/component exact set。不能采用“所有objective都必须Satisfied”的隐含AND、任一objective refuted即全局refuted、optional-only falsifier、模型prose或Campaign completion顺序替代该path reducer。

若Controller objective intent没有覆盖required component exact set，compiler返回`HYPOTHESIS_VERIFICATION_PLAN_CLAIM_COMPONENT_UNCOVERED`且原revision plan不seal。host reducer可选择显式`NarrowSuccessor`：以covered component exact set派生新的narrow semantic claim/revision、relation与独立plan，原宽claim保持nonterminal并写uncovered-component residual；禁止把narrow successor的Finding/refutation lineage回填成原claim终态。

Plan C为每个plan objective产生exact-one current outcome；objective闭集为`Satisfied|Refuted|Inconclusive|Blocked|ExhaustedWithResiduals|Unassigned|Invalidated`，component使用独立闭集且不允许`ExhaustedWithResiduals`冒充component事实。outcome receipt以连续ordinal、predecessor id/hash与locked campaign head形成latest-eligible CAS chain；每个outcome携带typed Campaign terminal/oracle/coverage/FactDelta-consumption receipt vectors及逐component proof/refutation members，count/set hash只是这些server-loaded refs的重算结果，caller不能只报hash。每个receipt ref的component set必须是当前plan objective component subset与plan required-component denominator的子集；oracle/coverage/FactDelta或proof/refutation member若绑定另一个objective/component，即使内容hash相同也拒绝，防止A的oracle重放给B。Campaign terminal只是outcome evidence member，绝不是revision authority。`Verified`仅由`∃ winning path, ∀该path required member/component valid Satisfied`决定；`Refuted`仅由`∀path, ∃该path designated falsifier在绑定required component上valid Refuted`决定。满足任一外层公式后，其他路径或路径内非决定性的unassigned/invalidated/inconclusive/blocked/exhausted项进入`non_decisive_limitations` exact set并随Finding/refutation/report limitation保留，不能反过来否决已成立的终态。只有两条外层公式都不成立时verdict才为`NonTerminal`；此时`unresolved_outcomes`必须exact-equal所有仍可能改变live-path判定的blocking objective/component members，reason precedence固定为`Invalidated > Blocked > Unassigned > ExhaustedWithResiduals > Inconclusive`。validator还要求winning path全required component proof、每条被falsify path至少一个designated valid required-component falsifier，并拒绝把non-decisive limitation伪装成decisive proof/falsifier。

Plan B不实现终态写入，但冻结Plan C唯一允许的seam：public repo入口必须在Plan A `with_all_fresh_tool_truth_authority_bundle` callback内取得同request opaque guard，再调用module-private `apply_verification_terminal_transition(authority, &AllFreshToolTruthAuthorityBundle<'_>)`；函数无无guard overload，`golish-core`只定义persisted DTO/hash validator，不依赖pentest-domain guard类型。它在同一Plan C transaction中验证B-owned plan、typed objective/component receipt vectors、adjudication、transition decision与transition receipt exact binding，以B-owned proof-path reducer重算aggregate decision，并将live guard的bundle seal/root/member/receipt/semantic/freshness/temporal policy+decision/observation-window/epoch-set/earliest-valid-until exact复制比对`PersistedAllFreshToolTruthAuthorityBindingV1`。adjudication之后发生TTL到期、epoch漂移、semantic orphan或bundle member改变时，即使Campaign receipts不变也禁止transition。Plan C只以compound FK消费B plan/path/member tables，不创建第二套plan或outer aggregation规则。即使plan只有一个objective、且有一个合法Campaign terminal receipt，也仍必须有完整objective/component outcomes、typed receipt lineage、revision adjudication、live AllFresh guard、transition decision和transition receipt。缺任一authority、plan/path/outcome漏重、stale plan、Campaign terminal直接调用或caller自报hash时只能保持`inconclusive/contested`并写residual，不能调用通用Candidate mutation API。

terminal hash依赖必须无环且按固定顺序计算：`adjudication_hash`（含plan/outcome/unresolved/AllFresh persisted binding）→ `transition_decision_hash`（只含predecessor、terminal state、plan/adjudication/AllFresh binding，不含successor/event/receipt）→ successor `revision_id/revision_hash` → creating `state_event_hash`（authority引用transition decision id/hash，不引用最终receipt hash）→ `transition_receipt_hash`（最后绑定decision、successor revision与event）。DB deferred FK最后验证五者identity；event hash或revision id绝不能反向包含transition receipt hash。golden/replay/property tests必须证明同ingredients稳定，任何顺序反转或self-reference constructor不可表达。

### Step 3d：消费 Plan A temporal/bundle authority

Plan B不定义第二个temporal policy/reducer。它直接消费Plan A operation-frozen `EvidenceTemporalValidityPolicyV1` header/member exact set、`TemporalValidityStatus` decisions与`CheckedToolTruthAuthorityBundle<'g>` private views；policy已冻结fact-class positive/negative/refutation TTL、target-state epoch requirement、`max_cross_observation_skew`与time-source contract，并由Plan A保证negative/refutation TTL各自短于positive。Plan A bundle host reducer只接收server receipt的`observed_at/valid_until/target_state_epoch`、DB transaction time、locked current target-state epoch heads exact set与tool-truth reconciliation state，生成canonical temporal/set/bundle hashes；模型/caller不能传“fresh=true”、当前时间、epoch set、TTL或decision hash。byte freshness不替代时间/目标状态判定，时间fresh也不能覆盖orphan tool truth。Plan B只验证并复制这些exact decisions进自己的snapshot census/authority hash；发现Plan A API/schema缺失时blocked，禁止本地重算一个弱化版本。

snapshot authority hash必须domain-separated绑定Plan A multi-root bundle seal/root/member/receipt exact-set hashes、每root denominator graph/semantic/freshness/temporal hashes + temporal policy id/hash + target-state epoch-set/observation-window hashes +完整decision set hash + managed-feed catalog/policy/required-source/member denominator seal + signature-algorithm/trust-store/key-revocation epoch hashes + signed knowledge-feed snapshot/product-version/feed-match census hashes + source set hash + stale residual/revalidation/enrichment obligation set hash。Plan A提供二级opaque authority：`CheckedToolTruthAuthorityBundle<'g>`保留exact全部roots/receipts及`consistent_fresh|semantic_invalid|expired|mixed_epoch|skew_exceeded`bundle member status，同时保留原始`TemporalValidityStatus::{Fresh,Expired,MixedEpoch,SkewExceeded}`与独立semantic reconciliation status；B只exact-copy，禁止把orphan混入temporal enum。只有全部member为`consistent_fresh`时才能形成B `sealed_ready` snapshot，caller不能先过滤stale。`AllFreshToolTruthAuthorityBundle<'g>`是Plan C action/revision verdict与Plan D current report的更强authority，B不得伪造或把自己的snapshot seal当作该runtime token。Gate及DB apply还以server current time/locked current target-state epoch heads exact set重验ready snapshot成员尚未过`valid_until`、未发生epoch-set drift且没有mixed-epoch；并以DB clock/current trust store重验全部expected feed成员仍未过age ceiling、签名算法仍受信、signer key未撤销且required denominator未漂移，生成Gate-time feed reevaluation exact-set hash。任一TTL/epoch/feed-age/key-revocation/expected-member失效都必须blocked并启动新checked bundle/snapshot，不能沿用旧proposal。feed match始终只是signed/versioned knowledge signal，即使current也不升级为proof authority。

### Step 4：实现固定 route order reducer

`ReducerDecision` 必须是 closed enum：

```rust
pub enum ReducerDecision {
    AttachCurrent { root_id: Uuid, revision_id: Uuid },
    CreateInitial { root_id: Uuid },
    ReopenHistorical { root_id: Uuid, predecessor_revision_id: Uuid },
    NoSemanticChange { root_id: Uuid, revision_id: Uuid },
    ExplicitTransitionRequired { historical_root_id: Uuid },
    Split { parent_root_id: Uuid, child_root_ids: Vec<Uuid> },
    Merge { parent_root_ids: Vec<Uuid>, successor_root_id: Uuid },
    Derive { source_root_id: Uuid, successor_root_id: Uuid },
    NarrowSuccessor {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        successor_root_id: Uuid,
        covered_claim_component_set_hash: String,
    },
    RootIdCollision { computed_root_id: Uuid },
}
```

route 顺序严格对应设计 §5.5.1 的 1–5；禁止 embedding、provider prose 或 completion timestamp影响结果。

### Step 5：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate -E 'test(semantic_) | test(root_) | test(reducer_) | test(provider_completion_) | test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(hypothesis_revision_adjudication_) | test(candidate_terminal_state_)')
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(hypothesis_revision_adjudication_) | test(investigation_projection_catalog_) | test(projection_plan_c_route_catalog_) | test(projection_ts_decl_golden_)')
```

Expected：key/root/replay/collision/order、host-owned VerificationContract exact-set、B-owned verification plan/path/member outer aggregation、revision adjudication authority、Plan A temporal/bundle authority adapter的exact-copy/fail-closed、Candidate terminal-state rejection与四类projection catalog/TS declaration round-trip tests全部`PASS`；没有caller构造contract/plan/path/member/temporal decision hash、以单Campaign terminal冒充revision authority或由Candidate写`verified/refuted/invalid`的public seam。TTL ordering/epoch/skew reducer证据归Plan A对应定向tests所有，B不复制；本Task未写generated文件。

### Future Commit

```bash
git add backend/crates/golish-core/src/verification_contract.rs backend/crates/golish-core/src/hypothesis_verification.rs backend/crates/golish-core/src/investigation_projection.rs backend/crates/golish-core/src/lib.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/mod.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/types.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/semantic_key.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_contract_compiler.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_plan_compiler.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/reducer.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/rollout.rs backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs
git commit -m "feat(hypothesis): add deterministic identity reducer"
```

---

## Task 5：实现 Registry/snapshot repo、原子 canonical apply 与whole-batch projector

**文件：**

- 创建：`backend/crates/golish-db/src/repo/{hypothesis_registry.rs,candidate_analysis.rs,hypothesis_legacy_projection.rs}`
- 创建：`backend/crates/golish-db/src/repo/investigation_projection/{mod.rs,types.rs,projector.rs}`
- 修改：`backend/crates/golish-db/src/repo/{mod.rs,capability_execution_receipts.rs}`（后者只暴露host-owned multi-root Checked bundle Candidate callback/typed consumer kind，不放宽、复制或降级Plan A `CheckedToolTruthAuthorityBundle`/`AllFreshToolTruthAuthorityBundle`二级guard）。
- 创建：`backend/crates/golish-agent-kit/src/db_traits/hypothesis_registry.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/mod.rs`
- 创建：`backend/crates/golish-agent-app/src/ai/db_bridge/hypothesis_registry.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- 测试：`backend/crates/golish-db/tests/hypothesis_registry.rs`

### Step 1：写 repository RED

增加 tests：

- snapshot freeze必须由Pg repo内部调用Plan A `with_checked_tool_truth_authority_bundle`，由server从operation/stage handoffs派生TI/EAS/Enum/Vuln等relevant-root census，在同一stable request、同一opaque guard lifetime与同一`REPEATABLE READ` DB transaction中检查全部roots/receipts的byte freshness、temporal validity与org/epoch后写Candidate snapshot/bundle-member/census exact set；snapshot持久字段与`CheckedToolTruthAuthorityBundle<'g>`的bundle seal/root/member/receipt/graph/semantic/freshness/temporal hashes exact-equal。漏一stage root、cross-org root、root-census drift或tamper任一root必须形成`blocked_authority_bundle` snapshot + stale residual/revalidation obligation且不能启动analysis/Gate；caller伪造seal/hash、传stale token、预过滤stale root、复用上一request guard、缓存/clone/serialize guard或先查旧`consistent` row再另开snapshot transaction均不可通过类型或repo边界。all-fresh成功freeze后source row改变/删除不改变本attempt authority或bytes，只能作为later source/FactDelta进入下一generation。
- snapshot同transaction从host-managed local feed store冻结signed CVE/CPE/KEV/vendor-advisory/detection-rule manifest exact set、product-version census及deterministic match census。tests覆盖signature/key/provenance/hash/version/published-at/age drift、CPE range boundary、explicit no-match、unknown/conflicting product version与feed stale/invalid；只有current verified match可产生`knowledge_signal`，且把该ref塞进proof/refutation/VerificationContract evidence必须拒绝。Candidate runner无网络/browser/feed refresh方法，任何真实feed更新留在独立运维/人工授权流程。
- 每个snapshot input由server基于完整source冻结chunk census与immutable redacted body/blob；改变chunk边界/ordinal/hash、漏读一个chunk、把oversize前缀伪装成完整payload或caller提交`read_complete=true`均fail closed。freeze后修改或删除live source，后续page/replay仍必须返回相同frozen bytes/hash/version。
- coverage map census必须exact-equal每input `checklist-member × chunk-partition`，bounded subreview可以并行但不能让单critic声称读完整个大input；随后每input/checklist-member cross-chunk及跨input synthesis census exact闭合，host reducer才可写per-input review。漏/重tuple、partition空洞/重叠、provider context truncation、page receipt存在但subreview未理解完整designated bytes、map/synthesis worker混用、漏cross-input组合均blocked；已有proposal时synthesis仍能发现遗漏的第二/第三hypothesis。`missed_hypothesis`以append-only event关闭attempt N并创建N+1；每attempt H1/H2/disposition/subreview/synthesis/coverage-review exact set独立，旧attempt receipt不能关闭新attempt，response-loss不会创建第二个后继attempt。deterministic sample只能落blocked/degraded residual。
- identical root ingredients replay 返回同 root；相同 UUID 不同 ingredients 返回 `ROOT_ID_COLLISION`。
- predecessor ordinal 不连续、两个 current、跨 org relation、AU ref 填入 proof role 均拒绝。
- generation + revision + host-derived claim-component exact set + VerificationContract exact set + B-owned HypothesisVerificationPlan objective/component/path/member exact set + event + membership +完整outbox batch要么全部commit，要么全部rollback；canonical writer不写materialized entity/legacy projection，也不推进projection head/change。
- response loss重放相同request id不增加revision/outbox batch；deterministic projector消费同一batch exact-once、一次推进完整sequence range。注入任一entity version、timeline change、compatibility version、receipt或head CAS失败时整个projection batch不可见，canonical source仍保留。
- canonical source commit后修改/删除source row，projector仍只从outbox immutable typed body/blob投影相同hash；projector type graph没有live source loader。
- entity version 1伪造predecessor、version N漏/错N-1或predecessor hash drift均拒绝并整batch rollback。
- projector与reader并发时，reader只能看到完整旧head或完整新head；不得看到batch中的部分hypothesis/relations/residuals或head已前进但entity version缺失。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(snapshot_) | test(snapshot_tool_truth_authority_) | test(snapshot_temporal_validity_) | test(candidate_knowledge_feed_) | test(candidate_analysis_attempt_) | test(hypothesis_coverage_subreview_) | test(hypothesis_coverage_synthesis_) | test(hypothesis_coverage_review_) | test(registry_) | test(finalizer_) | test(response_loss_) | test(projection_batch_) | test(projection_source_snapshot_) | test(projection_entity_predecessor_) | test(projection_batch_source_order_) | test(projection_head_isolation_)')
```

Expected：repo API 未定义而编译失败。

### Step 3：实现窄 repository port

`HypothesisRegistryRepository` 不暴露 `PgPool`、SQL row 或任意 JSON write：

```rust
#[async_trait::async_trait]
pub trait HypothesisRegistryRepository: Send + Sync {
    async fn freeze_candidate_snapshot(
        &self,
        request: FreezeCandidateAnalysisSnapshot,
    ) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError>;

    async fn load_snapshot_page(
        &self,
        request: LoadCandidateAnalysisPage,
    ) -> Result<CandidateAnalysisPageView, HypothesisRegistryError>;

    async fn load_snapshot_chunk_page(
        &self,
        request: LoadCandidateInputChunkPage,
    ) -> Result<CandidateInputChunkPageView, HypothesisRegistryError>;

    async fn record_analysis_artifact(
        &self,
        request: RecordCandidateAnalysisArtifact,
    ) -> Result<CandidateAnalysisArtifactReceipt, HypothesisRegistryError>;

    async fn seal_analysis_census(
        &self,
        request: SealCandidateAnalysisCensus,
    ) -> Result<CandidateAnalysisCensusView, HypothesisRegistryError>;

    async fn seal_hypothesis_coverage_subreview_census(
        &self,
        request: SealHypothesisCoverageSubreviewCensus,
    ) -> Result<HypothesisCoverageSubreviewCensusView, HypothesisRegistryError>;

    async fn record_hypothesis_coverage_subreview(
        &self,
        request: RecordHypothesisCoverageSubreview,
    ) -> Result<HypothesisCoverageSubreviewReceipt, HypothesisRegistryError>;

    async fn seal_hypothesis_coverage_synthesis_census(
        &self,
        request: SealHypothesisCoverageSynthesisCensus,
    ) -> Result<HypothesisCoverageSynthesisCensusView, HypothesisRegistryError>;

    async fn record_hypothesis_coverage_synthesis_review(
        &self,
        request: RecordHypothesisCoverageSynthesisReview,
    ) -> Result<HypothesisCoverageSynthesisReceipt, HypothesisRegistryError>;

    async fn reduce_hypothesis_coverage_review(
        &self,
        request: ReduceHypothesisCoverageReview,
    ) -> Result<HypothesisCoverageReviewReceipt, HypothesisRegistryError>;

    async fn load_candidate_gate_material(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateMaterial, HypothesisRegistryError>;

    async fn apply_candidate_gate_pass(
        &self,
        request: ApplyCandidateGatePass,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError>;
}
```

所有 write request 携带 operation/scope/org/snapshot/team plan/work item/worker/lease/attempt epoch 与 expected row versions；app bridge不根据模型字符串补 identity。`FreezeCandidateAnalysisSnapshot`只包含stable consumer request id与server可解析的operation/scope identity，不包含root/receipt list、bundle/set seal/hash、fresh token、`observed_at/valid_until`、target-state epoch set、temporal disposition或decision hash；relevant roots由server从frozen stage handoffs派生。`CandidateAnalysisSnapshotView`返回持久化bundle/member/temporal authority字段与`sealed_ready|blocked_authority_bundle`状态供后续Gate比对。generic `record_analysis_artifact`明确拒绝coverage subreview/synthesis/final-review kinds，后者只能走上述dedicated writers/reducer。

### Step 4：实现 snapshot freeze 与 server page receipts

Pg实现的public `freeze_candidate_snapshot`不得先查`reconciliation_state='consistent'`后自己开启独立transaction，也不得接受caller roots或可复用fresh token。它根据stable request与operation/stage handoffs由server派生relevant-root exact census，再调用Plan A `with_checked_tool_truth_authority_bundle`；该host service按root/receipt canonical order建立sealed snapshots、逐root authority/temporal set并封存bundle exact seal，随后开启一个`REPEATABLE READ` transaction，在同一opaque `CheckedToolTruthAuthorityBundle<'guard>` lifetime内调用module-private `candidate_analysis::freeze_snapshot_on(tx, &checked_bundle, request)`。callback锁 operation、scope seal、predecessor handoffs、current Application Model、previous generation seal、server-derived current target-state epoch heads exact set及authority revisions；它必须先验证bundle root census与server重新派生集合exact-equal、全部root同operation/org，再从bundle绑定的typed facts/bytes读取TI/EAS/Enum/Vuln facts、observations/evidence、technique outcomes，绝不回查mutable workspace path或仅相信旧consistent row。

`freeze_snapshot_on`把Checked bundle暴露的全部roots/receipts及Plan A `EvidenceTemporalValidityPolicyV1`/set/bundle decisions exact复制进snapshot bundle-member/temporal census；B重新校验count/set/member hashes但不能caller-side预过滤。若每个member都是`consistent_fresh`，先加载operation-frozen managed-feed catalog/trust policy与required feed-source/member denominator，再逐expected member读取本地signed manifest；整源缺失也落`unavailable` member，不能按store现有行反推分母。host绑定signature algorithm allowlist、trust-store version与key-revocation epoch，以server product-version census和versioned matcher生成feed-match exact set；current verified matches按`knowledge_signal`加入 `(source_kind,stable_key,source_hash)` source/input排序，stale/signature-invalid/signer-revoked/unavailable feed与unknown/conflicting product version只写residual + feed-refresh/product-enrichment obligation，并把相应coverage checklist member标blocked。若Checked bundle任一`semantic_invalid|expired|mixed_epoch|skew_exceeded`或任一required feed member非current，则在**同一Plan A transaction**写`blocked_authority_bundle` snapshot、完整bundle/temporal/feed metadata census与typed stale residual/revalidation obligation exact set，不写任何analysis input/chunk、不开analysis attempt。header直接复制bundle seal/root/member/receipt/denominator graph/semantic/freshness/temporal hashes，并绑定feed catalog/policy/denominator/trust-store/revocation epoch/product/match/temporal policy/target epoch-set与组合snapshot authority hash。漏root、漏expected feed source/member、cross-org、tamper-before-freeze、bundle/request drift、feed/matcher/temporal census漏重或callback rollback绝不留下可运行的半snapshot。相同stable request + exact bundle/temporal/feed/source payload response-loss replay同一snapshot；同request payload/hash漂移稳定拒绝。

genesis 必须写显式 `previous_generation_absent` source set；non-genesis 少一个 required source set即返回 `CANDIDATE_SNAPSHOT_SOURCE_SET_INCOMPLETE`。

chunker只能在freeze transaction读取完整canonical source并使用versioned deterministic redaction+boundary rule；先计算完整source hash/size，再生成canonical chunks、immutable redacted typed body或content-addressed blob与member-set hash。任何source读取/序列化失败、超过hard chunk/member/byte ceiling或无法无损表示时，必须写`blocked_oversize|blocked_unrepresentable` census、input `blocked` disposition和residual；不能截断、抽样或仅把前N字节交给模型后标`analyzed/informational/not_security_relevant`。`source_empty`是显式零成员sealed census，不等于未读取。`load_snapshot_chunk_page`只能读这些snapshot-owned immutable bodies/blobs，严禁按range回读live canonical source；live source修改、删除或权限变化都不改变既有snapshot bytes/hash。

分页函数必须由server按sealed chunk census生成cursor，并在返回页面同事务写`candidate_analysis_page_receipts`。request/DTO与receipt都绑定`input_id + chunk_ordinal range + chunk_census_hash + source_size + chunking/redaction versions`，receipt另含returned member count/page hash与consumer worker。child response中不接受`read_complete=true`；server receipts只证明指定bytes已交付，不能独自证明模型理解。coverage只有在`checklist-member × chunk-partition` subreview exact census、每tupletyped completion/context attestation及cross-chunk/cross-input synthesis exact census全部闭合后才有read/analysis closure；任一truncation或partial理解都blocked。

### Step 5：实现原子 apply

app 传入 pure Gate 产生的 stable-sorted mutation set、active analysis attempt identity/ordinal/prior-attempt-chain hash与 expected authority seals。DB canonical事务重新`FOR UPDATE`加载snapshot必须为`sealed_ready`、Plan A bundle seal/root/member/receipt/denominator-graph/semantic/freshness/temporal exact hashes、每root bundle member、temporal policy/decision/stale-obligation hashes、managed-feed catalog/policy/required denominator/trust-store/revocation hashes、全部snapshot-owned source-set/generation/chunk seals、active attempt、prior terminal attempt chain、同attempt H1/H2/per-input coverage subreview/synthesis/final-review/checklist/contract/verification-plan seals并比较exact hashes；它不得为重验payload回读live source，也不得把`CheckedToolTruthAuthorityBundle`持久字段伪装成可逃出request的opaque token。它还用DB transaction time和locked current target-state epoch heads exact set重验每个Tool Truth member仍未过`valid_until`、其source/current epoch binding仍一致且epoch-set hash未漂移；同时逐expected feed member重验age、签名算法、current signer key与revocation epoch并生成Gate-time feed reevaluation hash。若分析期间任一authority失效或required feed member缺失，则整个apply拒绝、当前attempt blocked，并通过新checked bundle/snapshot写stale/feed-refresh/revalidation obligation，不能消费旧proposal。它只在operation lock后锁source head分配batch seq，不锁或读取materialized projection head：

1. 应用仅含Candidate合法非终态的root/revision/event/relation；
2. host从typed claim派生并写`HypothesisClaimComponentV1` exact set，compile VerificationContract predicate/control sets，再按每revision写`HypothesisVerificationPlanV1` objective/component/path/member exact set及plan seal；component coverage不足则拒绝原plan或显式创建narrow successor，随后写generation、transition、membership、seal；
3. 写 input dispositions/relations与 readiness；
4. CAS source head分配`source_batch_seq/predecessor_batch_id`，按frozen policy生成一个immutable projection outbox batch header、canonical-sorted完整member exact set及每member的typed source snapshot body或outbox-owned blob；不写任何materialized/legacy compatibility row；
5. 写`plan_c_verification_unavailable` residual（仅两个新权威mode）及其outbox；
6. 写最小stage handoff/final closeout并commit；
7. commit后由唯一deterministic whole-batch projector消费outbox；canonical writer、legacy adapter、event handler和read API都不得直接推进head。

事务内不调用模型、HTTP、MQ、browser、shell或 provider。

### Step 5b：原子物化完整projection batch

`investigation_projection::projector`按`batch_id`claim但不逐row确认，验证header count/hash与member ordinal exact set后，在一个transaction中：锁operation head；分配连续seq range；只从outbox immutable `ProjectionSourceSnapshotV1` body/blob构造所有`ProjectionEntityV1` versions和`InvestigationTimelineEventV1`；验证entity predecessor version/hash；写entity versions/change rows；写batch receipt；最后CAS `change_seq + last_projected_batch_id`。同一batch replay返回原receipt，不新增version/event。projection hash/serialization错误、unknown catalog、sequence collision、predecessor drift或CAS失败整体rollback并保留outbox待重试。

projector claim query必须以`source_batch_seq`为序并验证predecessor receipt；不能用`created_at/projected_at`排序。并发测试固定：insert batch先commit，close batch后commit，但close worker先claim；close worker future必须保持pending直到insert receipt提交，随后两batch产生稳定连续change seq、entity version predecessor和相同deterministic event IDs。清空materialized projection后按source batch seq rebuild，source order/change seq/entity version/event IDs与排除时间字段的canonical manifest hash必须byte-for-byte相同；重建`projected_at`可以不同且不进入任何identity/hash。

read model永远从materialized entity versions按captured head查询，不从canonical tables“补新行”。这条隔离同样适用于后续Plan C/D entity kind；它们只能在source transaction写Plan B outbox batch，不能另写head或best-effort enqueue。

### Step 6：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(snapshot_) | test(snapshot_tool_truth_authority_) | test(snapshot_temporal_validity_) | test(candidate_knowledge_feed_) | test(candidate_analysis_attempt_) | test(hypothesis_coverage_subreview_) | test(hypothesis_coverage_synthesis_) | test(hypothesis_coverage_review_) | test(chunk_census_) | test(chunk_replay_after_source_change_) | test(registry_) | test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(finalizer_) | test(response_loss_) | test(projection_batch_) | test(projection_source_snapshot_) | test(projection_entity_predecessor_) | test(projection_batch_source_order_) | test(projection_rebuild_stability_) | test(projection_head_isolation_)')
```

Expected：全部`PASS`；Plan A Checked bundle在同request/transaction封存server-derived TI/EAS/Enum/Vuln multi-root exact census，漏stage root、cross-org/root-census drift、任一root tamper、expired/orphan member均只能得到完整`blocked_authority_bundle` snapshot+census+stale residual/revalidation obligation而不启动analysis/Gate；stale/cached/caller-forged token/hash/root list不可构造。all-fresh bundle才得到ready snapshot；negative/refutation短TTL与finalize前刚过期正反例闭合；signed knowledge-feed/product-version/match exact census可重放，stale/invalid feed与unknown version落typed obligation，feed signal不能作proof且Candidate不联网刷新；silent truncation与caller read-complete被拒；freeze后修改/删除source仍从snapshot materialization返回逐byte相同body/blob与hash；每input checklist×chunk-partition subreview及cross-chunk/cross-input synthesis exact set闭合后才有host-reduced review，single-proposal遗漏第二hypothesis会重开attempt，sample/context truncation只能blocked/degraded；missed attempt的历史exact sets保留且新attempt不能复用；claim-component exact denominator、VerificationContract与verification plan objective/component/path/member全覆盖seal原子写入，漏component只能拒绝或生成narrow successor而不终结原claim；canonical rollback时canonical/outbox batch均为0且两个head不变；canonical commit后即使live source删除或projector失败truth仍存在，projector从outbox冻结payload得到相同projection；完整projector batch按source seq exact-once物化全部entity/timeline versions并只CAS一次projection head；predecessor伪造整批拒绝，close worker抢先时等待且不能越过insert；rebuild得到相同source order/change seq/version/event IDs/canonical manifest hash而允许不同`projected_at`；任一步注入失败时reader仍只见旧head，response-loss replay不重复batch/version/change。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/hypothesis_registry.rs backend/crates/golish-db/src/repo/candidate_analysis.rs backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs backend/crates/golish-db/src/repo/investigation_projection/mod.rs backend/crates/golish-db/src/repo/investigation_projection/types.rs backend/crates/golish-db/src/repo/investigation_projection/projector.rs backend/crates/golish-db/src/repo/capability_execution_receipts.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-agent-kit/src/db_traits/hypothesis_registry.rs backend/crates/golish-agent-kit/src/db_traits/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/hypothesis_registry.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-db/tests/hypothesis_registry.rs
git commit -m "feat(hypothesis): add atomic registry repositories"
```

---

## Task 6：实现 Candidate Gate exact-set contract

**文件：**

- 创建：`backend/crates/golish-agent-kit/src/harness/hypothesis_registry/candidate_gate.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_contract_compiler.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_plan_compiler.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/hypothesis_registry/mod.rs`
- 修改：`backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs`

### Step 1：写 Gate RED table tests

每个 blocker 独立 fixture，至少覆盖：Plan A Checked bundle exact hashes/root census/org/tamper drift，任一`semantic_invalid|expired|mixed_epoch|skew_exceeded`冒充ready，caller预过滤stale/伪造或跨request复用guard、旧consistent row、temporal decision census漏重及nonfresh evidence混入input；TTL ordering本身只引用Plan A定向policy/schema test。feed fixtures覆盖expected source/member缺失、manifest/signature/provenance/age、trust-store/key-revocation epoch、Gate-time expiry、product/match漏重与knowledge-signal冒充proof。attempt/chunk/page/H1 fixtures覆盖链分叉、跨attempt复用、截断、漏读及page receipt冒充理解。coverage fixtures覆盖checklist×partition漏重、recursive child/dimension/global-root漏项、transitive worker复用、跨input partition与跨attack-class/boundary组合遗漏、sample伪造`adequate`、global miss仍finalize及H2/global-review orphan。semantic/plan fixtures覆盖跨org/identity非法merge、generation transition、gap/AU proof误用、VerificationContract drift、claim component跨revision/derivation substitution、required denominator/path union、optional-only或多component falsifier量词绕过、typed outcome receipt/lineage重放、unresolved set漏项、single Campaign terminal、live AllFresh失效、acyclic transition hash顺序与Candidate伪造`verified/refuted/invalid`。

示例：

```rust
#[test]
fn gate_blocks_gap_as_refutation_and_au_as_proof() {
    let forged = untrusted_controller_mutation_json(
        "refuted",
        vec![RevisionSourceRef::Gap("gap:web-auth".into())],
    );
    assert_eq!(
        CandidateHypothesisMutation::parse_controller_artifact(forged)
            .unwrap_err()
            .code(),
        "HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN",
    );

    let mut snapshot = valid_gate_snapshot();
    snapshot.mutations[0].proof_refs = vec![RevisionSourceRef::ApplicationContext(
        "application-model:item:role-admin".into(),
    )];
    assert_eq!(
        validate_candidate_gate(&snapshot).unwrap_err().code(),
        "HYPOTHESIS_APPLICATION_CONTEXT_IS_NOT_PROOF"
    );
}

#[test]
fn candidate_controller_cannot_write_terminal_or_server_only_states() {
    for forged in ["verified", "refuted"] {
        let artifact = untrusted_controller_mutation_json(forged, vec![]);
        assert_eq!(
            CandidateHypothesisMutation::parse_controller_artifact(artifact)
                .unwrap_err()
                .code(),
            "HYPOTHESIS_CANDIDATE_TERMINAL_STATE_FORBIDDEN",
        );
    }
    let artifact = untrusted_controller_mutation_json("invalid", vec![]);
    assert_eq!(
        CandidateHypothesisMutation::parse_controller_artifact(artifact)
            .unwrap_err()
            .code(),
        "HYPOTHESIS_INVALID_STATE_SERVER_ONLY",
    );
}
```

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate -E 'test(gate_)')
```

Expected：Gate 函数未实现或错误 code不匹配，tests 失败。

### Step 3：实现纯 Gate

`validate_candidate_gate(&CandidateGateSnapshot) -> Result<CandidateGatePass, CandidateGateBlock>`只能读取由repo private constructor形成的frozen material。snapshot必须是`sealed_ready`，绑定Plan A Checked bundle seal/root/member/receipt及每root graph/semantic/freshness/temporal hashes、server temporal reevaluation time/current target-state epoch heads exact-set hash、managed-feed catalog/policy/required denominator/signature algorithm/trust-store/key-revocation epoch hashes、signed feed/product-version/match censuses与Gate-time feed reevaluation exact set、唯一active analysis attempt id/ordinal、同attempt H1/H2/read/per-input coverage subreview/synthesis/final-review/checklist seals及从ordinal 0开始的prior terminal-attempt exact chain/hash；旧attempt material不得进入active exact set，bundle/root/feed/time/epoch-set/trust-store字段不得来自Controller。Controller交付objective intent后，host先从typed claim派生`HypothesisClaimComponentV1` exact set，从sealed predicate/control/policy registries生成`VerificationContractV1`，再从全部claim components/objectives/contracts生成B-owned `HypothesisVerificationPlanV1` objective-component/path/member exact set；Gate snapshot包含这些不可由模型构造的compiled authorities，DB apply时再次从locked authority重算并逐hash比较。任一path未覆盖全部required components时不能seal；若选择narrow successor，Gate必须输出新的窄claim mutation/lineage并保持原宽claim非终态。输出mutation set必须按`(organization_id, semantic_key_hash, operator_rank, proposal_id)`排序并带exact set hashes，且mutation state类型只能是`CandidateMutationEpistemicState`。

Gate先验条件顺序固定：Checked multi-root bundle exact authority + `sealed_ready` + Gate-time temporal epoch/TTL重验 → managed-feed expected denominator exact closure + DB-clock age/signature/current-key/revocation重验 → signed feed/product-version/match census与knowledge-signal非proof边界 → active/prior attempt chain → source/chunk census与无截断证明 → 同attempt server page/read receipts（仅bytes交付证明）→ 同attempt H1 proposal census与每input proposal disposition → checklist×chunk-partition subreview exact set → recursive cross-chunk/cross-input/cross-class/global synthesis tree exact set → 每input host-reduced`hypothesis_coverage_review`、checklist与H1-ref closure → 同attempt H2 census/components → Controller decisions与Candidate非终态限制 → semantic/reducer formulas → host-derived HypothesisClaimComponent exact set → host-owned VerificationContract predicate/control/pair/order exact set → B-owned HypothesisVerificationPlan objective-component/path/member全覆盖与outer aggregation exact set → generation transitions → input dispositions/relations → planning readiness与未来Plan C capability assessment authority严格分离 → final submitter。Plan B Gate不查询或伪造capability assessment；其缺失永远不能删除、refute或拒绝落库hypothesis。

coverage closure规则固定：H1 seal后server冻结每input完整chunk partitions，并构造`checklist-member × chunk-partition` subreview exact census，不论proposal ref count是0、1还是更多。bounded map critics只读designated partition+该input完整H1 refs，worker不得等于primary analyst；每input/checklist-member `cross_chunk` node消费全部partition subreviews，随后bounded `cross_input_partition/cross_input_reduce`归并到每attack-class×trust-boundary exact-one root，再由`cross_dimension_reduce`结合relationship cross-index归并到exact-one org/snapshot `global_semantic_root`。parent worker必须排除全部transitive descendants与primary analysts。host reducer只有在subreview、recursive synthesis tree、global review与H2 exact censuses全部闭合、无context truncation/omission、worker separation有效且feed-dependent checklist无未处置blocked member时才能写`adequate`。page receipts只证明bytes交付，不能取代typed subreview/synthesis closure。`adequate`只表示冻结checklist、signed feed context与完整census内未发现遗漏；它绝不是漏洞不存在、checked-empty或analysis coverage complete。zero-proposal只是H1 ref exact set为空的特例，同样不能推导refutation。`missed_hypothesis`（包括已有proposal时发现第二/第三遗漏或跨dimension组合）使本attempt不可finalize并进入新H1 attempt；`blocked`、stale feed/unknown product version或deterministic sampling必须落degraded residual/obligation，不能宣称coverage complete。到达bounded retry上限仍只能blocked。

`CandidateGatePass` 至少包含：

```rust
pub struct CandidateGatePass {
    pub snapshot_id: Uuid,
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub tool_truth_authority_root_set_hash: String,
    pub tool_truth_authority_bundle_member_set_hash: String,
    pub tool_truth_authority_receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_digest: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub gate_temporal_reevaluation_hash: String,
    pub knowledge_feed_catalog_policy_seal_hash: String,
    pub knowledge_feed_required_member_set_hash: String,
    pub knowledge_feed_signature_algorithm_set_hash: String,
    pub knowledge_feed_trust_store_hash: String,
    pub knowledge_feed_key_revocation_epoch_hash: String,
    pub knowledge_feed_snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub knowledge_feed_match_census_hash: String,
    pub gate_knowledge_feed_reevaluation_hash: String,
    pub stale_revalidation_obligation_set_hash: String,
    pub knowledge_feed_obligation_set_hash: String,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: u32,
    pub prior_terminal_attempt_chain_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub controller_decision_set_hash: String,
    pub mutation_set: Vec<CandidateHypothesisMutation>,
    pub mutation_set_hash: String,
    pub hypothesis_claim_components: Vec<HypothesisClaimComponentV1>,
    pub hypothesis_claim_component_set_hash: String,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_contract_set_hash: String,
    pub hypothesis_verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub hypothesis_verification_plan_set_hash: String,
    pub input_dispositions: Vec<InputProcessingDispositionDecision>,
    pub input_relations: Vec<InputHypothesisRelationDecision>,
    pub input_chunk_census_set_hash: String,
    pub hypothesis_coverage_subreview_census_set_hash: String,
    pub hypothesis_coverage_synthesis_census_set_hash: String,
    pub hypothesis_coverage_global_semantic_root_hash: String,
    pub hypothesis_coverage_global_review_hash: String,
    pub hypothesis_coverage_review_set_hash: String,
    pub hypothesis_coverage_checklist_set_hash: String,
    pub generation_transition_set_hash: String,
    pub final_submitter_worker_run_id: Uuid,
}
```

### Step 4：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate -E 'test(gate_) | test(candidate_authority_bundle_gate_) | test(candidate_temporal_validity_gate_) | test(candidate_knowledge_feed_gate_) | test(candidate_attempt_chain_gate_) | test(chunk_census_gate_) | test(hypothesis_coverage_subreview_gate_) | test(hypothesis_coverage_synthesis_gate_) | test(hypothesis_coverage_gate_) | test(zero_proposal_special_case_gate_) | test(verification_contract_gate_) | test(hypothesis_claim_component_gate_) | test(hypothesis_verification_plan_gate_) | test(paired_control_binding_gate_) | test(candidate_terminal_state_)')
```

Expected：semantic/reducer/Gate tests全部`PASS`；Checked multi-root bundle只有all-fresh/同org/完整root census且Gate-time TTL/epoch仍有效才可pass；signed feed/match census不可漂移、stale/unknown版本有obligation且knowledge signal不能作proof；active/prior attempt chain不能被漏项、分叉或旧receipt绕过；每input checklist×chunk-partition subreview、cross-chunk/cross-input synthesis、完整H1 refs/checklist/final review exact closure无法被空集合、截断、page receipt、同worker自证、single-proposal掩盖第二hypothesis或sample=`adequate`绕过；zero-proposal仅按同一通用规则闭合；claim-component denominator、contract predicate/control/pair/order与verification plan objective-component/path/member全覆盖exact set由host重算，漏组件只能拒绝或narrow successor且原claim非终态；Controller对`verified/refuted/invalid`的三个负例稳定拒绝且不产生mutation。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/hypothesis_registry/candidate_gate.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_contract_compiler.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/verification_plan_compiler.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/mod.rs backend/crates/golish-agent-kit/tests/hypothesis_registry_gate.rs
git commit -m "feat(hypothesis): enforce candidate analysis gate"
```

---

## Task 7：声明 Candidate Team policy 与三个 tool-free agents

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 修改：`resources/harness/stages/attack_candidate/spec.json`
- 创建：`backend/crates/golish-sub-agents/src/defaults/prompts/hypothesis_analysis.rs`
- 修改：`backend/crates/golish-sub-agents/src/defaults/{prompts/mod.rs,builder/mod.rs,builder/registry.rs,tests.rs}`
- 修改：`backend/crates/golish-sub-agents/src/executor/{tool_setup.rs,prompt_assembly.rs,response_parsing.rs,stream_processing.rs,inner.rs}`

### Step 1：写 policy/agent RED

在 `stage_spec.rs` tests 和 `defaults/tests.rs` 断言：Candidate spec 声明controller/analyst/critic；live lane范围恰为2–8且small-input one-lane阈值非零；chunking/redaction、attack-class/trust-boundary checklist、coverage partition/synthesis/sampling与signed knowledge-feed/product matcher contract version均为正且closed，hard source/chunk/subreview ceiling闭合；required Tool Truth root families exact为Plan A TI/EAS/Enum/Vuln enum闭集且`require_checked_tool_truth_temporal_authority=true`，spec不重复TTL/max-skew/policy version；analysis attempt bounded；三个agent都是readonly、仅`submit_result`、无delegatable agents且无network/browser/feed-refresh tool；Controller是唯一final submitter。critic prompt必须覆盖proposal-conflict、coverage subreview、cross-chunk/cross-input synthesis closed schemas，明确检查已有proposal之外的第二/第三attack-class×trust-boundary hypothesis并声明page receipt不是理解证明。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(candidate_analysis_team)')
just space-guard
(cd backend && cargo nextest run -p golish-sub-agents -E 'test(candidate_hypothesis) | test(merge_conflict)')
```

Expected：缺少 `candidate_analysis_team` field/agent definitions，tests 失败。

### Step 3：添加独立 policy

不要复用通用 `StageTeamSchedulerPolicy` 或 Verification policy。`ToolTruthRootFamilyV1`必须由先行Plan A在`golish_pentest_domain::tool_truth`公开并由B直接import（wire仅`ti|eas|enum|vuln`）；B不得重定义字符串root enum。若Plan A尚未冻结该public type及`CheckedToolTruthAuthorityBundle`/temporal guard契约，Task 2/5/7必须PAUSE，不能用B-local mirror继续：

```rust
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
```

在 `StageSpec` 加 `candidate_analysis_team: Option<CandidateAnalysisTeamPolicy>`。验证器要求 `AttackCandidate` 才能出现，`2 <= min <= max <= 8`，Controller等于final submitter且与 analyst/critic不同；root-family列表与Plan A `ToolTruthRootFamilyV1::ALL` exact-equal且无重复，`require_checked_tool_truth_temporal_authority=true`，coverage partition/synthesis ceiling非零，signed-feed必须为true。Team policy不拥有temporal policy version、TTL或max-skew；snapshot只能从operation-frozen Plan A guard复制policy id/version/hash与decisions，避免第二authority漂移。超过subreview ceiling只能按versioned deterministic sampling policy生成完整census中的`sampling_omitted`成员并blocked/degraded，不能缩小census后宣称adequate。

spec 添加：

```json
"candidate_analysis_team": {
  "schema_version": 1,
  "controller_role": "candidate_hypothesis_controller",
  "analyst_role": "candidate_hypothesis_analyst",
  "critic_role": "merge_conflict_critic",
  "final_submitter_role": "candidate_hypothesis_controller",
  "min_live_analysis_lanes": 2,
  "max_live_analysis_lanes": 8,
  "single_lane_input_limit": 12,
  "max_inputs_per_microbatch": 24,
  "chunking_contract_version": 1,
  "redaction_contract_version": 1,
  "attack_class_checklist_contract_version": 1,
  "trust_boundary_checklist_contract_version": 1,
  "coverage_partition_contract_version": 1,
  "coverage_synthesis_contract_version": 1,
  "hypothesis_coverage_sampling_contract_version": 1,
  "require_checked_tool_truth_temporal_authority": true,
  "knowledge_feed_snapshot_contract_version": 1,
  "product_version_match_contract_version": 1,
  "max_knowledge_feed_age_seconds": 86400,
  "require_signed_knowledge_feeds": true,
  "required_tool_truth_root_families": ["ti", "eas", "enum", "vuln"],
  "max_source_bytes_per_input": 1048576,
  "max_chunk_bytes": 16384,
  "max_chunks_per_input": 64,
  "max_chunks_per_coverage_partition": 4,
  "max_coverage_subreview_work_items": 4096,
  "max_synthesis_inputs_per_partition": 32,
  "max_proposals_per_artifact": 16,
  "max_controller_page_size": 64,
  "max_attempts_per_work_item": 2,
  "max_analysis_attempts": 2,
  "require_read_only_children": true,
  "require_tool_free_children": true
}
```

保留旧 `specialist: "analyst"` 给前三个 legacy-authority mode；新 runtime只在 frozen mode选择后读取新 policy。

### Step 4：添加静态 agents 与 allowlists

三个 prompt 明确：closed frozen input、目标内容 `instruction_authority=false`、不得发现/扫描/联网/浏览/feed-refresh/delegate、不得发明 identity/hash、只提交 host schema。signed CVE/CPE/KEV/advisory/rule feed match明确标成`knowledge_signal`：可建议hypothesis，不能声称proof/refutation或绕过产品版本unknown/feed stale residual。

critic由host以closed mode运行：`proposal_conflict_review.v1`、`hypothesis_coverage_subreview.v1`、`hypothesis_coverage_synthesis.v1`。subreview输入只含一个server-issued `(input,checklist-member,chunk-partition)` 的designated immutable chunks、该input全部H1 refs、checklist/feed applicability refs；critic不得声称看过其他partition或改变集合。synthesis node kind闭集为`cross_chunk|cross_input_partition|cross_input_reduce|cross_dimension_reduce|global_semantic_root`；每个node只消费server-sealed child exact set、level/partition/covered-input+checklist/relationship-index refs及transitive descendant-worker set，并寻找组合链/第二第三hypothesis，parent worker不得出现在descendant或primary set。任何node不得用page receipt冒充理解，也不能直接写最终per-input/global coverage review（由host reducer写）。zero-proposal只是一种空H1-ref输入。prompt不得把`adequate`描述为完整安全覆盖，任何context truncation、omission或deterministic sample只允许返回`blocked`/degraded。

builder definitions 使用：

```rust
SubAgentDefinition::new(
    "candidate_hypothesis_analyst",
    "Candidate Hypothesis Analyst",
    "Read-only analyst over one server-frozen Candidate microbatch.",
    build_candidate_hypothesis_analyst_prompt(),
)
.with_tools(vec!["submit_result".into()])
.with_readonly(true)
.with_max_iterations(8)
.with_idle_timeout(180)
```

Controller 与 critic同样只有 `submit_result`。在五个 executor boundary 文件加入 closed-role match，确保 workspace skills、MCP、自定义工具、shell/browser/network、普通 subagent delegation都不可见。

### Step 5：运行 GREEN 与 JSON check

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(candidate_analysis_team)')
just space-guard
(cd backend && cargo nextest run -p golish-sub-agents -E 'test(candidate_hypothesis) | test(merge_conflict)')
jq empty resources/harness/stages/attack_candidate/spec.json
```

Expected：tests `PASS`，`jq` exit 0；agent tool set 精确等于 `["submit_result"]`且无browse/feed-refresh；multi-root使用Plan A enum exact set、Checked temporal authority为required且spec没有第二套TTL/max-skew，signed-feed/matcher、attack-class/trust-boundary、partition/synthesis/sampling versions与ceilings固定；critic schemas拒绝漏tuple/checklist/subreview/synthesis member、context截断、sample=`adequate`和coverage-complete prose字段，feed signal proof字段也被拒绝。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/stage_spec.rs resources/harness/stages/attack_candidate/spec.json backend/crates/golish-sub-agents/src/defaults/prompts/hypothesis_analysis.rs backend/crates/golish-sub-agents/src/defaults/prompts/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/tests.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-sub-agents/src/executor/prompt_assembly.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-sub-agents/src/executor/stream_processing.rs backend/crates/golish-sub-agents/src/executor/inner.rs
git commit -m "feat(candidate): declare read-only analysis team"
```

---

## Task 8：实现 typed runner、两波 runtime 与 2–8 live lanes

**文件：**

- 创建：`backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs`
- 修改：`backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`
- 创建：`backend/crates/golish-agent-app/src/ai/{candidate_analysis_projection.rs,candidate_analysis_runtime.rs}`
- 创建：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_analysis_agent_runner.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{mod.rs,stage_team_scheduler.rs}`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- 创建：`backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs`

### Step 1：写 runtime RED

使用 fake runner/repository，覆盖：

- 12个或更少 input只启动1 lane；13+ input启动至少2 lane；任何规模 peak active不超过8。
- 300 inputs生成多于8个 work item并滚动完成，不把8当 lifetime item cap。
- server-derived TI/EAS/Enum/Vuln multi-root Checked bundle只要漏root、cross-org、temporal non-fresh、semantic orphan/invalid或tamper任一root，就原子返回`blocked_authority_bundle` snapshot+census/residual且runner调用数为0；caller不能传root/feed list或guard token。all-fresh ready snapshot在analysis期间TTL/target-state epoch exact set失效时final Gate前blocked并启动新snapshot，不能沿用旧attempt。
- signed CVE/CPE/KEV/advisory/rule feed match作为typed readonly input投影；current+known-version match可触发proposal但不能进入proof refs，stale feed/unknown version只产生residual/obligation且runner没有network/browser/feed-refresh调用。
- primary microbatch ownership互斥且覆盖全部server chunk census；relationship/trust-boundary cross-index可重复引用但不能改变primary owner或省略chunk。
- analyst完成后才seal H1；H1 seal后server逐input派生proposal disposition，并为**每个input**冻结checklist×chunk-partition subreview exact census。bounded map work items滚动运行且critic不同于primary analyst；随后每input/checklist-member cross-chunk及server-partitioned cross-input synthesis exact census滚动运行，最后host reducer写per-input review。proposal conflict components与全部subreview/synthesis/input-review closure完成后才seal H2；critic不得创建/删除proposal。
- 大input不能由单critic假装读完：test让一个input跨16 partitions并断言16×checklist tuple exact covered、peak workers≤8；漏partition、provider context truncation、只留page receipt、漏cross-input synthesis均blocked。超`max_coverage_subreview_work_items`时census仍保留完整tuple并标`sampling_omitted`，最终只能degraded residual。
- 任一input review（包括已有一个proposal）返回`missed_hypothesis`会以append-only attempt event使当前attempt失效并重开H1，不得由Controller直接补proposal；新attempt不能复用前一attempt的proposal/disposition/subreview/synthesis/review/census receipt，prior attempt chain必须连续且hash闭合。zero-proposal只是H1-ref空集特例，不产生refuted/checked-empty；同worker自证、缺一个input closure或bounded retry耗尽后仍强行finalize均拒绝。
- Controller分页读取全部cluster并且唯一 final submitter。
- blocked/failed child形成 `blocked` input disposition/residual，不生成 checked-empty/refutation。
- provider response loss重放同 worker/artifact receipt，不第二次调用 provider。
- snapshot freeze后修改/删除live source，runner重试仍只读取immutable chunk materialization并得到逐byte相同payload/source hash；绝不按range回源。
- final host compiler从typed claim派生claim-component denominator并生成objective-component/full-path plan；Controller漏objective时runtime不得seal原宽claim plan，只能返回blocked或显式narrow successor且原claim保持非终态。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test candidate_analysis_runtime)
```

Expected：runtime/trait不存在，test编译失败。

### Step 3：定义 typed schemas 与 runtime port

`hypothesis_analysis.rs` 定义：

```rust
#[async_trait::async_trait]
pub trait HypothesisAnalysisAgentRunner: Send + Sync {
    async fn run_controller_dispatch(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerDispatchInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>>;

    async fn run_analyst(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateAnalystInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>>;

    async fn run_critic(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateCriticInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>>;

    async fn run_controller_final(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateControllerFinalInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>>;
}

#[async_trait::async_trait]
pub trait HypothesisAnalysisStageRuntime: Send + Sync {
    async fn run(
        &self,
        request: HypothesisAnalysisStageRequest,
        runner: &dyn HypothesisAnalysisAgentRunner,
    ) -> anyhow::Result<HypothesisAnalysisStageOutcome>;
}
```

`CandidateCriticInput`/`HypothesisCriticArtifact`是deny-unknown closed enums，mode exact为`proposal_conflict|coverage_subreview|coverage_cross_chunk_synthesis|coverage_cross_input_partition|coverage_cross_input_reduce|coverage_cross_dimension_reduce|coverage_global_semantic_root`；每variant绑定server-issued census node identity/level/hash与child exact set，不能用一个“generic review”绕过tuple/partition/read authority。subreview artifact只返回local typed miss/block；synthesis artifact只消费sealed child refs且携带descendant-worker exact set；最终per-input/global coverage review没有agent runner variant，只能由host reducer生成。

proposal schema包含 structured claim、preconditions、impact、support/contradiction/AU/knowledge-signal/gap refs、readiness建议；不包含caller-provided semantic hash/root/revision。`knowledge_signal`有独立variant并绑定feed snapshot/match member hash，parser拒绝把它放进proof/refutation/VerificationContract source slots。

### Step 4：安全投影 snapshot pages

`candidate_analysis_projection.rs` 把 DB inputs映射为：

```rust
pub struct UntrustedCandidateInputChunkEnvelope {
    pub input_id: Uuid,
    pub input_key: String,
    pub input_kind: CandidateInputKind,
    pub provenance: CandidateInputProvenance,
    pub at_time_subject: AtTimeSubjectIdentity,
    pub source_hash: String,
    pub source_size: u64,
    pub chunk_ordinal: u32,
    pub chunk_census_hash: String,
    pub chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub bounded_payload: CandidateRedactedChunkBodyV1,
    pub bounded_payload_hash: String,
    pub instruction_authority: bool,
}
```

`CandidateRedactedChunkBodyV1`是server-owned tagged enum，只允许各`CandidateInputKind`对应的redacted typed fields或content-addressed blob reference，拒绝caller JSON、任意路径与live-source locator。`KnowledgeFeedMatchV1`专属variant必须携带feed kind/version/published-at/content+manifest hash/provenance/signature receipt/product-version/matcher/member hash并硬编码`source_authority=knowledge_signal_only`。构造器只接受`sealed_ready` snapshot-owned immutable chunk body/blob与current feed-match member，强制`instruction_authority=false`、字符串长度上限、typed provenance，并重验`input_id/chunk_ordinal/census hash/source size/chunking+redaction version/body hash`；banner/page/AU/feed文本不能拼入system prompt或tool schema。它没有live source repo/loader/feed updater handle，compile-time dependency test必须证明projection/runner无法回读可变canonical source或联网刷新feed。

### Step 5：实现 phase machine

`PgHypothesisAnalysisStageRuntime` 顺序：server派生multi-root census并在Plan A Checked bundle callback内freeze authority/temporal/managed-feed-denominator/product-match snapshot → 若`blocked_authority_bundle`则persist residual/obligation并停止（runner=0）→ ready snapshot immutable chunk census → server open analysis attempt 0 → Controller dispatch plan → server clamp → deterministic all-input/all-chunk microbatches/cross-index → rolling analyst semaphore → attempt-scoped artifact receipts → attempt-scoped H1 census seal → server逐input proposal disposition → server conflict graph/components + 每input checklist×chunk-partition subreview census/work items → rolling map-critic semaphore → per-input/member cross-chunk nodes → attack-class×boundary bounded cross-input partitions → zero/more cross-input reduction levels → zero/more cross-dimension reduction levels + relationship cross-index → exact-one org/snapshot global semantic root → host reduceglobal review与每input coverage review → seal包含conflict/subreview/full synthesis tree/input-review/global-review的H2 exact census → 若有`missed_hypothesis`则append-only close当前attempt并以CAS创建唯一bounded后继attempt，从Controller dispatch/H1重新开始；否则Controller读取active-attempt canonical cluster pages → host derive claim components → compile VerificationContract + full-component HypothesisVerificationPlan → final typed decision/Gate。Gate前server用DB clock/current epoch heads/current feed trust store同时重验Tool Truth TTL/epoch set与required feed age/signature/key-revocation/denominator exact set；任一失效都block attempt并新建checked bundle/snapshot。response-loss用`predecessor attempt + next ordinal + retry request id`返回同一后继attempt，不能分叉。

lane计算：

```rust
fn live_lane_limit(input_count: usize, policy: &CandidateAnalysisTeamPolicy) -> usize {
    if input_count <= policy.single_lane_input_limit as usize {
        return 1;
    }
    let microbatches = input_count.div_ceil(policy.max_inputs_per_microbatch as usize);
    microbatches.clamp(
        policy.min_live_analysis_lanes as usize,
        policy.max_live_analysis_lanes as usize,
    )
}
```

通用 `stage_worker_outputs` 只写：

```json
{
  "schema": "candidate_analysis_artifact_receipt.v1",
  "artifact_id": "server-issued-uuid",
  "artifact_hash": "sha256:server-computed"
}
```

`business_disposition` 为 `artifact_recorded`；安全语义只在 dedicated artifact/proposal/relation tables。

### Step 6：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test candidate_analysis_runtime -E 'test(candidate_analysis_lane_) | test(candidate_authority_bundle_) | test(candidate_temporal_recheck_) | test(candidate_knowledge_feed_) | test(candidate_analysis_attempt_retry_) | test(candidate_chunk_exact_) | test(candidate_chunk_source_replay_) | test(candidate_hypothesis_coverage_subreview_) | test(candidate_hypothesis_coverage_synthesis_) | test(candidate_hypothesis_coverage_review_) | test(candidate_zero_proposal_special_case_) | test(candidate_claim_component_plan_) | test(candidate_response_loss_)')
```

Expected：全部`PASS`；300-input test断言`work_item_count > 8`且`peak_live_workers == 8`；multi-root非all-fresh时runner=0，ready snapshot的temporal/feed hashes不可漂移；所有input/chunk以及checklist×partition subreview、cross-chunk/cross-input synthesis exact covered，单大input不会塞给一个critic；freeze后source change/delete不改变payload；每input（zero-proposal仅空H1特例）由不同map/synthesis workers闭合，missed hypothesis以连续append-only attempt chain重开且旧receipt不能跨attempt复用；feed signal不作proof，claim-component漏覆盖不能seal宽claim plan；任何partial/context-truncated/sampled/blocked路径均不产生terminal epistemic state或analysis-coverage-complete声明。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/hypothesis_analysis.rs backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs backend/crates/golish-agent-app/src/ai/candidate_analysis_projection.rs backend/crates/golish-agent-app/src/ai/candidate_analysis_runtime.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_analysis_agent_runner.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs
git commit -m "feat(candidate): run two-wave hypothesis analysis"
```

---

## Task 9：接入 stage_run、bridge 和原子 Candidate finalizer

**文件：**

- 创建：`backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/{mod.rs,commands/bridge_config.rs,tracking_bridge/chain.rs}`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,tool_execution/direct/stage_run_call.rs}`
- 修改：`backend/crates/golish-agent-runtime/src/{test_utils/context.rs,eval_support/single_turn.rs,eval_support/multi_turn.rs}`
- 修改：`backend/crates/golish-agent-bridge/src/{agent_bridge/mod.rs,agent_bridge/config.rs,agent_bridge/prepare.rs,agent_bridge/constructors/mod.rs,bridge_executor/trait_impl.rs}`
- 测试：`backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs`、`stage_run_call.rs` 内部 tests

### Step 1：写 policy-driven dispatcher/finalizer RED

table test用五种mode生成唯一`mode.policy()`，验证`canonical_writer=Legacy`走旧Candidate path、`Registry`走专用runtime；dispatcher自身不得再match五态。另测deployment default与operation mode不一致时仍按operation。finalizer test注入Checked bundle root/member/receipt/temporal、Gate-time TTL/epoch、signed feed/match、active-attempt-chain、chunk、coverage subreview/synthesis/final-review、claim-component、VerificationContract与verification-plan path drift，断言没有半个revision/generation/outbox/handoff；再绕过typed DTO向Controller artifact注入`verified/refuted/invalid`字符串，response parser先拒绝。test-only DB bypass随后分别direct INSERT forged revision+candidate event、只伪造Campaign terminal origin/receipt、伪造`hypothesis_revision_adjudication`但漏B-owned plan/objective+claim-component outcome exact set或transition receipt、伪造server-validator origin但无typed receipt、漏/重creating event；deferrable authority trigger必须在commit拒绝且不写revision/event/outbox。单个合法Campaign terminal也永远不是revision终态authority。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -E 'test(candidate_analysis_dispatch)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test candidate_analysis_runtime -E 'test(finalizer_)')
```

Expected：context没有 runtime field、dispatcher仍总走legacy，tests失败。

### Step 3：沿 AU seam 接线

在 `AgenticLoopContext`、`BridgeServices`、constructor、prepare、config分别加入：

```rust
pub hypothesis_analysis_runtime:
    Option<Arc<dyn golish_agent_kit::task_orchestrator::HypothesisAnalysisStageRuntime>>,
```

test/eval context显式设 `None`；app `configure_bridge` 注入 `PgHypothesisAnalysisStageRuntime`。telemetry将三个agent映射为Candidate analysis child chain，不映射成scanner/Verification角色。

### Step 4：policy-gated dispatcher

`stage_run_call.rs` 在任何 Candidate manifest seed/provider dispatch前加载 operation冻结pair：

```rust
match persisted_contract.investigation_mode().policy().canonical_writer {
    InvestigationAuthority::Legacy => execute_legacy_candidate_stage_run(...).await,
    InvestigationAuthority::Registry => {
        execute_hypothesis_analysis_stage_run(ctx, model, context, tool_id).await
    },
}
```

`persisted_contract`必须由Plan B `operation_rollout` repo按operation重新加载并验证joint pair，不能接收request中的mode/policy。不能把Candidate加入`stage_team_scheduler_admits_stage`后再复用扫描objective；专用runtime只复用Stage Team lease/work item tables。

### Step 5：原子 Gate closeout

`candidate_analysis_gate.rs`仅从repo private constructor加载`CandidateGateMaterial`：必须包含ready Checked bundle exact fields、server Gate-time temporal/epoch reevaluation、signed feed/match authority、coverage subreview/synthesis/final-review seals。它调用core claim-component/VerificationContract/verification-plan compilers与pure Gate，再调用DB CAS apply。每org Controller是唯一final submitter；operation coordinator只计数sealed org units，不读取跨org proposal内容。Candidate finalizer只能调用non-terminal apply；代码依赖图中不得出现Plan C `HypothesisRevisionAdjudicationAuthorityV1`、terminal transition、Campaign或oracle handle。

新权威mode完成后：

- 写 generation seal；
- 每revision写host-derived claim-component denominator；每个ready objective写host-owned VerificationContract及predicate/control/binding/component-subset exact set，再写full-claim `HypothesisVerificationPlanV1` objective/path/falsifier-member exact set；漏component时plan拒绝或显式narrow successor，原宽claim保持非终态；
- 写 `plan_c_verification_unavailable` residual；
- server-authored minimal deliverable/handoff明确 `verification_not_started`；
- Candidate Stage可转 Reporting；
- 不创建 CandidateAttempt、Campaign、Prepared Action、approval、oracle或Finding。

### Step 6：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -E 'test(candidate_analysis_dispatch) | test(candidate_analysis_stage_run)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test candidate_analysis_runtime -E 'test(finalizer_) | test(candidate_authority_bundle_) | test(candidate_temporal_recheck_) | test(candidate_knowledge_feed_) | test(candidate_coverage_authority_) | test(candidate_claim_component_plan_) | test(candidate_terminal_state_) | test(candidate_contract_authority_)')
```

Expected：五态routing、multi-root/temporal/feed/attempt/chunk/coverage/claim-component/contract/plan drift rollback、multi-org isolation、Plan-C residual tests全绿；Controller terminal/server-only state伪造在parser和DB deferrable authority-trigger两层均被拒，Campaign terminal直接写与缺plan/outcome/transition的revision adjudication也被拒；Candidate path没有任何`verified/refuted`写入口或Plan C authority handle。

### Future Commit

```bash
git add backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs backend/crates/golish-agent-app/src/ai/mod.rs backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/test_utils/context.rs backend/crates/golish-agent-runtime/src/eval_support/single_turn.rs backend/crates/golish-agent-runtime/src/eval_support/multi_turn.rs backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs backend/crates/golish-agent-bridge/src/agent_bridge/config.rs backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs
git commit -m "feat(candidate): route registry-authoritative stage runs"
```

---

## Task 10：实现 legacy projection、shadow compare 与 mutation guards

**文件：**

- 创建：`backend/crates/golish-core/src/investigation_comparison.rs`
- 修改：`backend/crates/golish-core/src/lib.rs`
- 创建：`backend/crates/golish-db/src/repo/investigation_projection/comparison.rs`
- 修改：`backend/crates/golish-db/src/repo/investigation_projection/{mod.rs,projector.rs,types.rs}`
- 修改：`backend/crates/golish-db/src/repo/mod.rs`
- 修改：`backend/crates/golish-db/src/repo/{attack_candidates.rs,attack_candidate_approvals.rs,candidate_attempts.rs,hypothesis_legacy_projection.rs}`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 测试：`backend/crates/golish-db/tests/hypothesis_registry.rs`

### Step 1：写 exact mode guard/projection RED

覆盖五态old acceptance/review/resume权限矩阵；shadow outbox batch response-loss replay；dual whole-record mismatch不得field fallback；comparison record必须包含bundle/temporal/feed、coverage review+synthesis/subreview set、sampling-degraded residual、claim-component/plan以及Plan C typed-not-available字段；registry canonical commit成功后，即使compatibility projector出现deterministic unsupported/derivation failure或transient transaction failure，canonical仍保留且旧consumer HOLD；Candidate与Attempt projection各自append-only version、batch/change seq/source/projected time正确；只有Campaign terminal而没有B plan+objective/component outcomes+revision adjudication+transition receipt时Attempt terminal projection必须unsupported；AU/gap/knowledge-signal/no-adapter hypothesis投影为typed unsupported residual，不删除canonical root；projector不得写旧Candidate/Attempt mutation tables。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(legacy_) | test(shadow_) | test(dual_read_) | test(projection_)')
```

Expected：现有legacy repos未检查investigation mode，禁止态测试失败。

### Step 3：DB defense-in-depth guard

`accept_gate_passed_candidate_batch_with_connection`、approval create/close、resume/attempt release都必须锁 `operation_state` 并调用：

```rust
fn require_legacy_candidate_mutation(mode: InvestigationRolloutMode) -> Result<(), AttackError> {
    if mode.policy().allow_legacy_mutation {
        Ok(())
    } else {
        Err(attack_conflict_code(
            "ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT",
        ))
    }
}
```

command层同样预检以给稳定code，但DB guard才是authority。

### Step 4：outbox、异步compatibility projection与compare规则

- 所有mode共享Plan B唯一whole-batch projector作为`investigation_projection_entity_versions/changes/heads`的唯一writer。source canonical/legacy transaction只插入immutable outbox header/member及outbox-owned source blob；projector按完整batch exact-once消费，在一个短transaction内写全部entity/compatibility/timeline versions、batch receipt并一次推进head。禁止source writer direct materialization/head bump与projector双计sequence。
- `shadow_registry` / `dual_read_compare`：legacy Candidate/Attempt terminal source事务插入不可变outbox batch；commit后/恢复时deterministic projector消费。source row变化不影响已冻结outbox source hash/body；无法形成complete record时通过唯一`compare_and_record_v1`记录`incomplete`，不阻断旧operation。
- `dual_read_compare`：比较complete legacy record hash与complete Registry projection hash；任一字段缺失即`incomplete`，禁止混合读取。
- `registry_authoritative_legacy_projection`：Registry/Campaign source finalizer只在canonical事务写outbox header/member及outbox-owned immutable source blob。projector异步派生Candidate和Attempt两种独立compatibility entity version；只有old-classifier-compatible、old-work-item-backed revision且B-owned plan/claim-component authority完整时可成为Candidate/NoCandidate ready version。Attempt还必须由未来Plan C canonical Campaign/action facts派生；若映射terminal disposition，则必须有current B-owned `HypothesisVerificationPlanV1`、objective+claim-component outcome exact set、`HypothesisRevisionAdjudication`与revision transition receipt完整authority。Campaign terminal/oracle receipts只是objective evidence members，单独存在不能派生terminal Attempt。Plan C未部署或字段不完整时写typed `unsupported/not_available_plan_c` version并让旧consumer fail closed，绝不伪造Attempt、oracle、adjudication或transition receipt。
- `new_only`：不新增legacy compatibility versions，但现有历史 projection 继续以 read-only `HistoricalReadOnly` 打开；不能把“停止新写入”误实现成“历史不可读”。
- capability adapter不存在不影响Registry；投影写`unsupported` residual。

compatibility version必须带`projection_authority=derived_compatibility`、source generation/revision/claim-component/verification-plan identity与record hash，以及按适用性带revision adjudication/objective+component outcome set/transition receipt identity；Campaign terminal/oracle仅作为objective evidence member refs，不能占据revision-authority字段。还必须带`projection_schema_version=1`、entity version、batch/change seq、`source_occurred_at/source_time_status/projected_at`与`read_only=true`；禁止伪造Campaign/oracle/adjudication/transition receipt。deterministic unsupported/diverged/derivation-failed是可物化的typed invalidation version，使同batch其余canonical projections仍可原子发布；数据库/serialization/CAS等transient failure则整batch rollback且head不动。两种失败都发生在canonical commit之后，因此都不能回滚或删除canonical truth。

唯一 `comparison_record.v1` 在 `golish-core::investigation_comparison` 冻结 semantic identity、Checked multi-root bundle/temporal/feed authority hashes、generation seal、hypothesis disposition/readiness、claim-component + VerificationContract + verification-plan objective/path exact sets、coverage subreview/synthesis/final-review/checklist set hashes、`candidate_hypothesis_coverage_sampling_degraded` residual membership、future capability assessment、revision adjudication/objective+component outcomes/transition、Campaign/oracle evidence members、Finding/refutation lineage及其余residual/coverage membership。Plan C 尚未存在的capability/adjudication/Campaign字段使用typed `not_available_plan_c`，不能省略/null后在Plan D改V1 hash。canonical serializer排除timestamp/row id/lease/prose，sorted exact set后生成SHA-256；`comparison.rs::compare_and_record_v1`是唯一sample writer。

### Step 5：运行 GREEN

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(legacy_) | test(shadow_) | test(dual_read_) | test(comparison_record_) | test(legacy_candidate_projection_) | test(legacy_attempt_projection_) | test(legacy_terminal_authority_projection_) | test(projection_batch_) | test(projection_failure_preserves_canonical_)')
```

Expected：全部`PASS`；matrix test精确验证canonical writer、Gate authority、legacy mutation、registry shadow、Campaign policy、JIT、compare policy与legacy projection八个policy字段；source transaction只写outbox header/member/owned blob；Candidate/Attempt compatibility仅由projector异步生成；typed invalidation或projector rollback都不改变canonical；旧consumer在missing/unsupported/diverged/stale version时稳定HOLD。

### Future Commit

```bash
git add backend/crates/golish-core/src/investigation_comparison.rs backend/crates/golish-core/src/lib.rs backend/crates/golish-db/src/repo/investigation_projection/mod.rs backend/crates/golish-db/src/repo/investigation_projection/projector.rs backend/crates/golish-db/src/repo/investigation_projection/types.rs backend/crates/golish-db/src/repo/investigation_projection/comparison.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/attack_candidates.rs backend/crates/golish-db/src/repo/attack_candidate_approvals.rs backend/crates/golish-db/src/repo/candidate_attempts.rs backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-db/tests/hypothesis_registry.rs
git commit -m "feat(investigation): project registry to legacy safely"
```

---

## Task 11：实现最小 investigation read model 与三个 IPC commands

**PAUSE B：没有 generated IPC/type-chain 明确授权时，不导出 ts-rs，不生成或修改 `frontend/lib/generated/`；四个core catalog enum bindings `ProjectionEntityKind.ts / ProjectionInvalidationReason.ts / TimelineEventKind.ts / ProjectionSourceTimeStatusV1.ts`同样在暂停范围内。**

**文件：**

- 修改：`backend/crates/golish-core/src/investigation_projection.rs`
- 修改：`backend/crates/golish-db/src/repo/investigation_projection/mod.rs`
- 修改：`backend/crates/golish-db/src/repo/investigation_projection/types.rs`
- 创建：`backend/crates/golish-db/src/repo/investigation_projection/{summary.rs,hypotheses.rs,legacy.rs,timeline.rs}`
- 修改：`backend/crates/golish-db/src/repo/mod.rs`
- 创建：`backend/crates/golish-agent-app/src/ai/commands/investigation/{mod.rs,dto.rs,cursor.rs}`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/mod.rs`
- 修改：`backend/Cargo.toml`
- 修改：`backend/crates/golish-agent-app/Cargo.toml`
- 创建：`backend/crates/golish/src/commands_facade/investigation.rs`
- 修改：`backend/crates/golish/src/commands_facade/mod.rs`
- 修改：`backend/crates/golish/src/commands_registry.rs`
- 创建：`backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs`
- 测试：`backend/crates/golish-db/tests/hypothesis_registry.rs`
- 生成：`frontend/lib/generated/InvestigationScopeRequest.ts`
- 生成：`frontend/lib/generated/InvestigationHypothesisListRequest.ts`
- 生成：`frontend/lib/generated/InvestigationHypothesisGetRequest.ts`
- 生成：`frontend/lib/generated/InvestigationSummaryView.ts`
- 生成：`frontend/lib/generated/InvestigationHypothesisListView.ts`
- 生成：`frontend/lib/generated/InvestigationHypothesisListItemView.ts`
- 生成：`frontend/lib/generated/InvestigationHypothesisDetailView.ts`
- 生成：`frontend/lib/generated/InvestigationTemporalSnapshotView.ts`
- 生成：`frontend/lib/generated/InvestigationProjectionEnvelope.ts`
- 生成：`frontend/lib/generated/InvestigationModePolicyView.ts`
- 生成：`frontend/lib/generated/InvestigationCommandError.ts`
- 生成：`frontend/lib/generated/ProjectionEntityKind.ts`
- 生成：`frontend/lib/generated/ProjectionInvalidationReason.ts`
- 生成：`frontend/lib/generated/TimelineEventKind.ts`
- 生成：`frontend/lib/generated/ProjectionSourceTimeStatusV1.ts`

### Step 1：写 authorization/cursor RED

按 Reporting auth fixture写 tests：trusted local desktop可读；wrong channel/provider failure/foreign project/unsealed scope/cross-org selector统一`INVESTIGATION_FORBIDDEN`；malformed operation/revision/organization ID返回`INVESTIGATION_INVALID_ID`；unknown filter value或互斥filter组合返回`INVESTIGATION_INVALID_ARGUMENT`；deleted live target仍返回at-time identity；V2 cursor签名/tamper/resource/filter/operation mismatch及任一`as_of_temporal_cutoff/authority_epoch_set_hash/earliest_effective_valid_until`篡改返回`INVESTIGATION_CURSOR_INVALID`。签名有效但`as_of_change_seq`落后、authority epoch exact set漂移、或DB transaction time已越过`earliest_effective_valid_until`都返回`INVESTIGATION_PROJECTION_STALE`且`restart_required=true`，即使projection head完全未变也绝不能返回另一页。再覆盖V2 canonical round-trip、caller时间被忽略、第一页和后续页envelope/cursor temporal fields exact-equal；V1只允许historical/legacy decoder读取，current Registry multipage收到合法V1也必须要求restart，不能签发或续写V1 next cursor。

DB read-model RED另覆盖：所有summary/list/detail只读materialized entity versions；projector在batch commit前暂停时reader仍看到完整旧head，commit后一次看到全部新entity；同一个`REPEATABLE READ READ ONLY` snapshot必须同时捕获projection head、DB clock temporal cutoff、current authority epoch exact-set hash与本次结果依赖authority members的最早effective `valid_until`，不能在事务外补读或信任caller clock。Timeline event kind由typed catalog映射，不把`entity_kind/change_kind`字符串交给UI猜；invalidation必须带typed reason；`source_occurred_at`与`projected_at`分别保留且排序只用`change_seq,event_id`；historical unknown source time保持null+explicit status；unknown kind/payload fail closed。core `projection_ts_decl_golden_`与app `investigation_temporal_snapshot_ts_golden_`/export test还要证明四个generated TS union exact-equal Rust`ALL/as_str` wire values，且`InvestigationTemporalSnapshotView.ts`与`InvestigationProjectionEnvelope.ts` exact包含V2 temporal字段、foreign/mirror enum或自由字符串DTO无法编译/序列化。

### Step 2：运行 RED

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_timeline_) | test(investigation_projection_catalog_) | test(projection_ts_decl_golden_)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization)
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(projection_read_head_isolation_) | test(timeline_typed_semantics_) | test(projection_dual_time_)')
```

Expected：commands/module或materialized-only Timeline/read-model实现不存在，相关app/DB tests失败；不能只跑IPC RED而漏掉本Step声明的DB RED。

### Step 3：定义 DTO 与 stable envelope

`dto.rs` 定义并由ts-rs导出：

```rust
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListRequest {
    pub operation_id: String,
    pub organization_ids: Vec<String>,
    pub epistemic_states: Vec<String>,
    pub readiness_states: Vec<String>,
    pub capability_states: Vec<String>,
    pub source_kinds: Vec<String>,
    pub cursor: Option<String>,
    #[ts(type = "number | null")]
    pub expected_change_seq: Option<i64>,
    pub page_size: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationScopeRequest {
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisGetRequest {
    pub operation_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationTemporalSnapshotView {
    /// 固定为 2；V1 不含完整 temporal snapshot，不能继续 current multipage。
    pub contract_version: u32,
    /// 来自首个 read transaction 的 DB clock，不接受 request/cursor 外的 caller time。
    pub as_of_temporal_cutoff: String,
    /// 首个 read snapshot 中全部适用 authority epoch heads 的 canonical exact-set hash。
    pub authority_epoch_set_hash: String,
    /// 本页投影所依赖 authority members 的最早 effective valid_until。
    pub earliest_effective_valid_until: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationProjectionEnvelope {
    pub projection_schema_version: u32,
    #[ts(type = "number")]
    pub change_seq: i64,
    pub read_at: String,
    pub temporal_snapshot: InvestigationTemporalSnapshotView,
    pub tool_truth_contract: String,
    pub investigation_contract_version: String,
    pub investigation_rollout_mode: String,
    pub mode_policy: InvestigationModePolicyView,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationModePolicyView {
    pub canonical_writer: String,
    pub gate_authority: String,
    pub allow_legacy_mutation: bool,
    pub campaign_write_policy: String,
    pub allow_prepared_action_jit: bool,
    pub compare_policy: String,
    pub legacy_projection_policy: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationCommandError {
    pub code: String,
    pub message: String,
    #[ts(type = "number | null")]
    pub current_change_seq: Option<i64>,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationSummaryView {
    pub envelope: InvestigationProjectionEnvelope,
    pub active_generation_id: Option<String>,
    pub active_generation_seal_hash: Option<String>,
    #[ts(type = "number")]
    pub current_hypothesis_count: i64,
    #[ts(type = "number")]
    pub closed_hypothesis_count: i64,
    #[ts(type = "number")]
    pub contested_hypothesis_count: i64,
    #[ts(type = "number")]
    pub residual_count: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListItemView {
    pub root_id: String,
    pub revision_id: String,
    pub organization_id: String,
    pub subject_kind: String,
    pub subject_identity_hash: String,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub predicate_schema: String,
    pub predicate_summary: String,
    pub trust_boundary: String,
    pub polarity: String,
    pub epistemic_state: String,
    pub lifecycle_state: String,
    pub planning_readiness: String,
    #[ts(type = "number")]
    pub support_count: i64,
    #[ts(type = "number")]
    pub contradiction_count: i64,
    #[ts(type = "number")]
    pub gap_count: i64,
    pub legacy_projection_status: Option<String>,
    pub residual_codes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisListView {
    pub envelope: InvestigationProjectionEnvelope,
    pub hypotheses: Vec<InvestigationHypothesisListItemView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../../frontend/lib/generated/")]
pub struct InvestigationHypothesisDetailView {
    pub envelope: InvestigationProjectionEnvelope,
    pub hypothesis: InvestigationHypothesisListItemView,
    pub predecessor_revision_id: Option<String>,
    pub lineage_revision_ids: Vec<String>,
    pub support_ref_ids: Vec<String>,
    pub contradiction_ref_ids: Vec<String>,
    pub application_context_ref_ids: Vec<String>,
    pub gap_ref_ids: Vec<String>,
    pub verification_objective_summaries: Vec<String>,
    pub legacy_unavailable_fields: Vec<String>,
}
```

summary/list/detail DTO只能包含at-time identity、structured claim摘要、三轴state、readiness、source计数、lineage IDs、objective摘要、residual code和legacy projection status；禁止raw payload、credential、prompt/prose artifact、lease token、checkpoint、cursor salt。

Plan B同时在`golish-core::investigation_projection`冻结、在DB`types.rs/timeline.rs`消费内部typed Timeline contract；`ProjectionEntityKind`、`TimelineEventKind`、`ProjectionSourceTimeStatusV1`、`ProjectionInvalidationReason`必须直接import自该core模块，DB/app不得重定义mirror enum。四个enum本Task生成独立TS bindings供Plan D复用；本Task暂不新增Timeline event DTO IPC，Plan D只能做authorized wrapper与分页展示：

```rust
pub struct ProjectionEntityRefV1 {
    pub kind: ProjectionEntityKind,
    pub entity_id: String,
    pub entity_version: u64,
}

pub struct InvestigationTimelineEventV1 {
    pub event_id: Uuid,
    pub change_seq: i64,
    pub event_kind: TimelineEventKind,
    pub entity: ProjectionEntityRefV1,
    pub organization_id: Option<Uuid>,
    pub source_occurred_at: Option<DateTime<Utc>>,
    pub source_time_status: ProjectionSourceTimeStatusV1,
    pub projected_at: DateTime<Utc>,
    pub invalidation_reason: Option<ProjectionInvalidationReason>,
}
```

constructor只接受persisted typed change+entity-version pair，验证kind/version/batch/change hash一致；`event_kind`已经表达`HypothesisSuperseded`、`LegacyAttemptProjectionInvalidated`等业务语义，frontend不得把generic insert/close改写成自然语言结论。event不携raw payload、模型prose或任意summary JSON；详情通过authorized entity API读取。

### Step 4：实现 opaque cursor

`cursor.rs` payload固定：

```rust
#[derive(Debug, Serialize, Deserialize)]
struct InvestigationCursorV2 {
    version: u8,
    resource_kind: String,
    operation_id: Uuid,
    projection_schema_version: u32,
    as_of_change_seq: i64,
    as_of_temporal_cutoff: DateTime<Utc>,
    authority_epoch_set_hash: String,
    earliest_effective_valid_until: DateTime<Utc>,
    tool_truth_contract: String,
    investigation_contract_version: String,
    investigation_rollout_mode: String,
    filter_digest: String,
    page_size: u32,
    stable_sort_key: InvestigationStableSortKeyV1,
}

/// 仅供已存在的historical/legacy token做受限decode；current writer永不签发V1。
#[derive(Debug, Serialize, Deserialize)]
struct InvestigationCursorV1Legacy {
    version: u8,
    resource_kind: String,
    operation_id: Uuid,
    projection_schema_version: u32,
    as_of_change_seq: i64,
    tool_truth_contract: String,
    investigation_contract_version: String,
    investigation_rollout_mode: String,
    filter_digest: String,
    page_size: u32,
    stable_sort_key: InvestigationStableSortKeyV1,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InvestigationStableSortKeyV1 {
    Hypothesis {
        organization_ordinal: i32,
        group_key: String,
        readiness_rank: i16,
        epistemic_rank: i16,
        root_id: Uuid,
        revision_ordinal: i32,
    },
    Campaign { wave_ordinal: i64, campaign_ordinal: i64, campaign_id: Uuid },
    Timeline { change_seq: i64, event_id: Uuid },
}
```

current writer的`version`固定为`2`、`projection_schema_version`固定为`1`；B现在就预留Hypothesis/Campaign/Timeline三种tagged key，Plan D只能复用同一个V2 converter与frozen temporal fields，不能另建cursor topology。`as_of_temporal_cutoff`必须是第一页同一read transaction的DB clock；`authority_epoch_set_hash`是该snapshot适用authority heads的canonical exact-set hash；`earliest_effective_valid_until`是所有返回行依赖authority member的最早有效期，三者与`as_of_change_seq`一起在所有后续页原样重放。后续页在授权和签名验证后、查询前，必须用DB clock验证`now <= earliest_effective_valid_until`并重算current authority epoch exact-set hash；TTL已过或epoch-set漂移即使head未变化也返回`INVESTIGATION_PROJECTION_STALE`、`restart_required=true`。caller request time、进程clock及游标外补读值一律不参与。

使用`base64` URL-safe编码，并以DB head中的32-byte `cursor_salt`作为key，对canonical payload计算HMAC-SHA256；验证使用`Mac::verify_slice` constant-time比较，禁止自制`sha256(payload || salt)` MAC。command先解析并授权operation，再验证签名/resource/filter/version与全部temporal fields；detail selector必须在operation授权后解析并做operation membership检查，避免用selector泄漏跨operation存在性。page size确定性clamp到`1..=100`；空filter数组表示“不限制”，重复值先去重排序再计算filter digest，unknown值或互斥组合返回`INVESTIGATION_INVALID_ARGUMENT`。tamper/signature/resource/filter/operation/temporal-field mismatch返回`INVESTIGATION_CURSOR_INVALID`；合法签名遇head、TTL或epoch-set drift才返回`INVESTIGATION_PROJECTION_STALE`并带`restart_required=true`。

`InvestigationCursorV1Legacy`只保留历史/legacy token的验证与decode能力：可用于显式historical single-page读取或返回restart提示，但不得为current Registry请求生成第二页、不得转换成V2续页、也不得签发新的V1。current multipage遇合法V1统一返回`INVESTIGATION_PROJECTION_STALE`并要求从第一页重启；无效V1签名仍返回`INVESTIGATION_CURSOR_INVALID`。V1缺少temporal cutoff/epoch set/effective-valid-until，绝不能仅凭`as_of_change_seq`被视为current snapshot authority。

Plan B 的 Hypothesis query 从一开始就按上述完整六字段 keyset 排序；`group_key`是server-derived grouping key，rank字段由canonical enum映射，最终以root/revision ordinal打破平局。Plan D只能复用该顺序，不能把同一个V2 cursor解释成另一种两字段排序。

在 `backend/Cargo.toml` workspace dependencies 增加 `hmac = "0.12"`；在 `backend/crates/golish-agent-app/Cargo.toml` 增加 `base64.workspace = true` 与 `hmac.workspace = true`，复用已有 `sha2.workspace = true`；不引入第二套cursor签名库。

### Step 5：实现 repeatable-read read model 与 commands

`mod.rs`只定义三个B-owned command：

```rust
#[tauri::command]
pub async fn investigation_get_summary(
    request: InvestigationScopeRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationSummaryView, InvestigationCommandError>;

#[tauri::command]
pub async fn investigation_list_hypotheses(
    request: InvestigationHypothesisListRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationHypothesisListView, InvestigationCommandError>;

#[tauri::command]
pub async fn investigation_get_hypothesis(
    request: InvestigationHypothesisGetRequest,
    state: State<'_, AgentState>,
) -> Result<InvestigationHypothesisDetailView, InvestigationCommandError>;
```

授权顺序沿用`authorize_reporting_scope`：principal → operation → active project → sealed scope/root membership → selector scope。`investigation_projection/mod.rs`在一个`REPEATABLE READ READ ONLY`事务中原子固定head/read_at、DB-clock `as_of_temporal_cutoff`、适用authority epoch heads exact-set hash与结果依赖members的`earliest_effective_valid_until`；所有`summary.rs/hypotheses.rs/legacy.rs/timeline.rs`查询只消费`change_seq <= captured head`的materialized entity versions/change rows，禁止join尚未project的canonical新row补齐，也禁止事务外分别读取epoch/TTL而形成混合快照。首个current list page以这四项构造V2 cursor和`InvestigationTemporalSnapshotView`；后续页必须exact复用并在同一DB transaction以current DB clock/current authority epoch set重验，head不变但TTL已过或epoch变化也fail closed并要求restart。`legacy.rs`在冻结legacy mode下读取projector产生或historical-backfill的只读compatibility version，缺失新字段显式`legacy_unavailable`；历史V1 cursor只允许受限decode/single-page或restart，绝不能继续current multipage。Plan D只在此目录新增`version.rs/campaigns.rs`并为既有`timeline.rs`增加authorized IPC wrapper，不搬迁、重写或另建Timeline semantic mapper，也不定义第二套cursor/temporal snapshot contract。

在facade/mod/registry注册三个command；禁止`commands_registry.rs`直接glob app commands。

### Step 6：运行 GREEN 与生成 bindings

`golish-agent-app`的`export_bindings` test必须显式调用四个core enum与`InvestigationTemporalSnapshotView`/envelope DTO的ts-rs export，同一批写出；generated路径仍由owner type的`#[ts(export_to=...)]`唯一决定，不能在app/frontend复制union或temporal snapshot文字。

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_timeline_) | test(investigation_projection_catalog_) | test(projection_ts_decl_golden_)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization -E 'test(investigation_auth_) | test(investigation_cursor_v2_) | test(investigation_cursor_v1_legacy_) | test(investigation_temporal_snapshot_)')
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry -E 'test(projection_read_head_isolation_) | test(projection_temporal_snapshot_) | test(projection_temporal_ttl_between_pages_) | test(projection_epoch_drift_without_head_change_) | test(timeline_typed_semantics_) | test(projection_dual_time_)')
just space-guard
(cd backend && cargo test -p golish-agent-app export_bindings -q)
```

Expected：authorization、V2 cursor canonical round-trip/逐temporal字段tamper、V1 legacy decode/current continuation拒绝、TTL跨页到期、head不变但epoch-set漂移、caller clock忽略、materialized-head isolation、typed Timeline catalog/invalidation、source-vs-projected time tests全`PASS`；ts-rs只生成本Task列出的Investigation DTO及四个core catalog enum bindings，四个TS union与Rust`ALL/as_str`逐值exact-equal，`InvestigationTemporalSnapshotView.ts`/envelope golden exact含`contractVersion/asOfTemporalCutoff/authorityEpochSetHash/earliestEffectiveValidUntil`且没有mirror/free-string字段，未手改其他generated文件；本Task没有Timeline event public IPC，但Plan D可直接复用已冻结的typed query/cursor/temporal contract与bindings。

### Future Commit

```bash
git add backend/Cargo.toml backend/crates/golish-core/src/investigation_projection.rs backend/crates/golish-db/src/repo/investigation_projection/mod.rs backend/crates/golish-db/src/repo/investigation_projection/types.rs backend/crates/golish-db/src/repo/investigation_projection/summary.rs backend/crates/golish-db/src/repo/investigation_projection/hypotheses.rs backend/crates/golish-db/src/repo/investigation_projection/legacy.rs backend/crates/golish-db/src/repo/investigation_projection/timeline.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/hypothesis_registry.rs backend/crates/golish-agent-app/src/ai/commands/investigation/mod.rs backend/crates/golish-agent-app/src/ai/commands/investigation/dto.rs backend/crates/golish-agent-app/src/ai/commands/investigation/cursor.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish-agent-app/Cargo.toml backend/crates/golish/src/commands_facade/investigation.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs backend/crates/golish-agent-app/tests/investigation_ipc_authorization.rs frontend/lib/generated/InvestigationScopeRequest.ts frontend/lib/generated/InvestigationHypothesisListRequest.ts frontend/lib/generated/InvestigationHypothesisGetRequest.ts frontend/lib/generated/InvestigationSummaryView.ts frontend/lib/generated/InvestigationHypothesisListView.ts frontend/lib/generated/InvestigationHypothesisListItemView.ts frontend/lib/generated/InvestigationHypothesisDetailView.ts frontend/lib/generated/InvestigationTemporalSnapshotView.ts frontend/lib/generated/InvestigationProjectionEnvelope.ts frontend/lib/generated/InvestigationModePolicyView.ts frontend/lib/generated/InvestigationCommandError.ts frontend/lib/generated/ProjectionEntityKind.ts frontend/lib/generated/ProjectionInvalidationReason.ts frontend/lib/generated/TimelineEventKind.ts frontend/lib/generated/ProjectionSourceTimeStatusV1.ts
git commit -m "feat(investigation): expose hypothesis audit read model"
```

---

## Task 12：交付最小只读 Hypothesis Registry Audit UI

**文件：**

- 创建：`frontend/lib/api/investigation.ts`
- 修改：`frontend/lib/api/{index.ts,error-codes.ts}`
- 创建：`frontend/components/Engagement/{HypothesisRegistryAudit.tsx,HypothesisRegistryAudit.test.tsx}`
- 修改：`frontend/components/Engagement/index.ts`
- 修改：`frontend/components/ToolCallDetailView/{ToolCallDetailView.tsx,ToolCallDetailView.candidate.test.tsx}`

### Step 1：写 UI RED

测试覆盖：首次loading；summary和list独立error/retry；empty；刷新时保留旧数据并显示stale；五态badge；legacy缺字段显示`legacy_unavailable`；unsupported与`plan_c_verification_unavailable` residual可见；点击项加载detail；旧响应晚到不能覆盖新operation；主DOM不出现`Queue N`。

```tsx
it("keeps the Registry authoritative while showing the Plan C residual", async () => {
  render(
    <HypothesisRegistryAudit
      operationId="operation-1"
      api={registryAuthoritativeFixtureApi()}
    />
  );
  expect(await screen.findByText("registry_authoritative_legacy_projection")).toBeVisible();
  expect(screen.getByText("plan_c_verification_unavailable")).toBeVisible();
  expect(screen.queryByText(/Queue \d+/)).not.toBeInTheDocument();
});
```

### Step 2：运行 RED

```bash
pnpm exec vitest run frontend/components/Engagement/HypothesisRegistryAudit.test.tsx
```

Expected：component/API module不存在，test失败。

### Step 3：添加 typed API wrapper

```ts
import type { InvestigationHypothesisDetailView } from "@/lib/generated/InvestigationHypothesisDetailView";
import type { InvestigationHypothesisGetRequest } from "@/lib/generated/InvestigationHypothesisGetRequest";
import type { InvestigationHypothesisListRequest } from "@/lib/generated/InvestigationHypothesisListRequest";
import type { InvestigationHypothesisListView } from "@/lib/generated/InvestigationHypothesisListView";
import type { InvestigationScopeRequest } from "@/lib/generated/InvestigationScopeRequest";
import type { InvestigationSummaryView } from "@/lib/generated/InvestigationSummaryView";
import { invoke } from "./client";

export const getInvestigationSummary = (request: InvestigationScopeRequest) =>
  invoke<InvestigationSummaryView>("investigation_get_summary", { request });

export const listInvestigationHypotheses = (request: InvestigationHypothesisListRequest) =>
  invoke<InvestigationHypothesisListView>("investigation_list_hypotheses", { request });

export const getInvestigationHypothesis = (request: InvestigationHypothesisGetRequest) =>
  invoke<InvestigationHypothesisDetailView>("investigation_get_hypothesis", { request });
```

error map增加并冻结`INVESTIGATION_FORBIDDEN`、`INVESTIGATION_INVALID_ID`、`INVESTIGATION_INVALID_ARGUMENT`、`INVESTIGATION_CURSOR_INVALID`、`INVESTIGATION_PROJECTION_STALE`、`INVESTIGATION_AUTHORITY_CORRUPT`、`INVESTIGATION_DATABASE`、`INVESTIGATION_LEGACY_PROJECTION_DIVERGED`。公开request中的ID保持`String`/`Vec<String>`，在handler内映射malformed ID；unknown或互斥filter映射invalid argument，不能让serde/UUID解析错误或“空结果”越过稳定错误契约。Plan D只能复用这些code和`InvestigationProjectionEnvelope/InvestigationTemporalSnapshotView/InvestigationCommandError/InvestigationCursorV2`；`InvestigationCursorV1Legacy`仅供B-owned historical decoder，D不得用它继续current pagination、另建cursor版本或改envelope topology。attack mutation继续使用`ATTACK_LEGACY_MUTATION_FORBIDDEN_BY_INVESTIGATION_CONTRACT`，D不得另造同义 code。

### Step 4：实现 audit component

component用request sequence防旧响应覆盖；summary/list/detail分别维护loading/error/empty；refresh保留当前data并显示`stale`。展示：mode、generation seal、hypothesis counts、三轴state/readiness、support/conflict/gap counts、legacy projection status、residual codes、at-time subject。

这是Plan D前的Audit panel，不实现Campaign/Wave/Timeline/JIT按钮，也不把旧queue复制进主视图。

### Step 5：从 exact Candidate stage_run operation挂载

在`ToolCallDetailView.tsx`增加纯helper，只在所有Candidate rows携带同一非空operationId时返回；冲突/缺失时不挂载，禁止从session-global hint猜operation。

legacy前三态保留现有Review/Attempt UI；Audit panel并排提供只读projection。新权威mode不产生legacy review hint，因而只显示Audit；即使伪造旧mutation，Task 10 DB guard也会拒绝。

### Step 6：运行 GREEN、Biome与typecheck

```bash
pnpm exec vitest run frontend/components/Engagement/HypothesisRegistryAudit.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
pnpm exec biome check frontend/lib/api/investigation.ts frontend/lib/api/index.ts frontend/lib/api/error-codes.ts frontend/components/Engagement/HypothesisRegistryAudit.tsx frontend/components/Engagement/HypothesisRegistryAudit.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
pnpm typecheck
```

Expected：focused Vitest全绿；Biome exit 0；typecheck exit 0；无raw `invoke()` 出现在component中。

### Future Commit

```bash
git add frontend/lib/api/investigation.ts frontend/lib/api/index.ts frontend/lib/api/error-codes.ts frontend/components/Engagement/HypothesisRegistryAudit.tsx frontend/components/Engagement/HypothesisRegistryAudit.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
git commit -m "feat(frontend): add hypothesis registry audit panel"
```

---

## Task 13：定向门禁、模块卡与 evidence 收尾

**文件：**

- 修改：`docs/modules/backend/golish-core.md`
- 修改：`docs/modules/backend/golish-db.md`
- 修改：`docs/modules/backend/golish-db/repo.md`
- 修改：`docs/modules/backend/golish-agent-kit.md`
- 修改：`docs/modules/backend/golish-agent-kit/harness.md`
- 修改：`docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改：`docs/modules/backend/golish-agent-kit/db_traits.md`
- 修改：`docs/modules/backend/golish-agent-app/ai.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/backend/golish-agent-bridge/agent_bridge.md`
- 修改：`docs/modules/backend/golish-agent-bridge/bridge_executor.md`
- 修改：`docs/modules/backend/golish-sub-agents/defaults.md`
- 修改：`docs/modules/backend/golish-sub-agents/executor.md`
- 修改：`docs/modules/frontend/lib.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`agent-progress.md`
- 修改：`feature_list.json`
- 只读核对：`clean-state-checklist.md`

### Step 1：先运行全部新鲜定向测试

```bash
just space-guard
(cd backend && cargo nextest run -p golish-core -E 'test(investigation_) | test(verification_contract_) | test(hypothesis_claim_component_) | test(hypothesis_verification_plan_) | test(hypothesis_revision_adjudication_) | test(investigation_projection_catalog_) | test(projection_plan_c_route_catalog_) | test(projection_ts_decl_golden_) | test(legal_contract_mode_pairs)')
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit --test hypothesis_registry_gate)
just space-guard
(cd backend && cargo nextest run -p golish-db --test hypothesis_registry)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test candidate_analysis_runtime)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app --test investigation_ipc_authorization)
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -E 'test(candidate_analysis_dispatch) | test(candidate_analysis_stage_run)')
just space-guard
(cd backend && cargo nextest run -p golish-sub-agents -E 'test(candidate_hypothesis) | test(merge_conflict)')
just space-guard
(cd backend && cargo test -p golish-agent-app export_bindings -q)
jq empty resources/harness/stages/attack_candidate/spec.json
pnpm exec vitest run frontend/components/Engagement/HypothesisRegistryAudit.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
pnpm typecheck
```

Expected：每条 exit 0；无 ignored/zero-matched test误报；export后四个projection TS enum bindings与Rust golden exact-equal且无手写mirror。记录命令、exit code和关键PASS摘要到`agent-progress.md`。

### Step 2：运行 scoped format/lint

```bash
just space-guard
(cd backend && cargo fmt -p golish-core -p golish-db -p golish-agent-kit -p golish-agent-runtime -p golish-agent-bridge -p golish-agent-app -p golish-sub-agents -p golish -- --check)
just space-guard
(cd backend && cargo clippy -p golish-core --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-db --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-kit --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-runtime --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-bridge --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-agent-app --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish-sub-agents --all-targets -- -D warnings)
just space-guard
(cd backend && cargo clippy -p golish --all-targets -- -D warnings)
pnpm exec biome check frontend/lib/api/investigation.ts frontend/lib/api/index.ts frontend/lib/api/error-codes.ts frontend/components/Engagement/HypothesisRegistryAudit.tsx frontend/components/Engagement/HypothesisRegistryAudit.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
```

Expected：全部exit 0，Rust/Clippy零warning，Biome无diff。

### Step 3：更新模块卡与进度事实

模块卡写清：Registry canonical authority、operation-frozen mode、two-wave runtime、Stage Team仅作控制面、minimal read API、Plan C residual边界。`docs/modules/INDEX.md`同步状态列。

`feature_list.json.verification`逐条核对并填新鲜evidence；未授权的大型门禁如实写“按项目策略未运行”，不能伪称通过。只有1+2+3+4+5完成定义全部满足才能标`passing`。

### Step 4：对照 clean-state checklist

确认没有：第二个`in_progress`、手改generated文件、schema外migration、Campaign/Prepared Action/oracle实现、rollout promotion、真实provider/目标调用、无证据完成声明。若有未commit半成品，在progress列出每个文件。

### Future Commit

```bash
git add docs/modules/backend/golish-core.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-kit/db_traits.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-bridge/agent_bridge.md docs/modules/backend/golish-agent-bridge/bridge_executor.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/frontend/lib.md docs/modules/frontend/components.md docs/modules/INDEX.md agent-progress.md feature_list.json
git commit -m "docs(investigation): record Plan B evidence"
```

---

## 完成判据

- 历史 operation保持`legacy_candidate_v1 + legacy_only`，resume/default变化不升级；fork升级有exact adoption receipt。
- 五态writer/Gate/legacy mutation/projection/JIT矩阵有pure+DB contract tests，Plan B所有mode都不能授权新hypothesis execution。
- semantic key只包含冻结identity字段；root/revision公式、reopen/split/merge/derive/collision/order稳定。
- 每org snapshot在Plan A `with_checked_tool_truth_authority_bundle`同request/guard/DB transaction内封存server-derived TI/EAS/Enum/Vuln multi-root exact census、bundle seal/root/member/receipt及graph/semantic/freshness/temporal hashes；漏root、cross-org、任一root semantic-invalid/temporal-nonfresh/tamper只能原子写`blocked_authority_bundle` snapshot+census+residual/obligation，不能启动analysis/Gate，caller不能过滤stale或构造/cache guard。
- `EvidenceTemporalValidityPolicyV1`冻结observed-at/effective-valid-until/target-state epoch-set/max-skew，negative/refutation TTL各自短于positive；`expired|mixed_epoch|skew_exceeded` facts不进Candidate input，Gate前TTL到期或epoch-set漂移也重开snapshot。signed CVE/CPE/KEV/advisory/rule feed与product-version matcher形成host exact census，match仅为knowledge signal；stale feed/unknown version落residual/obligation，Candidate不临时联网browse/refresh。
- ready snapshot包含完整source sets和逐input immutable redacted typed chunk bodies/blob refs；server-owned chunk census证明无截断，page只读snapshot materialization且只证明bytes交付，freeze后live source修改/删除仍返回相同bytes/hash。
- page/read receipts由server产生并绑定`input_id/chunk_ordinal/chunk_census_hash/source_size/chunking+redaction version`；每input exact-one disposition、零到多hypothesis relation。
- 第一波H1 proposal census和第二波critic census都按analysis attempt形成exact set；H1后每input生成`checklist-member × chunk-partition` subreview exact census，bounded map critics与recursive cross-chunk/cross-input/cross-dimension tree必须归并到exact-one org/snapshot global semantic root，随后host才写global/per-input `adequate|missed_hypothesis|blocked` review。单critic/page receipt不能假装理解大input，sampling/context truncation只能blocked/degraded；跨partition/class/boundary组合、已有proposal之外的第二/第三hypothesis仍须检查，zero-proposal只是空H1-ref特例。`missed_hypothesis`以append-only连续attempt chain重开且旧receipt不能复用；2–8是live concurrency cap，不是work-item总数。
- 只有Controller提交final decisions；children只读/tool-free；目标内容永远`instruction_authority=false`。
- Candidate mutation只允许`proposed/supported/contested/inconclusive`；`invalid`仅server validator可写。`verified/refuted`只能由Plan C在Plan A live `AllFreshToolTruthAuthorityBundle` callback内写入：current B-owned plan + typed objective/component outcomes/lineage + revision adjudication + acyclic transition decision/receipt + Finding/refutation + exact-five projection manifest必须全部exact绑定；Campaign terminal/oracle只是objective evidence members，单个terminal永远不充分。
- `golish-core::verification_contract`唯一拥有`VerificationContractV1`、`ContractCombinatorV1`、`PredicateComponentV1`、`VerificationControlV1`及pair/order bindings；combinator闭集只有`all_of/any_of/paired_differential/ordered_sequence`，Plan C直接import且不重定义。
- `golish-core::hypothesis_verification`唯一拥有claim component、plan、adjudication与transition types；每条proof path union覆盖全部required components，optional-only falsifier无效。`Verified := ∃`一条winning path全required proof；`Refuted := ∀`path各有designated valid required-component falsifier；两式均不成立才NonTerminal。其他路径/非决定性未决项进入limitation而不否决已成立的终态，decision-blocking live-path项才进入unresolved；漏component只能拒绝或创建narrow successor，不能终结原宽claim。
- Gate对跨org/subject/trust/polarity merge、gap refutation、AU/knowledge-signal proof、adapter-gated existence、multi-root/temporal/feed/subreview/synthesis/claim-component/contract/plan drift fail closed；Plan B不创建或查询Plan C-owned capability assessment authority。
- Registry canonical transaction只与完整immutable outbox source batch及其operation-local`source_batch_seq/predecessor`原子；每个outbox member冻结typed source body/blob，projector绝不回读live source。projector按最小连续batch验证entity direct predecessor，把全部entity versions/change/timeline/适用的legacy compatibility versions、batch receipt与projection head一次原子提交；较晚worker必须等前驱。projector或legacy compatibility失败不能回滚canonical truth，未推进head前reader只见旧完整batch；rebuild保持source order/change seq/version/event ID/canonical manifest hash，`projected_at`可不同且不进hash。
- Projection只使用typed `ProjectionEntityKind/ProjectionChangeKind/TimelineEventKind/ProjectionInvalidationReason/ProjectionSourceTimeStatusV1` catalog；B-owned verification plan seal、Plan C revision adjudication close/invalidate、Campaign terminal leaf及consult/strategy/budget/cleanup/callback/evolution/refinement等canonical mutations都有exact专属entity或typed Coverage aggregate route，unknown mutation在source commit前fail closed而不会卡head。四个public catalog enum只由core derive ts-rs并生成exact TS bindings，无mirror/free string；Timeline同时保存`source_occurred_at`与`projected_at`，unknown catalog/time mapping一律fail closed。
- 新权威mode不调用旧Verifier/Campaign/Prepared Action/oracle，只产生`plan_c_verification_unavailable` residual并转Reporting。
- 三个read commands经过project/scope/org授权并支持V2 stable cursor；第一页在同一DB read snapshot冻结`as_of_change_seq + as_of_temporal_cutoff + authority_epoch_set_hash + earliest_effective_valid_until`，后续页在head未变时仍对TTL/epoch drift fail closed并要求restart。V1只作historical/legacy decode，不能继续current multipage；deleted live target仍显示at-time identity。
- Audit UI有loading/error/empty/stale、mode、residual和legacy_unavailable；主DOM不引入queue-centric产品模型。
- 所有定向验证有新鲜、可重放evidence；未获授权的大型门禁未运行且如实记录。
