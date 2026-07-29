# 协作式 Verification Campaign、Prepared Action 与 Typed Oracle 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把现有以单个 `CandidateAttempt` 和 allowlisted replay 为中心的 Verification，渐进替换为可持久恢复的多 Agent Campaign；每轮先形成可证伪策略，再由服务端编译、展示、授权并执行具体 Prepared Action，最后由 deterministic action oracle 形成objective-local outcome/FactDelta，再由revision-level adjudicator依据sealed `HypothesisVerificationPlanV1`、latest objective outcome exact set与proof-path外层量词至多一次生成 Finding/refutation。

**架构：** 复用现有 operation/stage、organization isolation、scope、evidence ledger、lease/CAS、action journal、response-loss recovery 与 Finding lineage。新增纯领域 Campaign 状态机、append-only campaign/round/consult/strategy/action/oracle/coverage tables、typed repository seam、host-owned Action Compiler/Authorization Broker/Executor/Oracle/Terminalizer。认知 Agent 只能写 typed proposal/decision intent；目标、凭证、canonical args、authorization、oracle verdict、Finding、coverage receipt 和 FactDelta 均由 host/DB authority 生成。`legacy_only` 继续旧链；`shadow_registry / dual_read_compare` 只运行与 executor/auth/credential port 类型隔离的无副作用 shadow evaluation；只有 operation-frozen `registry_authoritative_legacy_projection / new_only` 且 `tool_truth_contract=receipt_v1`、reconciliation clean、coverage denominator sealed 时可以调度真实 Campaign。

**技术栈：** Rust 2021、Tokio、sqlx/PostgreSQL、rig-core、Tauri 2、ts-rs、React 19、TypeScript 6、Vitest、Biome。

**设计依据：** `docs/design/2026-07-29-tool-truth-hypothesis-verification-loop.md` §7–§11、§14–§18。

**执行前置：** Plan A 的 Tool Truth receipt/reconciliation 与 Plan B 的 Hypothesis Registry/generation/consolidation contract 必须已 `passing`。开始 Task 3 的 schema/migration 工作前，必须再次取得用户对修改 `golish-db` schema/migration 的明确授权。当前计划文件本身不授权 migration、真实扫描、provider 调用或 rollout promotion。

**当前停点：** 各 Task 的 `Future Commit` 是未来实现时的原子提交边界；本轮不执行实现、migration或产品测试。设计/计划/状态文档按用户要求单独commit，但不push。

## 不变量与交付边界

- 一个revision可以有多个verification objective；同一 `hypothesis_revision + objective_id + verification_contract_hash` 同时最多一个 active Campaign。首个Campaign admission前必须seal `HypothesisVerificationPlanV1`的exact objective/contract/proof-path集合；predicate/subject/trust boundary变化必须回到下一Registry generation，不能在Campaign内偷换。
- Campaign 可真实执行的唯一公式是：authoritative-new investigation mode + `receipt_v1` + Plan A request-scoped multi-root `AllFreshToolTruthAuthorityBundle<'guard>` + sealed verification plan/Wave/Campaign denominator + sealed capability assessment set + authorized Prepared Action + rollout safety hold 未启用；缺任一项都在任何 provider/adapter 前 fail closed。caller传入的裸`ReconciliationState/hash`、单root set或先过滤stale root的集合没有authority。
- coverage denominator 表示“必须验证的 objective/control obligation”，必须在第一个 Prepared Action 获授权前冻结，不能从实际执行过的 action 反推；新事实只能进入下一 generation/Campaign。
- consult 可以有界并行；Campaign外层同一时刻只允许一个active Prepared Action。普通Action为单请求；race/TOCTOU/double-spend类objective必须用一个原子的`ConcurrentActionGroupV1`（bounded subrequest exact set、start barrier/window、union conflict-key leases、aggregate budget/JIT、逐subaction receipt与deterministic concurrency oracle），不能用顺序action伪装。adapter不支持时记`adapter_missing + residual`并保持未覆盖。
- 每轮无条件拥有 exact-one round input、consult census、strategy decision、strategy-obligation manifest 和 round disposition；其余 artifact 只在真实发生时创建。
- `compile_rejected / no_action_compilable / denied / expired / superseded / manually_blocked` 不创建假的 execution 或 oracle。
- 只有 `authorized + durable begin` 才能在事务外执行；任何外部 HTTP、browser、CLI、OAST 或 provider 调用都不得发生在数据库事务中。
- `action_oracle_assessment.v1`只解释一个exact observation；`campaign_adjudication.v1`只聚合一个objective的复合predicate，产生objective-local proof/refutation/inconclusive，不能直接修改Hypothesis revision状态。
- Campaign closeout在一个事务内写objective outcome receipt、campaign terminal receipt、coverage receipt与exact-one immutable FactDelta；revision adjudicator必须先封存全量latest objective outcome/unassigned residual exact set，再按sealed proof paths计算外层量词，才可在另一个原子事务至多一次创建Finding或refutation lineage、revision terminal decision与Plan B state transition。blocked/untested/inconclusive对其所在path保持未决并形成coverage/residual；它们只有在没有另一条完整proof path、或仍存在一条未被designated falsifier击中的path时才阻止revision终态，不能把替代path语义偷偷改成全objective AND。
- Campaign local drain 不等待 FactDelta consumption/新 generation；Wave consolidation 与 Stage final seal 分层执行，避免互相等待。
- 前端只提交 decision、opaque ids、两个 hash、renderer version、CAS version、request id 和可选请求 expiry；不回传 target、args、secret、payload 或 risk tier。
- `frontend/lib/generated/` 只由 ts-rs 生成，禁止手改。
- 本计划的`complete`只表示sealed verification plan/Wave/Campaign denominator内的declared/planned obligations完成；在versioned `ThreatCoverageProfileV1`（asset class × trust boundary × attack class × role/identity × discovery source）上线前，全局红队coverage sufficiency固定为`not_assessed`，UI/report不得宣称“全覆盖/无漏洞”。

## 目标文件结构

### 领域与编排

- 修改 `backend/crates/golish-core/src/investigation_contract.rs`
- 复用 `backend/crates/golish-core/src/verification_contract.rs`（Plan B唯一VerificationContract类型；Plan C不得重定义）
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/mod.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/types.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/state.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/oracle.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/gate.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/verification_campaign/tests.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/mod.rs`
- 新增 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/consolidation.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/mod.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/reducer.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/rollout.rs`
- 新增 `backend/crates/golish-agent-kit/src/db_traits/verification_campaign.rs`
- 修改 `backend/crates/golish-agent-kit/src/db_traits/mod.rs`
- 修改 `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`
- 新增 `backend/crates/golish-agent-kit/src/task_orchestrator/verification_campaign.rs`

### 持久化与 bridge

- 新增 `backend/crates/golish-db/migrations/20260729000007_verification_campaigns.sql`
- 新增 `backend/crates/golish-db/src/repo/verification_campaigns.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_prepared_actions.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_oracles.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_fact_delta_bundles.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_campaign_coverage.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_capability_assessments.rs`
- 新增 `backend/crates/golish-db/src/repo/verification_campaign_shadow.rs`
- 新增 `backend/crates/golish-db/src/repo/hypothesis_objective_outcomes.rs`
- 新增 `backend/crates/golish-db/src/repo/hypothesis_revision_adjudications.rs`
- 新增 `backend/crates/golish-db/src/repo/hypothesis_consolidations.rs`
- 修改 `backend/crates/golish-db/src/repo/hypothesis_registry.rs`
- 修改 `backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`
- 新增 `backend/crates/golish-db/tests/verification_campaigns.rs`
- 新增 `backend/crates/golish-agent-app/src/ai/db_bridge/verification_campaign.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs`

### Agent team、compiler、executor 与 oracle

- 新增 `backend/crates/golish-sub-agents/src/defaults/prompts/verification_campaign.rs`
- 修改 `backend/crates/golish-sub-agents/src/defaults/prompts/mod.rs`
- 修改 `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`
- 修改 `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- 新增 `backend/crates/golish-sub-agents/src/executor/verification_campaign.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/mod.rs`
- 修改 `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- 新增 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/verification_campaign.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 新增 `backend/crates/golish-pentest-app/src/pentest_bridge/verification_action_compiler.rs`
- 新增 `backend/crates/golish-pentest-app/src/pentest_bridge/verification_oracles.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs`
- 修改 `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`

### 最小安全审批入口

- 修改 `backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- 修改 `backend/crates/golish/src/commands_facade/attack.rs`
- 修改 `backend/crates/golish/src/commands_registry.rs`
- 修改 `frontend/lib/api/attack.ts`
- 新增 `frontend/components/Engagement/PendingPreparedActionPanel.tsx`
- 新增 `frontend/components/Engagement/PendingPreparedActionPanel.test.tsx`
- 修改 `frontend/components/Engagement/index.ts`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionScopeRequest.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionDecision.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionDecisionRequest.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionReviewItem.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionReviewState.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionDisplayView.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionBudgetAxisView.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionAuthorizationView.ts`
- 由 ts-rs 生成 `frontend/lib/generated/AttackPreparedActionDecisionResponse.ts`

### 文档

- 修改 `docs/modules/backend/golish-core.md`
- 修改 `docs/modules/backend/golish-agent-kit/harness.md`
- 修改 `docs/modules/backend/golish-agent-kit/db_traits.md`
- 修改 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/backend/golish-sub-agents/defaults.md`
- 修改 `docs/modules/backend/golish-sub-agents/executor.md`
- 修改 `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改 `docs/modules/backend/golish.md`
- 修改 `docs/modules/frontend/components.md`
- 修改 `docs/modules/frontend/lib.md`
- 修改 `docs/modules/INDEX.md`
- 修改 `agent-progress.md` 与 `feature_list.json`

## Task 1：冻结 Campaign 状态机与 oracle 组合规则（RED）

**Files:**

- Create: `backend/crates/golish-agent-kit/src/harness/verification_campaign/{mod.rs,types.rs,state.rs,oracle.rs,gate.rs,tests.rs}`
- Modify: `backend/crates/golish-agent-kit/src/harness/mod.rs`

**Step 1：先写纯函数失败测试**

在 `tests.rs` 固定至少以下 case：

1. 同一 Campaign 不能有两个 active action；
2. `denied/expired/compile_rejected` terminal disposition 不要求 execution/oracle，但必须有 reason/residual；
3. `authorized + started` 缺 execution receipt 时 Gate BLOCK；
4. `landed + reconciled` 缺 action oracle 时 Gate BLOCK；
5. `control=invalid`、`coverage=partial`、`precondition=unknown` 只能得到 `inconclusive`；
6. 一个action的proof不能自动verified一个`all_of`复合contract；`all_of`任一完整component refuted才可refute，`any_of`必须全部component refuted才可refute；
7. `no_action_compilable` 可进入 adjudication/refinement，但不能生成 execution/oracle；
8. budget stop 先进入 `stopping/draining`，不能直接 terminal；
9. response-loss execution key 与 semantic no-progress fingerprint 分离；
10. Campaign terminal 不等待 FactDelta consumption。
11. coverage denominator 在首个授权前 sealed，terminal exact results 数等于 member count；tested 必须绑定 action+capability receipt+oracle，untested/degraded/blocked 必须绑定 residual；
12. shadow evaluation 的 artifact 永远不能进入 Campaign Gate/Finding/FactDelta/Reporting authority。
13. Wave denominator 在首个 Campaign admission 前冻结 sealed generation 的全部验证 objective；每个 Wave member 必须恰好进入一个 Campaign denominator 或 explicit unassigned residual，不能因没有生成 action 而从分母消失。
14. `paired_differential`缺任一pair/control、pair identity不匹配或control invalid只能inconclusive；`ordered_sequence`必须同一session/causal chain按声明step顺序完整满足，乱序、重复、缺step不能verified。
15. 对唯一四种combinator做property table：proof/refutation/inconclusive/control-invalid/duplicate/missing的任意排列不得改变set-based结果；未知future combinator必须反序列化/dispatch失败，不能fallback。
16. 单个objective Campaign得到proof/refutation后，revision仍保持原非终态；只有sealed `HypothesisVerificationPlanV1` exact objective outcome集合可触发revision adjudication。
17. proof path规则做property table：任一sealed path全proof即可verified；每条path都有valid path-falsifier refutation即可refuted；只有既无完整proof path、又至少有一条path没有valid designated falsifier时才nonterminal。非决定性path中的`unassigned/blocked/untested/inconclusive`仍进入unresolved/coverage/residual exact set与报告限制，但不得否决已经满足的外层量词。完成顺序不得改变结果。
18. 两个Prepared Action conflict-key集合部分重叠（`{A,B}`与`{B,C}`）必须互斥；不同credential但同mutable resource也必须碰撞；多key按canonical order获取，不能死锁。
19. race-class objective只能由`ConcurrentActionGroupV1`产生；顺序action或adapter missing必须留下coverage residual，不能verified/refuted。
20. Campaign denominator/oracle/objective outcome逐项绑定Plan B `HypothesisClaimComponentV1` id/hash；每条proof path的component outcome union必须与B plan冻结的required claim-component exact set一致，漏impact qualifier或只证明较窄component不能verified较宽claim。

测试不得在Plan C重定义`VerificationContract`或`ContractCombinator`。唯一类型来自Plan B已落地的core模块：

```rust
use golish_core::verification_contract::{
    ContractCombinatorV1,
    PredicateComponentV1,
    VerificationContractV1,
    VerificationControlV1,
};

pub enum PreparedActionDisposition {
    CompileRejected,
    Denied,
    Expired,
    Superseded,
    Succeeded,
    Failed,
    OutcomeUnknown,
    ManuallyBlocked,
}

pub enum ObjectiveCampaignOutcome {
    Continue,
    Proof,
    Refutation,
    Inconclusive,
    Blocked,
    ExhaustedWithResiduals,
}

pub enum HypothesisRevisionOutcome {
    NonTerminal,
    Verified,
    Refuted,
}
```

**Step 2：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(verification_campaign_)' --status-level fail)
```

Expected: 新测试因 module/type/reducer 尚未实现而失败；现有 `attack_execution` tests 不应被修改成跳过。

**Step 3：实现最小纯领域模型**

实现 versioned typed artifacts、合法状态迁移、action/campaign oracle reducer、Gate violation code。hash 输入只接受 canonical typed projection，不包含 timestamp、row id 或模型 prose。

```rust
pub fn reduce_action_oracle(
    contract: &ActionOracleContract,
    receipt: &ReconciledExecutionReceipt,
) -> Result<ActionOracleAssessment, VerificationContractError>;

pub fn adjudicate_campaign(
    contract: &VerificationContractV1,
    census: &OracleCensus,
    obligations: &ObligationDispositionSet,
) -> Result<CampaignAdjudication, VerificationContractError>;

pub fn adjudicate_hypothesis_revision(
    plan: &HypothesisVerificationPlanV1,
    outcomes: &HypothesisObjectiveOutcomeSet,
) -> Result<HypothesisRevisionAdjudication, VerificationContractError>;
```

reducer必须exhaustive match Plan B闭集combinator并重验contract hash、component/control exact census、arity、ordinal与duplicate。真值规则固定为：

- `all_of`：全部component proof才verified；任一component在完整前置/control下refuted即可refuted；其余inconclusive。
- `any_of`：任一component proof即可verified；只有全部component在完整前置/control下refuted才refuted；其余inconclusive。
- `paired_differential`：exact pair与required control都有效，且versioned relation rule满足/反满足时才verified/refuted；缺边、错pair或control invalid一律inconclusive。
- `ordered_sequence`：全部声明step在同一execution session/causal chain按ordinal满足才verified；只有contract标明可负向判定的step在其前置、control与observation window complete后被deterministic refute，才refuted；单纯未观察到/乱序只能inconclusive。

第二层revision reducer不复用单objective combinator。`HypothesisVerificationPlanV1`由host从Candidate generation冻结成一个或多个ordered proof paths，每个path成员绑定exact objective id、VerificationContract hash与`path_falsifier`标记。它先为每条path确定`proved / falsified / unresolved`：全体required member为proof才`proved`；至少一个valid refutation且该member被标为`path_falsifier`才`falsified`；其余为`unresolved`。外层规则固定为`Verified := exists(proved path)`，否则`Refuted := all(path is falsified)`，否则`NonTerminal`。winning path以外或已falsified path内的非决定性未决member仍保留在unresolved/coverage/residual/report limitation exact set，但不能否决已经满足的外层量词。未知plan contract/version fail closed。Campaign/Lead不能自报path或outer verdict。

**Step 4：运行 GREEN 与格式检查**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(verification_campaign_)' --status-level fail)
just space-guard
(cd backend && cargo fmt -p golish-agent-kit -- --check)
```

Expected: 全部新纯函数测试通过；非法状态均返回稳定 code，不 panic、不接受 prose verdict。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/verification_campaign/mod.rs backend/crates/golish-agent-kit/src/harness/verification_campaign/types.rs backend/crates/golish-agent-kit/src/harness/verification_campaign/state.rs backend/crates/golish-agent-kit/src/harness/verification_campaign/oracle.rs backend/crates/golish-agent-kit/src/harness/verification_campaign/gate.rs backend/crates/golish-agent-kit/src/harness/verification_campaign/tests.rs backend/crates/golish-agent-kit/src/harness/mod.rs
git commit -m "feat(verification): add campaign domain contract"
```

## Task 2：先冻结 operation-frozen cutover 与权限矩阵

**Files:**

- Modify: `backend/crates/golish-agent-kit/src/harness/verification_campaign/tests.rs`
- Modify: `backend/crates/golish-core/src/investigation_contract.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/rollout.rs`

**Step 1：写 exact mode matrix RED**

固定五种 mode：

| Mode | Canonical Campaign | Isolated shadow evaluation | Prepared Action dispatch | Legacy projection |
|---|---:|---:|---:|---:|
| `legacy_only` | no | no | no | legacy authority |
| `shadow_registry` | no | planner/matcher/oracle replay only | no | legacy authority |
| `dual_read_compare` | no | exact-set compare only | no | legacy authority |
| `registry_authoritative_legacy_projection` | yes（还要求 `receipt_v1`） | optional audit | yes | canonical-derived |
| `new_only` | yes（还要求 `receipt_v1`） | no | yes | read historical only |

新增测试证明 deployment default 变化不改变既有 operation；same-operation resume 保留 frozen mode；shadow/compare divergence 只能阻止 promotion，不能授权新 action。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(verification_campaign_rollout_)' --status-level fail)
```

Expected: tests 因 joint Tool Truth admission、shadow-evaluation policy 和 safety-hold check 尚未实现而失败；旧 legacy mode tests 仍通过。

**Step 2：复用唯一 policy 并实现 admission guard**

五态canonical writer/Gate/legacy mutation/JIT/compare语义只能读取Plan B `InvestigationRolloutMode::policy()`；本Task不定义第二套mode matrix。路由选择、shadow admission与真实Campaign execution admission必须拆成三个API，避免legacy/shadow路径被迫伪造不存在的Registry generation、Campaign denominator或capability assessment：

```rust
pub fn select_campaign_route(
    operation_contract: &PersistedOperationContractSnapshot,
) -> Result<CampaignRoute, VerificationContractError>;

pub fn authorize_shadow_evaluation(
    route: CampaignRoute,
    census: &ShadowEvaluationCensusSeal,
) -> Result<ShadowEvaluationAuthority, VerificationContractError>;

pub(crate) fn authorize_campaign_execution<'guard>(
    route: CampaignRoute,
    tool_truth: &'guard AllFreshToolTruthAuthorityBundle<'guard>,
    generation: &HypothesisGenerationSeal,
    verification_plan: &HypothesisVerificationPlanSeal,
    wave_denominator: &VerificationWaveDenominatorSeal,
    campaign_denominator: &CampaignCoverageDenominatorSeal,
    capability_assessments: &CapabilityAssessmentSetSeal,
    safety_hold: &CampaignDispatchHoldSnapshot,
) -> Result<CampaignExecutionAuthority<'guard>, VerificationContractError>;
```

类型归属一次冻结：`PersistedOperationContractSnapshot`、`HypothesisGenerationSeal`与唯一`HypothesisVerificationPlanV1/Seal`来自Plan B typed repo/core；opaque、不可Clone/Serialize的`CheckedToolTruthAuthorityBundle<'guard>/AllFreshToolTruthAuthorityBundle<'guard>`来自Plan A host guard；`VerificationWaveDenominatorSeal`、`CampaignCoverageDenominatorSeal`、`CapabilityAssessmentSetSeal`、`ShadowEvaluationCensusSeal`与`CampaignDispatchHoldSnapshot`由Plan C Task 3定义。后续Task不得改成近似名称（如`ToolTruthReconciliationState`、`CampaignCoverageSeal`、`RolloutSafetyHold`）、重定义plan或让caller构造裸struct。

真实admission只能在Plan A `with_checked_tool_truth_authority_bundle` callback内执行：同一request按server-derived relevant-root census为全部stage roots+derived receipts创建stable snapshots，在同一DB transaction seal各authority set与bundle；只有private all-fresh conversion成功才调用本函数并立即persist Campaign admission。persisted admission冻结`tool_truth_authority_bundle_seal_id / relevant_root_set_hash / member_set_hash / semantic_authority_bundle_hash / freshness_attestation_bundle_hash / temporal_validity_bundle_hash`以及Plan B snapshot epoch/policy hash；返回authority带`'guard`生命周期，不能逃出callback或被缓存。dispatch前仍重读current semantic/temporal heads与Plan C quarantine；若任一root已orphan/expired或quarantine compound尚未形成，直接HOLD/create-or-wait，绝不依赖旧Campaign row继续执行。

`CampaignDispatchHoldSnapshot`至少冻结`campaign_dispatch_held / campaign_dispatch_generation / row_version / reason_code / read_at`；`authorize_campaign_execution`只接受`held=false`并把exact campaign-dispatch generation写入`CampaignExecutionAuthority`、Prepared Action private manifest与authorization receipt。generation在该scope每次on/off transition单调递增，不能由updated timestamp、全局row version或另一scope generation代替；之后任何新业务send都必须重读`held=false`并精确匹配`campaign_dispatch_generation`，避免hold曾开启又关闭后旧授权复活。全局row version只用于管理CAS/审计，不是per-send authority，因此不相关的operation-admission变化不会误杀在途action。

`PersistedOperationContractSnapshot`只能由Plan B `operation_rollout` repo从`operation_state`构造，字段包含frozen Tool Truth contract、Investigation contract/mode和joint rank；其字段私有，不能由Agent、command request或调用方拼装policy。`select_campaign_route`只读取这个persisted snapshot，先重算合法joint rank，再调用`operation_contract.investigation_mode().policy()`，禁止接收caller-provided `InvestigationModePolicy`。

`CampaignRoute`闭集为`LegacyPath / ShadowEvaluationOnly / AuthoritativeCandidate`。rank 0/1直接返回LegacyPath，不加载Registry generation；rank 2–4只返回ShadowEvaluationOnly，不创建canonical Campaign/coverage；rank 5/6且policy canonical才返回AuthoritativeCandidate，但这还不是执行授权。`authorize_shadow_evaluation`只验证server-sealed旧terminal artifact census，返回的authority type graph不含任何外部/provider/LLM/network/browser/shell/Authorization Broker/credential/executor/action journal/lease/budget port。`authorize_campaign_execution`只接受AuthoritativeCandidate，并要求`receipt_v1 + multi-root bundle全semantic-consistent/temporally-fresh且与Plan B snapshot epoch/policy exact + verification-plan/generation/wave/campaign denominator exact seals + capability assessment exact set + campaign_dispatch hold off`，才返回lifetime-bound AuthoritativeCampaign authority。legacy operation不读取campaign hold，`operation_admission_held`也不在此函数解释。

route guard在任何consult census、Prepared Action、provider/adapter或LLM dispatch前运行；shadow evaluator只能对已冻结本地snapshot做纯计算，不能产生外部调用或目标侧副作用。DB read error、persisted pair/rank不一致、assessment set缺/额外/非latest或seal/hash漂移全部fail closed。

**Step 3：运行 GREEN**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(verification_campaign_rollout_)' --status-level fail)
```

Expected: 五种 mode 的读、写、执行允许集合逐格通过；没有逐字段 fallback。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/verification_campaign/tests.rs backend/crates/golish-core/src/investigation_contract.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/rollout.rs
git commit -m "feat(verification): freeze campaign rollout authority"
```

## Task 3：新增 append-only Campaign persistence

**授权暂停点：** 开始本 Task 前再次向用户确认允许新增 `20260729000007_verification_campaigns.sql` 和 `golish-db` schema。未确认时停止执行，不能用 JSON column 或旧表滥写绕过。

**Files:**

- Create: `backend/crates/golish-db/migrations/20260729000007_verification_campaigns.sql`
- Create: `backend/crates/golish-db/src/repo/{verification_campaigns.rs,verification_prepared_actions.rs,verification_oracles.rs,verification_fact_delta_bundles.rs,verification_campaign_coverage.rs,verification_capability_assessments.rs,verification_campaign_shadow.rs,hypothesis_objective_outcomes.rs,hypothesis_revision_adjudications.rs}`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Create: `backend/crates/golish-db/tests/verification_campaigns.rs`

**Step 1：先写 migration/repo RED**

测试必须覆盖：跨project/org/target拒绝、同revision+objective+contract最多一个active Campaign、round ordinal/row version CAS、consult exact census、单active Prepared Action、authorization/execution/oracle条件义务、response-loss stable request id replay、objective closeout atomicity、FactDelta exact-one、late result superseded witness；还要覆盖verification plan/proof-path exact seal、多个objective outcome的revision adjudication、单Campaign不能终结revision、capability assessment append-only latest/exact seal、adapter_missing/policy_denied/prerequisite_missing residual、Campaign member绑定exact assessment、Wave四态coverage exact union、superseded Campaign version复用wave member不触发全局唯一冲突；`explicit_no_control`仍产生denominator/oracle member且只能配`control_validity=not_required`；四层budget跨Campaign并发oversubscription、unknown-held与known settlement；server-derived conflict-key partial overlap跨Campaign/跨Hypothesis互斥、stale fence；ConcurrentActionGroup exact subaction set/barrier/receipt；authority quarantine compound与append-only lineage。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test verification_campaigns --status-level fail)
```

Expected: migration relation、coverage exact-set、shadow port isolation、terminal atomicity tests 因 `00007` 和 repo 尚不存在而失败；Plan B schema tests 仍通过。

**Step 2：在一个 forward-only migration 内新增实体**

最少包含：

- `verification_campaigns`
- `verification_campaign_rounds`
- `verification_consults`
- `verification_strategy_artifacts`
- `verification_strategy_obligations`
- `verification_prepared_actions`
- `verification_prepared_action_group_members`
- `verification_prepared_action_authorizations`
- `verification_action_executions`
- `verification_action_subexecutions`
- `verification_action_conflict_sets`
- `verification_action_conflict_set_members`
- `verification_conflict_key_heads`
- `verification_conflict_key_events`
- `verification_budget_contracts`
- `verification_budget_contract_axes`
- `verification_budget_scope_heads`
- `verification_budget_reservations`
- `verification_budget_ledger_entries`
- `verification_cleanup_obligations`
- `verification_callback_obligations`
- `verification_oracle_assessments`
- `verification_oracle_census_seals`
- `verification_oracle_census_members`
- `verification_campaign_adjudications`
- `verification_campaign_terminal_decisions`
- `hypothesis_objective_outcome_receipts`
- `hypothesis_objective_claim_component_outcome_seals`
- `hypothesis_objective_claim_component_outcome_members`
- `hypothesis_objective_outcome_heads`
- `hypothesis_objective_outcome_set_seals`
- `hypothesis_objective_outcome_set_members`
- `hypothesis_revision_adjudications`
- `hypothesis_revision_terminal_decisions`
- `verification_fact_delta_bundles`
- `fact_delta_consumptions`
- `hypothesis_evolution_proposals`
- `hypothesis_evolution_decisions`
- `hypothesis_consolidation_batches`
- `hypothesis_consolidation_receipts`
- `hypothesis_fixed_point_receipts`
- `enrichment_obligations`
- `application_fact_refinement_obligations`
- `verification_wave_coverage_denominators`
- `verification_wave_coverage_members`
- `verification_campaign_coverage_denominators`
- `verification_campaign_coverage_members`
- `verification_campaign_coverage_results`
- `verification_campaign_coverage_receipts`
- `verification_wave_coverage_receipts`
- `verification_campaign_shadow_evaluations`
- `verification_campaign_shadow_evaluation_items`
- `verification_capability_assessments`
- `verification_capability_assessment_set_seals`
- `verification_capability_assessment_set_members`
- `verification_wave_unassigned_coverage_results`
- `verification_authority_quarantine_events`
- `verification_authority_quarantine_members`
- `verification_authority_temporal_staleness_events`
- `hypothesis_re_adjudication_obligations`
- `verification_authority_correction_bundles`
- `verification_authority_correction_consumptions`
- `verification_campaign_safety_holds`（singleton默认held，Plan C无生产release setter；Plan D local-admin rollout协调器原位接管）

capability assessment是Campaign admission的持久authority，不是compiler临时返回值。`verification_capability_assessments`至少包含：`id, operation/project/organization, hypothesis_revision_id, verification_objective_id, verification_contract_hash, capability_key, capability_contract_version/hash, policy_snapshot_id/hash, assessment_ordinal, supersedes_assessment_id?, status, reason_code?, residual_id?, adapter_contract_version/digest?, source_snapshot_hash, assessed_at`。`status`闭集只有`unassessed|available|adapter_missing|policy_denied|prerequisite_missing`；只有`available`允许完整adapter identity，其他状态必须有typed reason/residual且不能生成Prepared Action。复合唯一键冻结`revision + objective + verification contract + capability contract + policy snapshot + ordinal`；current只按max ordinal/explicit successor读取，历史row不更新/删除。

每次compiler/policy评估后写`verification_capability_assessment_set_seals`与ordered members，count/hash必须与该revision/objective/contract/policy下latest assessment exact-equal；缺项、额外项、旧ordinal或hash漂移都不能admit Campaign。每个Wave/Campaign denominator member保存exact `capability_assessment_id`（即使状态不可用也保留obligation并转residual），不能只保存capability字符串。assessment、set seal/member均append-only，跨operation/project/org使用compound FK，并通过Plan B typed outbox entity/invalidation catalog投影。

在首个Campaign admission前，host必须消费Plan B `00006`已经sealed的唯一`HypothesisVerificationPlanV1`，不能让Lead或单个Campaign决定“验证到什么程度算整条假设成立”。唯一authority是Plan B的`attack_hypothesis_verification_plans / ..._plan_paths / ..._plan_path_members`（名称以Plan B最终migration冻结为准），它绑定operation/project/org、generation、revision、source snapshot、plan contract/version、ordered proof paths、objective/VerificationContract exact set、host-derived `HypothesisClaimComponentV1` clauses/impact qualifiers与`path_falsifier`。Plan C `00007`只用compound FK读取，禁止复制、重seal或创建第二套plan表/DTO。所有revision objective和required claim components必须由每条path按B contract exact覆盖；默认单path覆盖全部required components，只有Plan B host compiler显式声明替代路径时才产生多path。

每个Campaign只服务一个plan member引用的objective/contract及其claim-component subset。Campaign denominator、oracle census和terminal compound都保存component id/hash exact bindings；closeout先seal `hypothesis_objective_claim_component_outcome_seal/members`，每member compound绑定Plan B claim component id/hash、对应predicate/oracle/coverage members与`proof|refutation|inconclusive|blocked|unassigned|invalidated`。随后terminal产生append-only `hypothesis_objective_outcome_receipt`，保存objective-local `proof|refutation|inconclusive|blocked|exhausted_with_residuals|unassigned|invalidated`、claim-component outcome seal id/hash、monotonic outcome ordinal/predecessor、campaign adjudication/terminal/coverage/oracle/FactDelta refs与source authority。没有Campaign的objective/component仍必须由Wave consolidation写`unassigned + exact residual` outcome。每个plan objective有唯一CAS `hypothesis_objective_outcome_head`，只有expected predecessor匹配且新Campaign仍是current version时才能推进；late/superseded terminal只保存witness，不能覆盖head。

revision adjudicator不接受caller提交的outcome ids。它按objective canonical order锁全部heads，排除superseded/quarantined authority，并在同一transaction写`hypothesis_objective_outcome_set_seal/members`冻结截至cutoff的唯一latest eligible exact set；缺head或任一plan objective不exact-equal均不能adjudicate（没有Campaign的member必须先由Wave写`unassigned + residual` head）。随后逐path计算`proved/falsified/unresolved`，再写`hypothesis_revision_adjudications`：任一proof path全proof则`verified`；否则只有每条path都有被标为`path_falsifier`的valid refutation才`refuted`；其余写`nonterminal + unresolved objective exact set`。非winning path或已falsified path的未决member仍写coverage/residual，但不覆盖上述存在/全称量词。只有verified/refuted adjudication可在同一事务创建`hypothesis_revision_terminal_decisions`、Finding/refutation lineage和Plan B state event，且同一revision terminal transition至多一次；单Campaign terminal FK、挑选旧outcome、Finding suggestion或Agent prose不能满足Plan B DEFERRABLE guard。

revision adjudicator还必须从锁定后的latest objective outcome lineage由服务端推导全部relevant Tool Truth roots，并进入Plan A `with_checked_tool_truth_authority_bundle` callback；只有private conversion得到同一guard生命周期内的`AllFreshToolTruthAuthorityBundle<'guard>`才可继续。adjudication row必须持久化`tool_truth_authority_bundle_seal_id / relevant_root_set_hash / member_set_hash / semantic_authority_bundle_hash / freshness_attestation_bundle_hash / temporal_validity_bundle_hash / temporal_census_hash / temporal_policy_hash / target_epoch_set_hash / observation_window_start / observation_window_end / effective_valid_until`。objective之间的epoch、observation window与max-skew由host从exact member census重算；caller不能提交root list、旧seal、TTL、future time或“已检查fresh”布尔值。这样每次verified/refuted既可重放也可证明当时所有leaf source在同一DB-clock cutoff上all-fresh，而不是只在Rust内检查一次当前时间。

所有Plan C-owned verification header/member exact set统一用一个DB lifecycle：capability assessment set、Wave denominator、Campaign denominator、consult census、strategy-obligation manifest、action conflict set、oracle census、objective claim-component outcome set与objective-outcome set均先插`sealed_at=NULL` header，再按canonical ordinal插members，由DB/host重算count/hash，最后仅允许`NULL -> sealed timestamp`；Plan B-owned `HypothesisVerificationPlanV1`必须已经走完其同型seal lifecycle，Plan C只校验sealed FK/hash。authoritative reader/admission/authorization/adjudicator只读sealed header；trigger拒绝sealed header修改/删除、member修改/删除与post-seal INSERT，两个并发sealer只有同request+同payload exact replay可复用。direct-SQL tests逐类覆盖unsealed consume、post-seal append/hash drift、漏/重/替换claim component/impact qualifier与并发authority变化。

budget不是单个Action的自报整数，而是operation→wave→campaign→action四层同轴上限。`verification_budget_contracts/axes`冻结scope identity、parent contract、Plan A同一closed axis enum、limit与contract hash；每个child limit不得大于parent。`verification_budget_scope_heads`是唯一允许CAS更新的计数头，逐axis保存`consumed + reserved + unknown_held`，其余contract/reservation/ledger rows append-only。`begin_authorized_action`按operation→wave→campaign→action固定顺序锁全部ancestor heads，同时为Prepared Action静态upper bound在四层reserve；任一层任一axis超限则整个begin rollback，不能只检查Campaign局部余额。每次真实send/chunk/deadline/retry都把actual计入已reserve额度并写ledger；known closeout按Plan A observed actual结算，cleanup完成后才释放未用reservation。`outcome_unknown`或cleanup未知必须把最坏upper bound转为`unknown_held`，不能释放后让并发Campaign复用；只有typed recovery/manual resolution能settle。scheduler开启新round/wave前重读同一ledger receipt和remaining exact set。并发测试必须证明两个Campaign各自不过限但合计超过operation ceiling时exact-one成功，另一个在外部调用前拒绝；unknown reservation持续占额，known closeout才释放未用部分。

conflict authority也由server从Prepared Action派生，不接收模型字符串，但不能把整个member set只hash成一个domain：`{A,B}`与`{B,C}`必须因共享`B`冲突。`verification_action_conflict_sets/members`冻结每个action需要的canonical conflict-key exact set；key至少包括target/rate-limit bucket、credential/session、每个mutable resource和control fixture。`verification_conflict_key_heads`以`operation/org + key_kind + key_identity_hash`为独立CAS head，保存`free|active|recovery_hold`、owner Campaign/Prepared Action、monotonic fencing token、expiry与row version；每次transition追加`verification_conflict_key_events`并通过Plan B typed outbox投影。`begin_authorized_action`与四层budget reserve同一transaction按`key_kind + key_hash`排序获取**全部**heads并分配每key fence；任一key不可得则全体rollback，避免partial lease/deadlock。不同credential若写同一resource仍在resource key碰撞；只有adapter contract能证明commutative/read-only的resource可显式省略。known closeout且cleanup完成才按同一exact set release；`outcome_unknown`、expired in-flight或cleanup未知把全部keys转`recovery_hold`，绝不因时钟到期自动重用。所有adapter写入携带相关key fences，stale owner不能close/release新lease。测试覆盖partial-overlap、same-resource/different-role、canonical multi-key ordering、跨Campaign/跨Hypothesis、member/hash伪造、expiry→recovery_hold、stale fence与safe all-set release。

Prepared Action采用tagged contract：`single_action_v1`或`concurrent_action_group_v1`。group header冻结2..N个`verification_prepared_action_group_members`（ordinal、canonical request hash、credential/session binding、barrier cohort、expected start-window、per-subaction upper budget、oracle role）；JIT review一次展示整个group及所有副作用，不允许只批准其中一项。`begin_authorized_action`对union conflict-key set和aggregate upper budget原子reserve，写group durable begin后才允许事务外启动；executor在host barrier下按bounded max concurrency释放，并为每个member写exact-one `verification_action_subexecution`及Plan A capability receipt。group closeout要求member exact census、start-window observation、每subaction known/unknown状态、cleanup与budget settlement完整；任何缺member/response loss都保持outcome_unknown/held。concurrency oracle只能消费该sealed group和subexecution exact set，使用versioned relation（例如exact-one success、double-success、stale-read/write ordering）判定，不能拿时间接近的两次普通Action拼成race proof。首版没有instrumented adapter的race objective必须由capability assessment写`adapter_missing`、Wave/Campaign residual和`coverage_sufficiency=not_assessed`。

关键字段最少冻结为：

```text
verification_budget_contracts
  id, operation/project/org, scope_kind operation|wave|campaign|action,
  scope_id, parent_contract_id?, contract_version/hash, sealed_at
verification_budget_contract_axes
  contract_id, axis_kind(Plan A closed enum), limit, ordinal
verification_budget_scope_heads
  contract_id, axis_kind, consumed, reserved, unknown_held, row_version
verification_budget_reservations
  id, prepared_action/authorization/execution identity, contract-set hash,
  upper-bound membership hash, state active|settled|unknown_held
verification_budget_ledger_entries
  reservation_id, ancestor_contract_id, axis_kind,
  entry_kind reserve|consume|settle|hold_unknown|release, delta, resulting_head_hash, fence

verification_action_conflict_sets
  id, prepared_action_id UNIQUE, member_count/hash, sealed_at
verification_action_conflict_set_members
  set_id, ordinal, key_kind target_rate_limit|credential_session|resource|control_fixture,
  key_identity_hash, adapter_commutativity_authority_hash?
verification_conflict_key_heads
  operation/project/org, key_kind, key_identity_hash,
  state free|active|recovery_hold, owner_campaign_id?, owner_prepared_action_id?,
  fencing_token, expires_at?, row_version,
  PRIMARY KEY(operation_id,organization_id,key_kind,key_identity_hash)
verification_conflict_key_events
  key head identity, event_ordinal, acquire|renew|recovery_hold|release,
  expected/new fence, owner identity, reason/residual, event_hash
```

Campaign adjudication还必须引用一份host-sealed oracle census。`verification_oracle_census_seals`绑定`campaign_id + VerificationContractV1 hash + campaign denominator/result hash + expected member count/hash`；`verification_oracle_census_members`逐predicate component/control-binding/coverage obligation保存ordinal、typed identity、`assessed|untested|blocked` disposition，以及exact-one `oracle_assessment_id`或`residual_id`。control binding是tagged exact-one：`required(control_member_id,control_contract_hash)`或`explicit_no_control(no_control_marker_hash)`；后者仍产生census/denominator member，不能因controls为空而消失或vacuously verify。member集合必须与host-owned contract components/control bindings和sealed Campaign denominator exact-equal；模型不能添加、删除、排序或自报hash。`verification_campaign_adjudications`保存`oracle_census_seal_id/hash`，terminal transition必须重读并复算；缺/额外/重复member、旧oracle revision或hash drift一律HOLD。

terminal并非“一旦写入就永远有权威”。Plan A workspace artifact每次fresh rehash后可能把原本consistent receipt判为orphan；Plan C因此建立独立append-only authority quarantine，而不是UPDATE旧terminal/Finding/FactDelta。`verification_authority_quarantine_events`绑定latest invalid semantic reconciliation/freshness authority、原Campaign terminal、objective outcome、coverage/oracle census、FactDelta bundle，以及所有反向引用该outcome的revision adjudication/terminal decision/Finding或refutation lineage；ordered members对这些typed refs做exact set。若原`fact_delta_consumptions`已是`applied`，旧row保持原样，绝不改成quarantined；同一compound另写`verification_authority_correction_bundles/consumptions`，以显式supersession/retraction delta让Registry在下一generation撤销由失效authority导入的support/contradiction，并保留完整因果链。

`quarantine_campaign_authority`在一个canonical transaction内：重读Plan A latest invalid semantic reconciliation与fresh token、锁terminal/objective head及引用它的revision authorities、append quarantine/event members、把current objective head推进为host-derived `invalidated + residual` successor、创建exact correction bundle（如已消费），并对所有引用该leaf outcome的旧aggregate adjudication/terminal/Finding/refutation/report source追加invalidation/supersession。因为Plan B revision terminal transition是immutable且至多一次，quarantine **不得**在同一revision同步再写第二个verified/refuted decision，也不能用另一proof path让旧Finding“继续有效”；它必须创建open re-adjudication/consolidation obligation。随后outer loop生成H(g+1) successor和新的B-owned plan，才能基于剩余current valid outcomes按正常C adjudicator重新verified/refuted。最后按实际受影响exact set写Plan B typed `Invalidate/Close` outbox batch（CampaignTerminal/ObjectiveOutcome/HypothesisRevisionAdjudication/Coverage/FactDelta/Hypothesis/Reporting dependency）；任何一步失败全部rollback。旧artifact、terminal、Finding/refutation、FactDelta与consumption bytes都不删除不改写。所有canonical reader、Wave consolidation、Reporting finalizer在使用terminal前强一致查quarantine及aggregate invalidation；projection lag只影响展示，不能让已quarantine leaf或其旧aggregate authority继续作为current。重复检测同一invalid reconciliation exact replay原quarantine event；new invalid reconciliation ordinal/version产生新event lineage。

TTL到期与semantic orphan/tamper必须分开。host强一致selector对每个objective outcome、revision adjudication、terminal decision、Finding/refutation派生闭集`current_authority_status = authoritative | temporally_stale | semantically_invalid`：到`effective_valid_until`后历史row仍可按`observed_as_of`审计，但立即从current authority、Gate、Finding current selector和Reporting current source中剔除；同一transaction追加`verification_authority_temporal_staleness_event`、Plan A revalidation obligation引用及`hypothesis_re_adjudication_obligation`，不写quarantine/retraction。revalidation即使得到same-semantic新receipt也只能供H(g+1)的新objective outcome exact set与新revision adjudication使用，绝不能改旧row、延长旧`effective_valid_until`或“复活”旧terminal/Finding。只有semantic invalid才走上一段quarantine/correction链。测试固定terminal后TTL到期、same-semantic refresh、semantic orphan三条互斥路径及Workspace/current selector结果。

`hypothesis_residual_risks` 已由 Plan B `00006` 创建并继续作为唯一 residual ledger；`00007` 不得再次 `CREATE TABLE`。migration test 必须断言 relation 只有一份，并验证 Campaign FK/typed writer复用它。Plan B 也已经把 `investigation_projection_changes.entity_kind` 冻结为 A–D 完整词汇；`00007` 不替换该 CHECK，只做 catalog test证明 Campaign/coverage/fact-delta kind均可由同一 outbox helper写入。

Plan B `00006`已安装revision↔exact-one creating state-event的DEFERRABLE authority trigger，并在B阶段明确禁止普通writer创建`verified/refuted`。`00007`只能以向后兼容方式扩展这一个trigger：verified/refuted state event必须在同一transaction exact引用sealed `HypothesisVerificationPlanV1`、与plan objective exact-equal的objective-outcome set、host-derived revision adjudication、revision terminal decision及对应Finding/refutation lineage与operation/project/org compound authority；单个Campaign terminal/adjudication/oracle/coverage receipt永远不足。`invalid`仍只接受Plan B server-validator receipt，其他origin继续拒绝。不得disable/drop后不恢复、不得新建第二套terminal trigger。clean/upgraded migration fixture、普通Candidate writer绕过、单Campaign提前终态、伪造outbox snapshot route和revision terminal compound rollback都要覆盖。

关键唯一约束以 partial unique index/compound FK 表达，而不是只靠 Rust：

```sql
CREATE UNIQUE INDEX verification_campaigns_one_active_contract
ON verification_campaigns(hypothesis_revision_id, verification_objective_id, verification_contract_hash)
WHERE terminal_at IS NULL AND superseded_at IS NULL;

CREATE UNIQUE INDEX verification_prepared_actions_one_active_lane
ON verification_prepared_actions(campaign_id)
WHERE state IN ('pending_authorization', 'authorized', 'started', 'outcome_unknown');

CREATE UNIQUE INDEX verification_action_executions_one_per_ordinal
ON verification_action_executions(prepared_action_id, authorization_receipt_id, execution_ordinal);

CREATE UNIQUE INDEX verification_fact_delta_one_per_terminal
ON verification_fact_delta_bundles(campaign_terminal_decision_id);
```

coverage必须把“是否测到”与“测出的认识结论”拆成正交轴：

- `coverage_disposition`闭集只有`tested_complete|tested_degraded|untested|blocked`；
- `epistemic_outcome`闭集为`proof|refutation|inconclusive|not_assessed`，只来自typed oracle/adjudicator；
- `control_validity`闭集为`valid|invalid|not_assessed|not_required`，不能拿coverage状态代替control；`not_required`只允许`explicit_no_control` binding，required control绝不能借此绕过评估。

denominator在首个Prepared Action authorization前sealed；member是`hypothesis_revision + verification objective + predicate component + tagged control binding + exact capability assessment + expected action/oracle`的exact obligation，不从实际action反推。`required`保存control member ID/hash；`explicit_no_control`保存contract marker/hash，二者exact-one且都必须生成member。`tested_complete/tested_degraded`都必须同时引用Prepared Action、Plan A capability receipt和oracle；`tested_degraded`还必须有exact residual。`untested/blocked`必须有exact residual且三个执行引用全部为NULL。Campaign receipt只统计四种coverage disposition，其总和必须等于member count；proof/refutation等只在独立adjudication census统计，不能混进coverage count。

coverage authority 的关键字段从 migration 起冻结：

```text
attack_hypothesis_verification_plans / paths / path_members (Plan B 00006, read-only FK source)
  id, operation/project/org, generation/revision, plan contract/hash,
  ordered proof paths, exact claim-component + objective + VerificationContract members,
  path_falsifier, sealed_at
hypothesis_objective_claim_component_outcome_seals / members
  id, plan_id, objective_id, member_count/hash, sealed_at;
  seal_id, ordinal, claim_component_id/hash, predicate/oracle/coverage member refs,
  component_outcome proof|refutation|inconclusive|blocked|unassigned|invalidated,
  member_hash
hypothesis_objective_outcome_receipts
  id, plan_seal_id, objective_id, outcome_ordinal, predecessor_outcome_id?,
  outcome proof|refutation|inconclusive|blocked|exhausted_with_residuals|unassigned|invalidated,
  campaign_terminal_id?, campaign_adjudication_id?, campaign_coverage_receipt_id?,
  claim_component_outcome_seal_id/hash, fact_delta_bundle_id?, residual_id?, source_authority_hash
hypothesis_objective_outcome_heads
  plan_seal_id, objective_id, current_outcome_id, current_ordinal, row_version
hypothesis_objective_outcome_set_seals / members
  id, plan_seal_id, cutoff/head-set hash, member_count/hash, sealed_at;
  seal_id, objective_id, selected_current_outcome_id/ordinal/hash
hypothesis_revision_adjudications
  id, plan_seal_id, objective_outcome_set_seal_id/hash,
  tool_truth_authority_bundle_seal_id, relevant_root_set_hash, member_set_hash,
  semantic_authority_bundle_hash, freshness_attestation_bundle_hash,
  temporal_validity_bundle_hash, temporal_census_hash, temporal_policy_hash,
  target_epoch_set_hash, observation_window_start/end, effective_valid_until,
  outcome nonterminal|verified|refuted, unresolved_set_hash?, adjudication_hash
hypothesis_revision_terminal_decisions
  id, revision_adjudication_id UNIQUE, decision verified|refuted,
  finding_lineage_id?, refutation_lineage_id?, state_event_id, decision_hash

verification_wave_coverage_denominators
  id, operation/project/org, generation_seal_id, contract_version,
  source_snapshot_hash, member_set_hash, member_count, sealed_at
  UNIQUE(generation_seal_id)

verification_wave_coverage_members
  id, denominator_id, semantic_key, input_ref_kind/id/identity_hash,
  claim_component_id/hash, objective_id, predicate_component_id,
  control_binding_kind required|explicit_no_control,
  required_control_id?, required_control_hash?, no_control_marker_hash?,
  capability_assessment_id, expected_capability/action/oracle kind
  UNIQUE(denominator_id, semantic_key)

verification_campaign_coverage_denominators
  id, operation/project/org, campaign_id, hypothesis_revision_id,
  wave_denominator_id, contract_version, source_snapshot_hash,
  member_set_hash, member_count, sealed_at
  UNIQUE(campaign_id)

verification_campaign_coverage_members
  id, denominator_id, wave_coverage_member_id, semantic_key,
  claim_component_id/hash, obligation_kind, capability_assessment_id,
  expected_capability/action/oracle kind
  UNIQUE(denominator_id, semantic_key), UNIQUE(denominator_id,wave_coverage_member_id)

verification_campaign_coverage_receipts
  id, campaign_id, campaign_terminal_decision_id UNIQUE, denominator_id,
  denominator/result/residual membership hashes,
  tested_complete/tested_degraded/untested/blocked counts,
  coverage_status complete|partial|invalid, created_at

verification_campaign_coverage_results
  coverage_receipt_id, coverage_member_id, coverage_disposition,
  epistemic_outcome, control_validity valid|invalid|not_assessed|not_required,
  prepared_action_id?, capability_execution_receipt_id?, oracle_assessment_id?, residual_id?
  PRIMARY KEY(coverage_receipt_id, coverage_member_id)

verification_wave_unassigned_coverage_results
  wave_coverage_receipt_id, wave_coverage_member_id, residual_id,
  disposition untested|blocked, result_hash
  PRIMARY KEY(wave_coverage_receipt_id,wave_coverage_member_id)
```

Wave denominator必须在首个Campaign admission前冻结；Campaign denominator是某个Campaign version对Wave成员的immutable partition，并在首个Prepared Action authorization前冻结。历史/superseded Campaign version可引用同一Wave member，因此禁止全局`UNIQUE(wave_coverage_member_id)`；只在单个Campaign denominator内唯一。Wave consolidation选择current terminal Campaign denominator/receipt exact set，然后校验`all wave members = disjoint union(selected campaign members, verification_wave_unassigned_coverage_results)`，每个Wave member在本次Wave receipt中exact-one。新事实产生的新objective进入下一generation/Wave，不修改sealed denominator。Reporting只可消费最终`verification_wave_coverage_receipt`，不能拿任意局部Campaign receipt冒充整代coverage。

Plan C 同时创建 safety-hold singleton 的精确可变头，只有 Plan D 的本地管理协调器能在未来写入；C 只提供 read/guard：

```sql
CREATE TABLE verification_campaign_safety_holds (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    campaign_dispatch_held BOOLEAN NOT NULL DEFAULT TRUE,
    operation_admission_held BOOLEAN NOT NULL DEFAULT FALSE,
    campaign_dispatch_generation BIGINT NOT NULL DEFAULT 0 CHECK (campaign_dispatch_generation >= 0),
    operation_admission_generation BIGINT NOT NULL DEFAULT 0 CHECK (operation_admission_generation >= 0),
    row_version BIGINT NOT NULL DEFAULT 0 CHECK (row_version >= 0),
    reason_code TEXT NOT NULL DEFAULT 'initial_rollout_hold' CHECK (BTRIM(reason_code) <> ''),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO verification_campaign_safety_holds(singleton) VALUES (TRUE);
```

初始状态只hold新Campaign external dispatch，不阻止rank 0 legacy operation创建；`operation_admission_held`仅供Plan D紧急止血显式开启。每个scope on/off transition只递增自己的generation并同时CAS全局row version；Campaign authorization/per-send绑定`campaign_dispatch_generation`，operation create/fork绑定`operation_admission_generation`。Campaign guard读取前者，operation creation只读取后者，禁止把一个默认true布尔或不相关scope变化同时解释成两种authority。

shadow evaluation表与canonical Campaign/action/execution表没有可执行FK。evaluation只保存frozen snapshot/obligation census；item保存compiled semantic signature、legacy capability receipt ref、deterministic oracle replay ref与Plan B `investigation_projection_compare_samples.comparison_id`（每item exact-one）。它不得自行保存第二份match/mismatch hash/diff truth；`match|mismatch|incomplete|authority_corrupt`只能从Plan B `compare_and_record_v1`写入的sample读取。表不能持有authorization token、credential、lease、budget reservation、Finding或FactDelta ref。

所有新 row 都带 `operation_id / project_id / organization_id` 或通过不可绕过的 compound FK 继承 exact ownership；对 live target 同时保存 at-time target identity/hash，删除 live row 后仍可审计但不得重新 dispatch。除明确列出的`verification_budget_scope_heads / verification_conflict_key_heads / hypothesis_objective_outcome_heads / verification_campaign_safety_holds` CAS heads外，contract/member/event/receipt/reservation/quarantine/correction表全部DB append-only；每个objective head推进必须同事务写新append-only outcome receipt、expected predecessor/ordinal与完整typed outbox，late/superseded outcome不得成为current。所有heads变化都要求expected row version、typed event/outbox，禁止无审计setter。

`verification_action_executions` 是新 Campaign 的 action journal，不复制 Tool Truth：每个 durable-begin row 在 closeout 后必须引用恰好一个 Plan A `capability_execution_receipt`；raw witness、typed landing、business/evidence persistence 和 actual budget 仍只以该 receipt 为执行真值。旧 `candidate_attempt_actions` 保持 legacy-only，不能被新 Campaign 直接写入。

**Step 3：实现短事务 repo compounds**

提供 server-owned 方法：

- `seal_wave_coverage_denominator`
- `record_capability_assessment`
- `seal_capability_assessment_set`
- `admit_campaign`
- `open_round_with_consult_census`
- `record_strategy_decision`
- `persist_compiled_prepared_action`
- `seal_action_conflict_set`
- `decide_prepared_action_authorization`
- `begin_authorized_action`
- `record_action_subexecution`
- `closeout_action_execution`
- `record_action_oracle`
- `seal_oracle_census`
- `seal_objective_claim_component_outcomes`
- `close_campaign_objective_with_fact_delta`
- `seal_hypothesis_objective_outcome_set`
- `adjudicate_hypothesis_revision_with_fresh_tool_truth`
- `record_fact_delta_consumption`
- `quarantine_campaign_authority`
- `record_authority_correction_consumption`

其中`begin_authorized_action`必须在Plan A fresh-authority guard callback内，把四层budget reserve、canonical all-key lease/fences与single/group durable begin封成不可拆的同一transaction；`closeout_action_execution`把known actual settlement/unknown-held、group subexecution exact census、cleanup状态与all-key release/recovery-hold封成同一compound，caller不能先释放其中一项。`close_campaign_objective_with_fact_delta`只校验objective-local adjudication/evidence/control/completeness并原子写Campaign terminal、objective outcome、coverage、residual和exact-one FactDelta，**不创建Finding、不写revision verified/refuted**。`adjudicate_hypothesis_revision_with_fresh_tool_truth`由repo从latest outcome lineage推导relevant roots，在Plan A guard callback内调用module-private `adjudicate_hypothesis_revision_on(tx, &AllFreshToolTruthAuthorityBundle, ...)`；它重验sealed plan与objective outcome exact set后，才在verified/refuted时创建或复用Finding/refutation、revision terminal receipt与Plan B state event，nonterminal只写unresolved adjudication。`quarantine_campaign_authority`按上文写完整失效/纠正/outbox compound。外部执行不在这些事务中。

**Step 4：运行 GREEN focused DB tests**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db --test verification_campaigns --status-level fail)
```

Expected: positive、CAS conflict、stale authority、cross-org、response-loss replay、terminal rollback 和 conditional-artifact cases 全绿；旧 Candidate/Attempt rows 不被 migration 回填为新 verdict。

### Future Commit

```bash
git add backend/crates/golish-db/migrations/20260729000007_verification_campaigns.sql backend/crates/golish-db/src/repo/verification_campaigns.rs backend/crates/golish-db/src/repo/verification_prepared_actions.rs backend/crates/golish-db/src/repo/verification_oracles.rs backend/crates/golish-db/src/repo/verification_fact_delta_bundles.rs backend/crates/golish-db/src/repo/verification_campaign_coverage.rs backend/crates/golish-db/src/repo/verification_capability_assessments.rs backend/crates/golish-db/src/repo/verification_campaign_shadow.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/verification_campaigns.rs
git commit -m "feat(db): persist verification campaign authority"
```

## Task 4：打通 typed repository seam 与 app bridge

**Files:**

- Create: `backend/crates/golish-agent-kit/src/db_traits/verification_campaign.rs`
- Modify: `backend/crates/golish-agent-kit/src/db_traits/{mod.rs,repo.rs}`
- Create: `backend/crates/golish-agent-app/src/ai/db_bridge/verification_campaign.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- Test: inline `#[cfg(test)]` modules in both new `verification_campaign.rs` files

**Step 1：写 default-unavailable RED**

证明旧/mock repository 未实现新 seam 时返回 stable `verification_campaign_repository_unavailable`，且 scheduler 在任何 provider/adapter dispatch 前停止。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -E 'test(verification_campaign_repository_)' --status-level fail)
```

Expected: tests 因 Campaign/coverage repo trait、Pg bridge和isolated shadow evaluator seam不存在而失败；legacy repository mocks仍能编译并返回 typed unavailable。

**Step 2：定义窄 trait**

```rust
#[async_trait]
pub trait VerificationCampaignRepository: Send + Sync {
    async fn seal_wave_coverage_denominator(&self, request: SealWaveCoverage) -> RepoResult<WaveCoverageSeal>;
    async fn record_capability_assessment(&self, request: RecordCapabilityAssessment) -> RepoResult<CapabilityAssessment>;
    async fn seal_capability_assessment_set(&self, request: SealCapabilityAssessmentSet) -> RepoResult<CapabilityAssessmentSetSeal>;
    async fn admit_campaign_with_fresh_tool_truth(&self, request: AdmitCampaignRequest) -> RepoResult<CampaignLease>;
    async fn open_round(&self, request: OpenCampaignRound) -> RepoResult<CampaignRound>;
    async fn persist_strategy_decision(&self, request: PersistStrategyDecision) -> RepoResult<()>;
    async fn seal_coverage_denominator(&self, request: SealCampaignCoverageDenominator) -> RepoResult<CampaignCoverageDenominatorSeal>;
    async fn begin_action(&self, request: BeginPreparedAction) -> RepoResult<ActionBeginReceipt>;
    async fn record_action_subexecution(&self, request: RecordActionSubexecution) -> RepoResult<ActionSubexecutionReceipt>;
    async fn closeout_action(&self, request: CloseoutPreparedAction) -> RepoResult<ActionCloseout>;
    async fn recover_unknown_action(&self, request: RecoverUnknownPreparedAction) -> RepoResult<ActionRecoveryCloseout>;
    async fn seal_oracle_census(&self, request: SealOracleCensus) -> RepoResult<OracleCensusSeal>;
    async fn close_campaign_objective(&self, request: CloseCampaignObjective) -> RepoResult<ObjectiveOutcomeReceipt>;
    async fn adjudicate_hypothesis_revision_with_fresh_tool_truth(&self, request: AdjudicateHypothesisRevision) -> RepoResult<HypothesisRevisionAdjudicationReceipt>;
    async fn quarantine_campaign_authority(&self, request: QuarantineCampaignAuthority) -> RepoResult<AuthorityQuarantineReceipt>;
}

#[async_trait]
pub trait VerificationCampaignShadowRepository: Send + Sync {
    async fn open_evaluation(&self, request: OpenShadowEvaluation) -> RepoResult<ShadowEvaluation>;
    async fn record_receipt_replay_and_compare_v1(&self, request: RecordShadowReceiptReplay) -> RepoResult<ComparisonId>;
    async fn close_evaluation(&self, request: CloseShadowEvaluation) -> RepoResult<ShadowEvaluationReceipt>;
}
```

opaque domain DTO不暴露SQL row；adapter canonical args和credentials不经过Agent-facing trait。shadow trait/type graph只能读取已经落库的immutable redacted snapshots和旧capability receipts，不得出现provider/LLM、HTTP/network/DNS、browser、shell/CLI、OAST、Authorization Broker、Executor、credential resolver、action journal、lease或budget reservation handle；compile-time constructor test与上述每类panic/call-count mock共同证明external call总数为0，而不只是“没有目标executor”。`record_receipt_replay_and_compare_v1`必须在同一transaction构造Plan B完整`comparison_record.v1`两侧并调用唯一`investigation_projection::comparison::compare_and_record_v1`；`close_evaluation`只聚合这些immutable comparison IDs，不能实现第二个serializer/comparator。

`admit_campaign_with_fresh_tool_truth`不接受`ReconciliationState/hash/seal id/root list`参数。Pg实现根据request中的stable consumer request id让Plan A host从consumer spec生成relevant-root census并调用`with_checked_tool_truth_authority_bundle`；只有all-fresh conversion成功才在同一guard callback/DB transaction调用module-private `admit_campaign_on(tx, &AllFreshToolTruthAuthorityBundle, ...)`。opaque authority字段只由repo复制进admission row。mock实现也必须通过不可构造的test guard token，避免接口层把P0退化成“调用方保证fresh”或“只选好看的roots”。

`adjudicate_hypothesis_revision_with_fresh_tool_truth`使用同一形状，但relevant roots由repo从锁定的current objective outcome exact set及其execution/oracle/evidence lineage推导，不能复用Campaign admission时的旧bundle或接收caller过滤后的root list。callback内同时seal跨objective temporal census并持久化上文全部bundle/epoch/window/effective-validity字段；任何root expired、mixed epoch、max-skew超限或quarantine pending都只写typed nonterminal/revalidation obligation，不创建terminal decision。

Pg implementation的每个canonical compound必须在同一transaction写Plan B typed `investigation_projection_outbox` batch；Campaign/round/prepared_action/authorization/action_execution/subexecution/oracle/adjudication/campaign_terminal/objective_outcome/coverage/fact_delta/revision_adjudication/revision_terminal/consolidation/capability_assessment使用Plan B已冻结`ProjectionEntityKind / ProjectionChangeKind / TimelineEventKind`及invalidation catalog。canonical tx只写server-redacted immutable snapshot payload与outbox，不写change/entity-version/head；projector按operation-local `source_batch_seq`整batch原子写materialized entity versions + changes并推进head。Campaign closeout的terminal + objective outcome + coverage + FactDelta属于一个完整batch；revision adjudication/terminal/Finding-or-refutation/state-event属于另一个完整batch。任何一项projection失败都不能出现半个batch。禁止commit后best-effort enqueue、裸字符串kind、直接推进head或query拼接尚未projected canonical row。

Task 4还要建立exhaustive producer catalog test：每个会改变Workspace DTO、Gate、open-work或report input的mutation，必须在同一canonical tx发出自身typed entity snapshot，或明确的typed parent aggregate invalidation。至少覆盖`open_round_with_consult_census`、consult terminal、strategy/obligation decision、budget reserve/consume/unknown-hold/settle/exhaust、conflict lease acquire/recovery-hold/release、cleanup/callback open/close、capability assessment、prepared action/auth/execution/oracle/adjudication、coverage、FactDelta、authority quarantine/correction consumption、evolution proposal/decision、enrichment/application-refinement obligation与consolidation/fixed-point。任何repo mutation未登记catalog应令test失败；不能靠前端轮询current canonical table补洞。

**Step 3：实现 Pg bridge**

Pg bridge 只把 domain request 映射到 Task 3 的 typed repo compounds，并把 ownership、CAS、replay 与 unavailable error 原样映射回窄 trait；不得暴露 pool/transaction/raw row，也不得在 bridge transaction 内发起外部调用。

**Step 4：运行 GREEN focused tests**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app -E 'test(verification_campaign_repository_)' --status-level fail)
```

Expected: unavailable fail-closed、ownership propagation、CAS/replay mapping 和 error code 保持一致。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/db_traits/verification_campaign.rs backend/crates/golish-agent-kit/src/db_traits/mod.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-app/src/ai/db_bridge/verification_campaign.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs
git commit -m "feat(verification): bridge campaign persistence"
```

## Task 5：把现有九角色改造成 durable Campaign team

**Files:**

- Create: `backend/crates/golish-sub-agents/src/defaults/prompts/verification_campaign.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/prompts/mod.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`
- Modify: `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- Create: `backend/crates/golish-sub-agents/src/executor/verification_campaign.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/{mod.rs,response_parsing.rs,tool_setup.rs}`
- Modify: `backend/crates/golish-sub-agents/src/defaults/tests.rs`
- Test: inline `#[cfg(test)]` module in `backend/crates/golish-sub-agents/src/executor/verification_campaign.rs`

**Step 1：写角色/权限 RED**

保留并重新定义现有角色能力：

- Owner：`verification_lead`，只拥有 consult、typed strategy decision、terminal intent；无 raw attack tool；
- Strategist/Specialist：pentester、researcher、PoC designer、auth/API/business/injection specialist，只读；
- Review：evidence analyst、independent critic；只读；
- Refinement：refiner；只写 typed plan delta；
- Recovery：adviser/reflector；仅 deterministic stall/no-progress 触发；
- Operator：仅 host 选择的 exact typed adapter，不能被 Agent 自由点名 raw args。

测试证明任何 child 不能调用 action dispatch、Finding、FactDelta 或 terminal submit；Lead 不能传 target/headers/body/credential/oracle verdict。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-sub-agents -E 'test(verification_campaign_) | test(candidate_role_permissions_)' --status-level fail)
```

Expected: durable role registry、consult census、typed artifact parser和forbidden-tool tests 因新 team尚不存在而失败；legacy candidate verifier tests不被删除或改成skip。

**Step 2：在 provider 调用前持久化 consult census**

每轮由 host 冻结 1–3 个真实 consult lane，状态闭集为 `queued/running/completed/failed/timed_out/cancelled`。每个 child 接收同一 round input 的安全 typed projection，并返回 versioned proposal/critique；provider 完成顺序不得改变 proposal identity 或 strategy decision hash。

**Step 3：替换临时 host pipeline**

把 `response_parsing.rs` 现有 `candidate_verifier` 临时 consult/pipeline 分支路由到 `verification_campaign.rs`；旧 operation 继续走 legacy branch。新 team 的 Evidence Analyst/Refiner/Reflector 产出必须落 durable artifact，不能只拼回 Lead prompt。

**Step 4：运行 GREEN**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-sub-agents -E 'test(verification_campaign_) | test(candidate_role_permissions_)' --status-level fail)
```

Expected: 真实角色定义、权限、census、timeout/cancel、deterministic proposal identity 与 legacy branch 隔离全部通过。

### Future Commit

```bash
git add backend/crates/golish-sub-agents/src/defaults/prompts/verification_campaign.rs backend/crates/golish-sub-agents/src/defaults/prompts/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/executor/verification_campaign.rs backend/crates/golish-sub-agents/src/executor/mod.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-sub-agents/src/defaults/tests.rs
git commit -m "feat(verification): build durable campaign team"
```

## Task 6：实现 Action Compiler、能力矩阵与 risk tier

**Files:**

- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/verification_action_compiler.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,verification_capabilities.rs}`
- Test: inline `#[cfg(test)]` module in `backend/crates/golish-pentest-app/src/pentest_bridge/verification_action_compiler.rs`

**Step 1：写 capability matrix RED**

每个公开capability必须同时存在：objective contract、observation contract、compiler、single/group executor kind、oracle、authorization tier、四层逐轴budget contract、network destination/proxy/TLS/cookie policy、conflict-key derivation、recovery policy。race/TOCTOU/double-spend类还必须存在`ConcurrentActionGroupV1` barrier、bounded subaction census与concurrency oracle；任一列缺失都要append-only写`verification_capability_assessments`的`adapter_missing|policy_denied|prerequisite_missing|unassessed`与exact residual，不能进入action proposal allowlist或被计为已覆盖；可用能力也必须写`available + adapter contract/version/digest`，不能靠内存registry结果授权。

同时固定 compiler 输出 exact obligation set、每个 action 到 coverage member 的 binding，以及不含 secret/args 的 semantic signature。shadow matcher 只有在 signature、frozen target/control/adapter/oracle version和legacy capability receipt authority全部 exact 时才能复用旧 receipt；unmatched或缺 receipt必须是`incomplete`。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(verification_action_compiler_) | test(verification_capability_matrix_) | test(verification_shadow_matcher_)' --status-level fail)
```

Expected: compiler、risk matrix、obligation binding和shadow matcher tests 因实现缺失而失败；现有 raw/general executor不能被当作fallback。

**Step 2：实现 server-owned compiler**

```rust
pub fn compile_prepared_action(
    strategy: &SealedStrategyDecision,
    authority: &CampaignAuthoritySnapshot,
    registry: &VerificationCapabilityRegistry,
) -> Result<CompiledPreparedAction, CompileDisposition>;
```

compiler从frozen wave/campaign coverage member、host-owned`VerificationContractV1` component/control、exact capability assessment、hypothesis/scope/target/credential handle/template digest派生canonical manifest。Lead只提交`strategy_obligation_id`；不能覆盖denominator membership、target、args、credential、risk、budget、network policy或oracle。compiler输出action-to-member binding与未编译member residual，但不能删除、合并或新增sealed denominator member。assessment writer在同一短transaction追加assessment/outbox；全部member评估完再seal exact assessment set，Campaign admission只接受该seal。

compiler结果是closed tagged union：`SingleActionV1`或`ConcurrentActionGroupV1`。后者只能由标为race-class且registry声明group-capable的objective产生，并冻结2..N个canonical subrequests、barrier cohort、max concurrency、start-window、每member credential/session、per-member及aggregate upper budget、union conflict-key exact set和concurrency oracle rule/version/digest。generic shell、两次顺序HTTP、模型建议的sleep或无instrumentation adapter不得拼成group；无法编译时保留objective member并写`adapter_missing + exact residual`。

Prepared Action 同时生成：

- private canonical manifest/hash；
- deterministic redacted display projection/hash；
- renderer version；
- T0–T3 risk decision；
- static/host-governed request upper bound；
- tagged action kind与（仅group）redacted subaction exact set/start-window/concurrency limit；
- server-derived conflict-key exact set与single/group oracle binding；
- expected control、oracle/recovery version；
- cleanup/data handling obligations。

每个触网Prepared Action还必须包含host生成并纳入private manifest/hash的`NetworkDestinationPolicyV1`：exact scheme/normalized host/port/origin与path boundary、同源/跨源redirect规则、`max_redirect_hops`、每次connection/retry的DNS resolution policy、允许的canonical destination set，以及禁止loopback/link-local/private/metadata等range的规则；V1 authoritative固定`proxy_mode=none`并忽略/禁用ambient `HTTP_PROXY/HTTPS_PROXY/ALL_PROXY`与system proxy，另冻结TLS trust-store/version/certificate/hostname/SNI policy、per-action isolated cookie jar policy、credential injection exact origin，以及redirect/retry时cookie/Authorization stripping规则。只绑定proxy endpoint/digest不足以证明代理如何DNS/CONNECT/redirect，因此不能授权；未来controlled proxy必须新contract并为每次send返回host可验证的destination/DNS/policy/budget/Tool Truth receipt后才能加入。anonymous/authenticated differential的两个lane使用隔离cookie jar与独立credential context，绝不能共享环境cookie或把authenticated状态污染anonymous control。例外只能来自exact scope authority并绑定policy hash；模型、redirect `Location`、环境变量、system proxy或adapter都不能扩展destination/credential边界。

secret 使用 opaque handle/version/scope/injection contract，不保存明文或明文 hash。

**Step 3：首批authoritative只封闭能走trusted host transport的两类adapter**

Plan C初始authoritative registry只接入已能由host HTTP transport逐connection治理、并满足完整矩阵的：

1. anonymous/authenticated differential；
2. directory soft-404/content fingerprint。

exact Nuclei replay在当前subprocess/CLI形态下无法让host逐request/redirect/retry/DNS观察并阻断，也无法完成Plan A required budget axes，因此只保留legacy/shadow signal，canonical assessment写`unassessed(reason_code=contract_pending)`，不进入authoritative registry。只有未来引入受控egress proxy/instrumented transport，并提供真实process fixture证明N+1发送前阻断、DNS/redirect scope与actual axes完整，才可新建adapter contract revision升级为available。其他technique同样写闭集assessment与residual；不使用raw shell、通用`pentest_run`、任意browser或未治理CLI fallback。

**Step 4：运行 GREEN compiler tests**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -E 'test(verification_action_compiler_) | test(verification_capability_matrix_) | test(verification_shadow_matcher_)' --status-level fail)
```

Expected: manifest/hash 可重复；scope/target/template/credential/oracle/destination/proxy/TLS/cookie/conflict/budget policy drift生成新revision并使旧授权失效；无有限request bound或无法受控egress的能力拒绝auto-approval。

### Future Commit

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/verification_action_compiler.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs
git commit -m "feat(verification): compile bounded prepared actions"
```

## Task 7：交付最小完整的 JIT read/review/mutation API

**PAUSE B — IPC 类型链授权：** schema批准不等于 generated IPC批准。开始生成或修改`frontend/lib/generated/`前，必须再次取得用户对跨IPC类型链变更的明确授权；未授权时可停在Rust command/DTO RED，不能手写生成文件绕过。

**Files:**

- Modify: `backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- Modify: `backend/crates/golish/src/commands_facade/attack.rs`
- Modify: `backend/crates/golish/src/commands_registry.rs`
- Modify: `frontend/lib/api/attack.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionScopeRequest.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionDecision.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionDecisionRequest.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionReviewItem.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionReviewState.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionDisplayView.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionBudgetAxisView.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionAuthorizationView.ts`
- Generate: `frontend/lib/generated/AttackPreparedActionDecisionResponse.ts`
- Create: `backend/crates/golish-agent-app/tests/prepared_action_ipc_authorization.rs`

**Step 1：写 IPC ownership/CAS RED**

新增命令：

- `attack_list_pending_prepared_actions`
- `attack_decide_prepared_action`

list request只接收operation id与可选campaign selector；server重验project/session/operation ownership、frozen scope、local principal，并证明campaign属于同一operation。`campaign_id=None`支持Plan C的operation-scoped fallback，`Some`支持Plan D Campaign detail；两者返回同一canonical pending row。mutation request精确为：

```rust
pub struct AttackPreparedActionScopeRequest {
    pub operation_id: String,
    pub campaign_id: Option<String>,
}

pub enum AttackPreparedActionReviewState {
    Pending,
    Authorized,
    Denied,
    Expired,
    Superseded,
    Drifted,
}

pub struct AttackPreparedActionAuthorizationView {
    pub authorization_receipt_id: String,
    pub decision: String,
    pub decided_at: String,
    pub expires_at: Option<String>,
}

pub struct AttackPreparedActionBudgetAxisView {
    pub axis: String,
    pub planned_limit: i64,
    pub unit: String,
}

pub struct AttackPreparedActionDisplayView {
    pub action_kind: String,
    pub target_at_time: String,
    pub method: String,
    pub redacted_sequence: Vec<String>,
    pub expected_control: String,
    pub destination_scope_summary: String,
    pub redirect_policy: String,
    pub max_redirect_hops: i64,
    pub network_policy_hash: String,
    pub planned_budget_axes: Vec<AttackPreparedActionBudgetAxisView>,
    pub cleanup_summary: Option<String>,
}

pub struct AttackPreparedActionReviewItem {
    pub prepared_action_id: String,
    pub operation_id: String,
    pub campaign_id: String,
    pub display_projection: AttackPreparedActionDisplayView,
    pub private_manifest_hash: String,
    pub display_projection_hash: String,
    pub renderer_version: String,
    pub risk_tier: String,
    pub review_state: AttackPreparedActionReviewState,
    pub row_version: i64,
    pub expires_at: Option<String>,
    pub authorization: Option<AttackPreparedActionAuthorizationView>,
}

pub enum AttackPreparedActionDecision {
    Approve,
    Deny,
}

pub struct AttackPreparedActionDecisionRequest {
    pub operation_id: String,
    pub campaign_id: String,
    pub prepared_action_id: String,
    pub decision: AttackPreparedActionDecision,
    pub private_manifest_hash: String,
    pub display_projection_hash: String,
    pub renderer_version: String,
    pub expected_row_version: i64,
    pub stable_request_id: String,
    pub requested_expiry: Option<String>,
}

pub struct AttackPreparedActionDecisionResponse {
    pub operation_id: String,
    pub campaign_id: String,
    pub prepared_action_id: String,
    pub review_state: AttackPreparedActionReviewState,
    pub authorization: Option<AttackPreparedActionAuthorizationView>,
    pub row_version: i64,
    pub replayed: bool,
}
```

测试拒绝跨project/org、campaign不属于operation、stale scope、deleted live target、过期packet、hash/renderer drift、重复不同payload的request id。`campaign_id=None`与同operation的`Some(campaign_id)`对同一pending row返回相同hash/row version；exact response-loss replay返回同一authorization receipt。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(prepared_action_ipc_)' --status-level fail)
```

Expected: command、完整DTO、ownership/CAS/replay tests 因实现不存在而失败；现有 attack command仍通过。

**Step 2：实现 Authorization Broker**

- T0/T1 只由 server policy auto decision；UI 不出现人工按钮；
- T2/T3 要求 pending exact display packet 与 human JIT；
- backend 从 local principal/policy 派生 actor、tier、actual expiry；客户端 expiry 只能被 clamp；
- deny/expire 写 residual，不生成 execution/oracle；
- display projection 不含 secret/raw payload/raw response。

**Step 3：生成 ts-rs types**

只修改 Rust `#[derive(TS)]` source，然后运行受影响 crate 的 binding export tests；不手改生成文件。

```bash
just space-guard
(cd backend && cargo test -p golish-agent-app export_bindings -- --nocapture)
```

Expected: 新 DTO bindings 写入 `frontend/lib/generated/`；git diff 中不存在手工格式漂移。

**Step 4：运行 GREEN focused backend tests**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -E 'test(prepared_action_ipc_)' --status-level fail)
```

Expected: ownership、stable code、CAS、replay、expiry 与 drift 全绿。

### Future Commit

```bash
git add backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish/src/commands_facade/attack.rs backend/crates/golish/src/commands_registry.rs backend/crates/golish-agent-app/tests/prepared_action_ipc_authorization.rs frontend/lib/api/attack.ts frontend/lib/generated/AttackPreparedActionScopeRequest.ts frontend/lib/generated/AttackPreparedActionDecision.ts frontend/lib/generated/AttackPreparedActionDecisionRequest.ts frontend/lib/generated/AttackPreparedActionReviewItem.ts frontend/lib/generated/AttackPreparedActionReviewState.ts frontend/lib/generated/AttackPreparedActionDisplayView.ts frontend/lib/generated/AttackPreparedActionAuthorizationView.ts frontend/lib/generated/AttackPreparedActionDecisionResponse.ts
git commit -m "feat(verification): expose prepared action authorization"
```

## Task 8：实现 DB-bootstrap 的最小安全审批 UI

**Files:**

- Create: `frontend/components/Engagement/PendingPreparedActionPanel.tsx`
- Create: `frontend/components/Engagement/PendingPreparedActionPanel.test.tsx`
- Modify: `frontend/components/Engagement/index.ts`
- Modify: `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- Modify: `frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx`
- Modify: `frontend/lib/api/attack.ts`

**Step 1：写 UI RED**

覆盖 loading/error/empty/stale-refresh 四态，以及：

- cold start、restore、missed event、没有 selected tool 时都按 operation 主动 list；
- T0/T1 只显示 policy decision/audit，无 approve/reject；
- T2/T3 显示 method/origin/path/redacted payload diff/request ceiling/control/oracle/expiry；
- secret/token/cookie/raw body 不进入 DOM；
- digest、renderer、expiry、row version 漂移立即禁用按钮并 reload；
- approve/reject 复用同一个 stable request id 处理 response loss；
- mutation pending 时按钮 disabled；错误按 backend `code` map 显示；
- refresh event 只是 revalidate hint，不能取代 mount bootstrap。

**Step 1b：运行 RED**

```bash
pnpm exec vitest run frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
```

Expected: panel、cold-start DB bootstrap、CAS/expiry/drift与secret sentinel tests 因组件/API尚不存在而失败；现有Candidate detail tests保持原断言。

**Step 2：实现 minimal panel**

Plan C 先把 panel 挂到现有 Engagement/ToolCall pane 的 operation-scoped 区域；不得要求用户先选 Candidate tool call。Plan D 只迁移布局到 Workspace，不改变 API 或授权语义。

**Step 3：运行 GREEN 前端验证**

```bash
pnpm exec vitest run frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
pnpm exec biome check frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/lib/api/attack.ts
pnpm typecheck
```

Expected: focused tests、Biome、typecheck exit 0；DOM snapshot 无 secret fixture；无 `invoke()` 绕过 API wrapper。

### Future Commit

```bash
git add frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/lib/api/attack.ts
git commit -m "feat(frontend): add prepared action review panel"
```

## Task 9：接入 trusted execution、budget governor 与 recovery

**Files:**

- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs`
- Create: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/verification_campaign.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Create: `backend/crates/golish-agent-kit/src/task_orchestrator/verification_campaign.rs`
- Modify: `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`
- Test: inline `#[cfg(test)]` modules in `verification_capabilities.rs`, `verification_campaign.rs` and `candidate_verification.rs`

**Step 1：写 execution/recovery RED**

覆盖：authorization expired/drift before begin、Plan A artifact在admission后/begin前tamper时fresh guard令零send、caller伪造/缓存authority token失败、conflict-key partial-overlap collision、四层budget并发oversubscription、request N+1 fail closed、redirect/retry计入actual、single与ConcurrentActionGroup durable begin后response loss、group缺subexecution/outcome_unknown不自动重放且reservation/all-key leases保持recovery hold、late result只落superseded witness、cleanup未完成阻止正常terminal。

网络负例必须逐hop固定：默认拒绝redirect；未授权cross-origin redirect在第二次send前拒绝；scheme/host/port/path boundary漂移拒绝；DNS同时返回public+private/loopback/link-local/metadata地址时整次connection拒绝；首查public、retry/redirect时rebind到private也拒绝；每次connection/retry都重新resolve并验证全部A/AAAA，然后把选定validated IP直接pin到socket，同时保留原Host/SNI，禁止HTTP client再次自行解析。环境proxy/system proxy不能旁路pin；untrusted cert/hostname/SNI mismatch拒绝；anonymous/auth两个lane的cookie jar不共享，cross-origin redirect剥离cookie/Authorization。blocked hop只写policy residual/audit，不产生trusted observation或oracle input。

safety-hold race必须覆盖：durable begin后、第一次send前开启hold时零send；第一跳后、redirect或retry前开启hold时不发生下一跳；campaign hold on→off导致`campaign_dispatch_generation`变化时旧authorization仍拒绝；只改变`operation_admission_generation/row_version`时合法在途send不被误杀。credential handle在begin后或两hop之间rotate/revoke时，旧version/revocation generation在任何secret injection/send前永久失效。只允许另签typed cleanup/recovery authority执行exact allowlist动作，且该流量同样受destination/budget治理、只能形成cleanup audit/residual，不能成为原security oracle proof。

再加联合 admission负例：rank 0–4、`tool_truth_contract != receipt_v1`、Plan A checked bundle漏root/任一member非consistent或temporal expired/mixed-epoch/skew-exceeded、Plan B snapshot epoch/policy不匹配、verification plan/denominator未sealed或rollout safety hold开启时，panic executor调用数必须为0；只有joint rank 5/6且all-fresh bundle进入真实执行。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -p golish-pentest-app -E 'test(verification_campaign_execution_) | test(prepared_action_recovery_) | test(request_budget_governor_) | test(network_destination_policy_) | test(dns_rebinding_) | test(verification_joint_admission_)' --status-level fail)
```

Expected: direct runtime、joint admission、budget/recovery tests因实现缺失而失败；legacy verifier tests仍走原路径。

**Step 2：实现三段式执行**

```text
fresh guard + begin transaction: stable snapshot/reconcile exact root+derived authority set + lock/reserve all operation→wave→campaign→action budget axes + acquire canonical all-key fenced leases + durable single/group begin
outside transaction: execute exact typed adapter (group uses bounded host barrier) + stage raw witness + exact subexecution receipts
fresh guard + closeout transaction: re-attest source authority + CAS + typed observation/fact/evidence + actual counters + exact-one Tool Truth receipt per execution/subexecution + coverage-member result binding or quarantine/HOLD
```

`execution_ordinal` 在首次 begin 冻结；response loss 使用 `prepared_action_id + authorization_receipt_id + execution_ordinal` 返回原 receipt，group另以member ordinal稳定复用subexecution。begin只能由Plan A checked-bundle callback内在all-fresh conversion后执行module-private compound，并冻结新multi-root bundle seal/hash/temporal window；它按固定祖先和conflict-key顺序原子reserve/acquire。closeout再次构造checked bundle并要求all-fresh，且action/control observations必须满足Plan B `EvidenceTemporalValidityPolicyV1`的same-epoch/max-skew（negative/refutation用更短TTL），再按known/unknown规则settle并与all-key release/recovery-hold同事务；若source已orphan/expired则只close/quarantine/revalidation-HOLD，不能产生objective proof/refutation。任何再次触达目标都必须创建新 Prepared Action/authorization。

**Step 3：实现host-owned逐hop transport policy与budget accounting**

所有authoritative HTTP adapter只能接收`AuthorizedPinnedTransport`，不能拿通用client。transport在每个initial connection、redirect与retry前执行固定顺序：

1. 强一致重读`verification_campaign_safety_holds`，要求`campaign_dispatch_held=false`且`campaign_dispatch_generation`与authorization exact匹配；同时重读Plan A semantic authority heads/Plan C quarantine、credential handle current version/scope/injection contract/revocation generation，并从Prepared Action校验`NetworkDestinationPolicyV1` hash、authorization expiry/CAS与exact scheme/host/port/path；任何authority/credential/campaign-scope drift都在secret injection与本次网络I/O前停止，旧authorization永久不可复活。全局row version仅记录审计而不参与send authority；
2. 对redirect先规范化Location；默认deny，同源也必须在policy允许且hop未超`max_redirect_hops`，跨源只接受compiler冻结的exact destination；
3. 由host resolver取得全部A/AAAA，逐个检查scope与禁止range；混合合法/非法结果整体拒绝，不能挑一个合法地址掩盖恶意答案；
4. 选择已验证IP并直接pin socket，TLS SNI/HTTP Host保持canonical hostname，adapter/client不得二次resolve；V1 client必须禁用ambient/system proxy；TLS trust/cert/hostname/SNI按冻结policy验证；每个Prepared Action/lane使用隔离cookie jar，redirect/retry按origin规则剥离cookie与Authorization；
5. 在send前从既有action reservation原子consume，并同时校验operation/wave/campaign/action四层head与Plan A各required budget axis；第N+1次或任一祖先额度不足都在网络I/O前拒绝；response body bounded stream逐chunk累计bytes，monotonic deadline记录wall-clock；不能在per-send临时创造超出begin upper bound的新reservation；
6. 每次retry/redirect/新connection都从第1步重新开始，重新检查safety hold、DNS与完整policy；DNS rebinding、metadata endpoint、loopback/private/link-local、proxy/cookie/TLS drift和未授权port/path均写stable policy residual/audit，零trusted observation。

已经开启hold后仍允许本地stage witness、closeout、budget unknown-hold与lease recovery-hold落库；如确需目标侧cleanup，必须由独立`CleanupTransportAuthorityV1`绑定exact cleanup action/destination/budget/current hold policy的显式allowlist。它不能复用原业务authorization，也不能生成proof/refutation oracle input。

browser/OAST目前不在首批authoritative registry；未来接入时browser subrequest、WebSocket、download与OAST registration/callback token也必须经过等价governor。CLI自报、stdout行数或模型估算不成为authority。

**Step 4：运行 GREEN focused tests**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-runtime -p golish-pentest-app -E 'test(verification_campaign_execution_) | test(prepared_action_recovery_) | test(request_budget_governor_) | test(network_destination_policy_) | test(dns_rebinding_) | test(verification_joint_admission_)' --status-level fail)
```

Expected: no duplicate side effect；unknown/recovery/late result 语义稳定；外部 work 不在 transaction 内。

### Future Commit

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/verification_campaign.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-kit/src/task_orchestrator/verification_campaign.rs backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs
git commit -m "feat(verification): execute prepared actions safely"
```

## Task 10：落地三个 deterministic action oracle 与 Campaign adjudicator

**Files:**

- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/verification_oracles.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/{anonymous_access.rs,verification_capabilities.rs}`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- Modify: `backend/crates/golish-db/src/repo/verification_oracles.rs`
- Create: `backend/crates/golish-pentest-app/tests/verification_oracles.rs`
- Modify: `backend/crates/golish-db/tests/verification_campaigns.rs`

**Step 1：写 oracle fixture RED**

- Nuclei no-match默认`inconclusive/scanner_no_match`；当前CLI receipt因transport/budget不可完整，永远不能refute。只有未来受控egress adapter的新contract revision同时满足exact deterministic negative rule + prerequisites/control + complete receipt时，才可能refute exact template condition；
- anonymous access differential必须冻结相同resource semantic identity（method/origin/path/query/body shape）、隔离的anonymous/authenticated cookie+credential contexts、authenticated-session-valid control、bounded observation window/max-skew、cache/redirect normalization和versioned status/content/sentinel comparator；任意2xx或不同资源响应不等于proof。authenticated control失效、baseline动态/不稳定、lane跨epoch/超skew或cache/WAF差异只能inconclusive；
- directory soft-404先请求多个server-derived deterministic nonexistent paths并冻结baseline exact set；重验status、redirect chain/final origin、content-type、normalized body fingerprint/length variance、WAF/captcha/challenge与versioned tolerance。candidate只有在完整baseline稳定且relation rule满足时才proof/refutation；敏感词、单个200或长度差异单独只能signal/inconclusive；
- parser reject、partial Tool Truth、invalid control、unknown precondition 都降为 inconclusive；
- single-action proof 只满足对应 predicate component。
- oracle census必须与`VerificationContractV1` component/control和Campaign coverage obligation exact-equal；漏component、漏control、duplicate、forged census hash或旧oracle revision均HOLD。
- `explicit_no_control` component仍必须有exact census/coverage member并返回`control_validity=not_required`；空controls不能形成空census或vacuous verified，required control也不能伪装成not_required。
- combinator完整真值表：`all_of`一项valid refutation可refute但一项proof不能verify；`any_of`一项proof可verify但必须全项valid refutation才refute；paired缺pair/control和ordered乱序/跨session均inconclusive。
- `ConcurrentActionGroupV1` oracle必须消费sealed group/subexecution exact set、barrier/start-window与versioned concurrency relation；缺member、顺序Action拼接、超window、任一outcome unknown或control污染均inconclusive。

shadow fixture还要证明deterministic oracle可只读消费旧链已存在的capability receipt，但无法匹配、receipt不完整或rule/version不同时只能通过Plan B `compare_and_record_v1`写`incomplete` sample；不得产生第二份shadow verdict truth、canonical adjudication、coverage result、Finding或FactDelta。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -p golish-db -E 'test(nuclei_action_oracle_) | test(anonymous_access_oracle_) | test(directory_oracle_) | test(campaign_adjudication_) | test(shadow_receipt_oracle_replay_)' --status-level fail)
```

Expected: oracle registry、coverage outcome mapping和shadow replay tests因实现缺失而失败；Plan A scanner no-match regression仍保持inconclusive。

**Step 2：实现 versioned oracle registry**

每个assessment绑定`oracle_rule_id/version/digest`、Prepared Action或ConcurrentActionGroup、execution/subexecution receipt exact hash、evidence snapshot、Plan A temporal policy/target epoch/observation window、Plan B claim-component id/hash、tagged control binding/action/predicate-component ids、coverage member ids、preconditions、control、completeness、reason/limitation codes。模型prose只进入解释字段。anonymous differential registry固定same-resource、credential isolation、session-validity、cache/redirect normalization与semantic/sentinel equivalence；directory registry固定nonexistent-path count/derivation、variance tolerance、content-type/redirect/WAF规则，unknown/dynamic baseline fail closed。canonical oracle给每个绑定member产生typed`epistemic_outcome`与claim-component outcome；coverage disposition由terminalizer另算，control validity单独列；required binding只能`valid|invalid|not_assessed`，explicit-no-control只能`not_required`，三轴不能混成一个outcome enum。所有expected oracle obligations终态后由host调用`seal_oracle_census`，按contract/claim-component ordinal与canonical hash写seal/member；adjudicator只接受该immutable seal，不接收调用方临时`Vec<assessment>`。

**Step 3：实现 campaign adjudication**

exhaustive match Plan B唯一`ContractCombinatorV1`四值，严格按Task 1冻结的`all_of/any_of/paired_differential/ordered_sequence`真值表聚合exact oracle census与obligation disposition。paired必须重验pair/control binding；ordered必须重验step ordinal、同session/causal chain、interleaving/reset policy和complete observation window。缺oracle membership、required control或completeness时不得verified/refuted；explicit-no-control只有marker/census/result三者exact且非空才可参与聚合；未知future combinator fail closed。

**Step 4：运行 GREEN**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-pentest-app -p golish-db -E 'test(nuclei_action_oracle_) | test(anonymous_access_oracle_) | test(directory_oracle_) | test(campaign_adjudication_) | test(shadow_receipt_oracle_replay_)' --status-level fail)
```

Expected: 正/负/控制失败/partial/legacy fixture 全绿；广义“没有漏洞”文案不出现在 authoritative verdict。

### Future Commit

```bash
git add backend/crates/golish-pentest-app/src/pentest_bridge/verification_oracles.rs backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs backend/crates/golish-db/src/repo/verification_oracles.rs backend/crates/golish-pentest-app/tests/verification_oracles.rs backend/crates/golish-db/tests/verification_campaigns.rs
git commit -m "feat(verification): adjudicate with typed oracles"
```

## Task 11：实现 terminalizer、FactDelta、outer-loop consolidation 接口

**Files:**

- Modify: `backend/crates/golish-agent-kit/src/harness/verification_campaign/gate.rs`
- Create: `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/consolidation.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/hypothesis_registry/{mod.rs,reducer.rs}`
- Modify: `backend/crates/golish-db/src/repo/{verification_campaigns.rs,verification_campaign_coverage.rs,verification_fact_delta_bundles.rs,hypothesis_registry.rs,hypothesis_objective_outcomes.rs,hypothesis_revision_adjudications.rs}`
- Create: `backend/crates/golish-db/src/repo/hypothesis_consolidations.rs`
- Modify: `backend/crates/golish-db/src/repo/finding_lineage.rs` only through its public compound seam
- Modify: `backend/crates/golish-db/tests/verification_campaigns.rs`

**Step 1：写 atomicity/lineage RED**

覆盖：

- objective-local proof/refutation同事务只写Campaign adjudication/terminal、objective outcome、coverage receipt与FactDelta，不能创建Finding或改变revision epistemic state；注入任一步失败后全部不存在；
- blocked/inconclusive/exhausted/unassigned保留exact residual并推进该objective current outcome head，但不改变revision epistemic state；late/superseded Campaign outcome不能覆盖head；
- revision adjudicator只接受server锁定current heads后形成、与Plan B plan objective及每条path required claim-component union exact-equal的sealed outcome set；单Campaign、caller挑旧proof、漏objective/component/impact qualifier或stale/quarantined outcome均整体rollback；
- revision adjudicator从selected lineage自行derive relevant roots并在Plan A guard内持久化multi-root bundle与跨objective temporal census；漏root、caller传seal、expired TTL、mixed target epoch或max-skew超限都不能终态；
- revision-level verified在独立事务写adjudication、terminal decision、Finding/evidence membership与Plan B state event；refuted写adjudication、terminal decision、exact refutation lineage与state event而不建Finding。注入任一步失败全部rollback；
- FactDelta 不接受模型 accept/reject，只能 exact-once `applied/no_semantic_change/quarantined_invalid_authority`；该第三态表示首次消费时authority已无效，一旦写入永不因后续quarantine改状态；已applied后才失效则保留`applied`并走独立correction lineage；
- Plan A same-semantic/all-fresh multi-root bundle保持valid时不改变Campaign/report semantic hash；Campaign closeout和revision adjudication都重验temporal policy/epoch/window，expired或超skew不能终态。后续artifact tamper产生invalid semantic reconciliation时，quarantine compound同事务追加leaf outcome/terminal/coverage/FactDelta及依赖aggregate/Finding/refutation/report invalidation、Gate residual/HOLD与typed outbox；
- 已`applied`的FactDelta consumption保持immutable，quarantine另写exact correction bundle/consumption；重复同一invalid reconciliation exact replay，new invalid version追加lineage；
- quarantine canonical commit后即使projection仍旧，Gate/Wave/Reporting strong reader也立即拒绝原authority；projector整批失败不重新授权旧terminal；
- terminal/Finding到期但未semantic invalid时只转`temporally_stale`并创建revalidation + H(g+1) re-adjudication obligation；same-semantic replacement receipt不能复活旧terminal，semantic orphan才转`semantically_invalid`并走quarantine；
- material delta 触发 H(g+1)，全 no-semantic-change 只写 fixed-point receipt；
- Campaign terminal不等待上述consumption；Wave consolidation等全部current Campaign terminal/unassigned outcome后独立运行；revision adjudication再等objective outcome exact set，不与Campaign local drain互相等待。
- Campaign terminal receipt、objective outcome、host-sealed oracle census、exact-one coverage receipt、每个denominator member exact-one result、residual membership与FactDelta bundle在同一transaction；tested结果缺action+capability receipt+oracle、oracle census缺/额外component/control，或四类coverage disposition count之和不等于member count时整体rollback。epistemic outcome与control validity使用各自census/hash，不混入四类coverage count。
- Wave consolidation 只有在全部 Campaign terminal/coverage receipt 完成后才能写 wave coverage receipt；任一 wave member 既不在 Campaign partition 也无 explicit unassigned residual 时整体 rollback。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db -E 'test(verification_campaign_terminal_) | test(verification_campaign_coverage_) | test(verification_fact_delta_) | test(hypothesis_generation_from_verification_)' --status-level fail)
```

Expected: terminal/coverage/FactDelta/consolidation tests因atomic compound尚未实现而失败；注入失败不会留下部分row。

**Step 2：实现 canonical server-derived bundle**

FactDelta只能从execution receipt + sealed oracle census + objective-local campaign adjudication + evidence membership生成；Lead只能另写optional `hypothesis_evolution_proposal.v1`。bundle保留source authority，即使quarantine也不可删除/重写；若已消费，只有host-derived authority correction bundle能在后续generation显式撤销受污染的support/contradiction，旧consumption状态永不复用。coverage receipt只从已sealed denominator和exact member results生成，不能从Campaign实际action列表反推。Wave receipt只从sealed wave denominator、selected current且未quarantine的Campaign partition/receipt与`verification_wave_unassigned_coverage_results` exact set生成，不能把未编译/未分配objective删出分母。

只有本Task的typed `adjudicate_hypothesis_revision_with_fresh_tool_truth` seam可以请求Plan B Registry把revision推进为`verified/refuted`：repo自行锁/选择每个plan objective的latest eligible head，seal outcome exact set，从其lineage推导server-owned relevant roots，并在Plan A guard callback内运行Plan B唯一proof-path reducer；它绑定revision adjudication/terminal decision、Finding lineage（verified）或refutation lineage（refuted）及完整operation/project/org/all-fresh temporal authority与可重放census。Plan B普通Candidate mutation writer仍硬拒`verified/refuted`；单Campaign terminal/oracle/coverage/FactDelta、Lead/Controller/evidence refs都不能调用transition。server contract-invalid继续使用独立validator seam。

Campaign closeout compound在同一canonical transaction写Plan B typed outbox batch，exact四个ordinal固定为`campaign_terminal / objective_outcome / campaign_coverage / fact_delta`；revision terminal compound另写exact四个ordinal `hypothesis_revision_adjudication / hypothesis_revision_terminal_decision / finding_or_refutation / hypothesis_state_event`，nonterminal adjudication按catalog写adjudication+residual exact manifest。所有members带同一`source_batch_seq/predecessor`与server-redacted immutable payload/hash；projector只能整batch原子materialize entity versions/change rows/head，不能先让UI看到单Campaign“已验证”、terminal却没有coverage/FactDelta，或Hypothesis终态却没有aggregate authority。outbox写失败让canonical transaction整体rollback，不能靠Plan D补发；projection消费失败不回滚canonical truth，但head不推进并让read side fail closed/stale。

quarantine是独立compound，不修改上述历史batch：它冻结原leaf outcome/terminal/FactDelta/consumption、所有依赖aggregate/Finding或refutation/report source及Plan A invalid authority exact set，必要时创建correction bundle与H(g+1) re-adjudication obligation，并写只包含实际受影响entity的typed `Invalidate` batch。`fact_delta_consumptions`没有quarantine UPDATE路径；Gate/current authority selector以quarantine event为append-only否定authority。Plan D Reporting finalizer每次finalize/reuse/current-view前强读该event；若检测到orphan但compound尚未完成，则创建或等待同一stable quarantine request，绝不能仅显示warning后继续。

**Step 3：接入 Registry reducer 与新 consolidation seam**

传递 expected delta exact set、optional evolution proposal exact set、application-fact refinement obligations；结构性冲突由 Controller typed decision，support+contradict set-based union 不受完成顺序影响。

**Step 4：运行 GREEN**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-db -E 'test(verification_campaign_terminal_) | test(verification_campaign_coverage_) | test(verification_fact_delta_) | test(hypothesis_generation_from_verification_)' --status-level fail)
```

Expected: terminal atomicity、exactly-once、quarantine invalidation、material/fixed-point 和 concurrency-order invariance 全绿。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/harness/verification_campaign/gate.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/consolidation.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/mod.rs backend/crates/golish-agent-kit/src/harness/hypothesis_registry/reducer.rs backend/crates/golish-db/src/repo/verification_campaigns.rs backend/crates/golish-db/src/repo/verification_campaign_coverage.rs backend/crates/golish-db/src/repo/verification_fact_delta_bundles.rs backend/crates/golish-db/src/repo/hypothesis_registry.rs backend/crates/golish-db/src/repo/hypothesis_objective_outcomes.rs backend/crates/golish-db/src/repo/hypothesis_revision_adjudications.rs backend/crates/golish-db/src/repo/hypothesis_consolidations.rs backend/crates/golish-db/src/repo/finding_lineage.rs backend/crates/golish-db/tests/verification_campaigns.rs
git commit -m "feat(verification): close campaigns into fact deltas"
```

## Task 12：替换 Verification scheduler、三层 drain/Gate 与 stall policy

**Files:**

- Modify: `backend/crates/golish-agent-kit/src/task_orchestrator/verification_campaign.rs`
- Modify: `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/attack_execution/verification_gate.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
- Modify: `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- Create: `backend/crates/golish-agent-kit/tests/verification_campaign_scheduler.rs`
- Modify: `backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs`
- Test: inline `#[cfg(test)]` cases in `stage_run_call.rs`

**Step 1：写 scheduler RED**

覆盖：consult bounded parallel/每Campaign一个active Prepared Action（single或原子ConcurrentActionGroup）、new-round admission、campaign budget stop、local drain、Wave consolidation admission、objective outcome current-head selection、revision-level proof-path adjudication、new-generation materiality、new-wave runnable obligation、stage final seal。特别证明local Campaign terminalizer不等待Registry consumption、单Campaign不能终结revision，final Gate不把`blocked/exhausted/unassigned/race adapter_missing`渲染成完整覆盖。

再覆盖shadow/dual：旧Attempt terminal后才启动isolated shadow evaluator；它只读复用已落库旧capability receipt与redacted snapshot，把obligation/action/oracle/residual完整record交给Plan B唯一`compare_and_record_v1`，并通过返回的comparison IDs形成evaluation exact set。panic provider/LLM、HTTP/network/DNS、browser、shell/CLI、OAST、executor、Authorization Broker、credential resolver、lease/journal/budget mock的调用数全部为0，新capability receipt数也为0；任何unmatched/缺receipt/replay失败在同一sample ledger中为`incomplete`。

**Step 1b：运行 RED**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -E 'test(verification_campaign_scheduler_) | test(verification_campaign_gate_) | test(verification_campaign_fixed_point_) | test(verification_shadow_evaluator_)' --status-level fail)
```

Expected: authoritative scheduler、shadow evaluator、三层drain和route tests因实现缺失而失败；legacy path仍通过。

**Step 2：实现 deterministic stall/fixpoint**

semantic fingerprint 固定 exact predicate/objective/control/action/adapter/oracle versions/evidence membership；无关 evidence 或 prose 改写不绕过。连续无新 strategy/action/evidence 或 budget/deadline 到限触发 stop scheduling；drain 后 terminal `exhausted_with_residuals`。

**Step 3：operation-frozen dispatch**

canonical scheduler只服务joint rank 5/6；rank 0/1继续现有`candidate_verifier`/Attempt scheduler，rank 2–4也继续旧authority并只在其terminal后调用isolated shadow evaluator。shadow evaluator不创建canonical Campaign/Prepared Action/execution/adjudication/coverage/FactDelta，也不进入Gate/Reporting。切换default不迁移旧operation。

Plan B 的 `plan_c_verification_unavailable` 是部署顺序占位，不是 Plan C 完成后的 authoritative-new 行为。Candidate finalizer 与 graph handoff 必须同时改成：

- 对 Plan C 上线后新建、尚未 sealed placeholder residual 的 authoritative-new operation，Candidate seal 后进入 Verification Campaign，不再写 `plan_c_verification_unavailable`，也不直接跳 Reporting；
- 对已经 sealed `plan_c_verification_unavailable` 并转 Reporting/terminal 的历史 operation，保持原事实和原 route，禁止 retroactive Campaign、删除 residual 或同 operation 语义升级；
- 在 Plan D promotion 前，deployment default 仍是 `legacy_only`，因此 authoritative-new 只允许本地 fixture/test operation；若数据库出现非 fixture 的 authoritative-new operation 且既没有合法 placeholder terminal receipt、也没有 Campaign admission receipt，启动/恢复必须 fail closed 并要求人工审计。

新增 exact route tests 覆盖以上三格，防止 Plan B 临时 stop-gap 在 Plan C 后继续吞掉 Verification，或用新 binary 重解释旧 operation。

**Step 4：运行 GREEN**

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -E 'test(verification_campaign_scheduler_) | test(verification_campaign_gate_) | test(verification_campaign_fixed_point_) | test(verification_shadow_evaluator_)' --status-level fail)
```

Expected: recovery、budget、callback、cleanup、quarantine 和 next-wave cases 都得到确定性 route；无无限重试。

### Future Commit

```bash
git add backend/crates/golish-agent-kit/src/task_orchestrator/verification_campaign.rs backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs backend/crates/golish-agent-app/src/ai/candidate_analysis_gate.rs backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-kit/src/harness/attack_execution/verification_gate.rs backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/tests/verification_campaign_scheduler.rs backend/crates/golish-agent-app/tests/candidate_analysis_runtime.rs
git commit -m "feat(verification): schedule and gate campaigns"
```

## Task 13：兼容投影、模块卡与 scoped release evidence

**Files:**

- Modify: `backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs`
- Modify: `backend/crates/golish-db/src/repo/investigation_rollout.rs`
- Modify: `docs/modules/backend/golish-core.md`
- Modify: `docs/modules/backend/golish-agent-kit/harness.md`
- Modify: `docs/modules/backend/golish-agent-kit/db_traits.md`
- Modify: `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- Modify: `docs/modules/backend/golish-db/repo.md`
- Modify: `docs/modules/backend/golish-agent-app/ai.md`
- Modify: `docs/modules/backend/golish-sub-agents/defaults.md`
- Modify: `docs/modules/backend/golish-sub-agents/executor.md`
- Modify: `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- Modify: `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- Modify: `docs/modules/backend/golish.md`
- Modify: `docs/modules/frontend/components.md`
- Modify: `docs/modules/frontend/lib.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

**Step 1：验证 compatibility projection**

authoritative Campaign的legacy Candidate/Attempt只能由Plan B异步compatibility projector从canonical typed outbox snapshot派生；canonical terminal transaction不直接写legacy row，新Campaign永远不写`candidate_attempt_actions`旧journal。projection lag/error/divergence写typed projection status/residual并让旧consumer fail closed，但不回滚或改写canonical verdict；same batch的Candidate/Attempt projection必须all-or-none。legacy operation只通过read-only adapter读取既有row，不回填Registry/Campaign，也不伪造request packet/oracle/consult；无法映射时read model显示`legacy_unavailable`。

**Step 2：运行最终定向 Rust 验证**

每条 Cargo 命令前运行 `just space-guard`：

```bash
just space-guard
(cd backend && cargo nextest run -p golish-agent-kit -E 'test(verification_campaign_)' --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-db --test verification_campaigns --status-level fail)
just space-guard
(cd backend && cargo nextest run -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app -E 'test(verification_campaign_) | test(prepared_action_) | test(campaign_adjudication_)' --status-level fail)
just space-guard
(cd backend && cargo clippy -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app --all-targets -- -D warnings)
just space-guard
(cd backend && cargo fmt -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-pentest-app -- --check)
```

Expected: 所有 focused tests 通过，受影响 crates clippy 零 warning，rustfmt exit 0。

**Step 3：运行最终定向前端验证**

```bash
pnpm exec vitest run frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx
pnpm exec biome check frontend/components/Engagement/PendingPreparedActionPanel.tsx frontend/components/Engagement/PendingPreparedActionPanel.test.tsx frontend/components/Engagement/index.ts frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.candidate.test.tsx frontend/lib/api/attack.ts
pnpm typecheck
```

Expected: focused tests、Biome、typecheck exit 0。

**Step 4：离线 replay acceptance**

使用无外部目标 fixture 验证：

1. 两轮策略后单Campaign只原子关闭objective outcome/coverage/FactDelta，revision保持非终态；全部current objective+claim-component outcomes exact闭合后，独立revision adjudication才原子写verified Finding/terminal/state event；
2. Nuclei no-match 为 inconclusive；
3. JIT denied 无 execution/oracle；
4. outcome_unknown 不重放；
5. budget exhausted 先 drain 再 residual；
6. support+contradict 并发 consolidation 得到同一 semantic hash；
7. cold-start UI 能发现 pending T2 packet；
8. legacy/shadow/compare不授权新执行；shadow/dual仍完成planner/signature/receipt-replay/oracle/exact-set evaluation，panic provider/LLM/network/DNS/browser/shell/OAST/executor/auth/credential/lease/journal/budget调用数与新receipt数全部为0；
9. denominator member exact-set与coverage result/receipt计数闭合，未执行objective不会从分母消失。
10. 多proof-path、漏impact qualifier、旧outcome cherry-pick与单Campaign提前终态全部fail closed；
11. multi-root all-fresh temporal bundle、negative/refutation TTL、same-epoch/max-skew在begin/closeout/revision adjudication三处重验；
12. ConcurrentActionGroup partial-overlap conflict keys、subexecution exact census与race oracle闭合；不支持的race-class保持adapter_missing residual。

**Step 5：记录证据与状态**

把每条命令、exit code、关键 case count 和离线 fixture authority hash 写入 `agent-progress.md`；逐条核对 `feature_list.json.verification`。未获得真实目标/provider/rollout 授权时如实记录未运行，不得伪称 live 验证。只有完成定义 1–5 均满足时标 `passing`。

### Future Commit

```bash
git add backend/crates/golish-db/src/repo/hypothesis_legacy_projection.rs backend/crates/golish-db/src/repo/investigation_rollout.rs docs/modules/backend/golish-core.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/db_traits.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish.md docs/modules/frontend/components.md docs/modules/frontend/lib.md docs/modules/INDEX.md feature_list.json agent-progress.md
git commit -m "docs(verification): record campaign rollout evidence"
```

## Rollback 与部署顺序

1. 先部署 additive schema、纯读 repo、isolated shadow evaluator和disabled canonical scheduler；joint deployment default保持rank 0，Campaign safety hold默认held。
2. `shadow_registry`/`dual_read_compare`只运行无外部副作用evaluation，不创建canonical Campaign、待审批Prepared Action、execution、Finding、FactDelta或report authority。
3. Plan C 不修改任何production default，也不提供promotion/release-hold setter。authoritative path只由自动回滚fixture验证；显式canary和default promotion等待Plan D的local-admin协调器与独立授权。
4. singleton/default永不向后移动；已冻结operation永不改mode。紧急处理只能保持/设置append-only safety hold：停止新operation和新Campaign dispatch，允许已发生action安全closeout/recovery，并把未完成工作记为residual/inconclusive。
5. 修复通过新的前向contract/criteria发布；旧binary不能识别任一frozen mode时不得回滚部署。mismatch、coverage/promotion receipt与audit facts不删除、不重写。

## 计划完成自检

- [ ] 所有任务都有精确文件、RED/GREEN、命令、Expected 与未来 commit 边界。
- [ ] schema 修改有独立授权暂停点，当前计划未创建 migration。
- [ ] Plan C 自带可工作的 JIT API/UI，不依赖 Plan D 才能安全授权。
- [ ] no-action/denied/expired/unknown/cleanup/quarantine 路径均有 typed terminal/residual，不伪造 execution/oracle。
- [ ] Lead/subagent/host/DB 权限边界明确，raw target/secret/oracle/Finding 不归模型。
- [ ] Campaign local drain、Wave consolidation、Stage final seal 没有循环依赖。
- [ ] 单Campaign只产objective/claim-component outcome；只有B-owned plan exact outcome set的revision adjudicator能写verified/refuted与Finding/refutation。
- [ ] admission/begin/closeout/revision verdict均绑定Plan A multi-root all-fresh temporal bundle，不能caller过滤stale root或挑旧outcome。
- [ ] partial-overlap conflict keys与ConcurrentActionGroup可表达race类；adapter缺失不会被计为已覆盖。
- [ ] `complete`仅指declared/planned denominator；ThreatCoverageProfile未上线时全局coverage sufficiency保持not_assessed。
- [ ] legacy/shadow/dual modes 不授权新执行，operation mode 不被部署默认漂移。
- [ ] 无占位内容、占位测试或全仓大型门禁。
