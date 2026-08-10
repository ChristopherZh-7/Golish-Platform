# Target Intel Goal Loop 审计权威与 Cutover 实现计划

> Superseded by `2026-08-04-scoping-and-autonomous-corporate-asset-discovery.md`; the new production path has no legacy six-axis publication compatibility.

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在 Plan A Shadow 证据通过评审后，把 Target Intel 的完成 authority 从固定六轴 Gate切到冻结 Review Bundle、通用只读 Reviewer和确定性 Finalizer；只对用户批准后新建的 Red Team operation生效，Pentest hard-skip、历史 operation和其它阶段保持原样。

**架构：** 新增 immutable per-operation Goal合同、additive review authority/V2 frontier表和 Stage WorkItem host-owned execution profile。Goal owner在 request epoch关闭前调用 review barrier；数据库在一致快照上原子冻结四段 bundle并创建通用 reviewer WorkItem。Reviewer只能按顺序读取 durable state、observable actions、frozen contract、completion claim，最后提交 `intel_review.v1`。REWORK原子恢复同一 Controller chain，NEEDS_HUMAN形成可验证、可恢复的typed hold；PASS后不再调用LLM，而由宿主执行non-vacuous deterministic finalizer并在单事务生成兼容submission、发布Handoff/final seal和支持response-loss replay。

**技术栈：** Rust 2021、PostgreSQL/sqlx、rig-core 0.36、serde/serde_json、ts-rs、React 19/TypeScript 6、Vitest、Biome、现有 runtime-memory final seal/Handoff/StageTeam read model。

**设计依据：** [`../../design/2026-08-02-target-intel-goal-loop-and-audit.md`](../../design/2026-08-02-target-intel-goal-loop-and-audit.md)

**前置依赖：** `target-intel-goal-loop-shadow-2026-08-02` 必须为 `passing`，且 Shadow divergence/cost/safety报告已经由用户评审。

**必须重新取得的明确授权：** 开始 Task 1 前，用户必须明确批准：

1. 新建 `20260802000001_target_intel_goal_review.sql` 和 `20260802000002_target_intel_goal_frontier_scope.sql` 两个 additive migration；
2. 新增 immutable per-operation Goal/authority合同、Goal/review generation、finding resolution、typed hold/fulfillment、material revision、V2 frontier和 review authority表，并修改 `stage_work_items` additive columns及 StageTeam安全 trigger；`expansion_queue` legacy schema/unique保持不变；
3. 生成/更新 StageTeam review read-model IPC类型；
4. frozen seam部署后，分别批准两次 production profile mutation：先只让新 Red Team operation进入 non-blocking `observe_shadow`，完成 promotion报告后才允许新 operation进入 `intel_goal_v1`；
5. 若需要真实实体验收，另行批准 exact workspace、organization、provider/public-source清单、请求边界和费用上限。

任何一项未获批准时，本计划保持 `not_started`；不得先建 migration、手改 generated文件或暗中切 profile。

## Production 修订合同（2026-08-02 审阅后，优先级高于下文旧措辞）

本节是执行 Plan B 时的权威合同。下文任何步骤若仍可被理解为“读当前 profile 重解释历史 operation”“审计 PASS 后再让模型提交”“修改 legacy frontier unique”或“Shadow verdict 阻塞旧 Gate”，均按本节修正，不得采用旧解释。

### A. Operation-frozen authority 是第一依赖

在修改任何 production profile 前，必须先用 additive schema落一份完整、不可变、可回放的 per-operation合同：

```text
target_intel_goal_operation_contracts
  operation_id PK/FK
  profile_id
  runtime_mode
  completion_authority
  goal_contract_version
  canonical_goal_contract
  goal_contract_sha256
  methodology_payload + methodology_sha256
  tool_manifest + tool_manifest_sha256
  provider_capability_manifest + provider_capability_sha256
  browser_policy + budget_policy
  created_at
```

规则：

- 没有该 row 的历史 operation确定性解释为 `legacy_six_axis_v1`，不得按当前 `red_team.json` 补写或重解释；
- row与所有 payload/hash一经创建不可 UPDATE/DELETE；hash不足以恢复内容，必须同时保存 canonical payload，或保存可由 hash精确读取的 immutable content-addressed artifact；
- operation创建必须与该 row同事务提交；失败则 operation整体不创建；
- resume只读 frozen row；unknown mode/version/hash mismatch fail closed；
- operation fork必须在 fork transaction中显式冻结目标 operation合同。same-profile continuation复制 exact source合同；cross-profile fork由服务器根据已批准目标 profile生成新合同，禁止从运行时当前文件静默推断；
- runtime不得再用 profile id重载当前 embedded `intel_policy`决定历史行为；profile只参与新 operation合同构造；
- frozen mode至少为：
  - `observe_shadow`：旧 Controller/六轴 Gate/final seal完整照常运行；审计作为 detached、read-only、non-barrier sidecar，不能打回、hold、延迟或改变 pass token；
  - `advisory_rework`：Goal owner与通用 worker运行，reviewer可在独立 review fuel内建议 REWORK；最终 completion authority仍是 legacy Gate；reviewer `NEEDS_HUMAN`只形成明确 advisory residual，除非用户对该 operation另行批准 hold；
  - `intel_goal_v1`：review PASS + deterministic finalizer是唯一 Target Intel completion authority，六轴仅为兼容 projection；
- 生产默认 profile的两次 mutation必须都发生在上述 frozen seam已部署并验证之后：第一次经批准只把**新** Red Team operation切到 `observe_shadow`；完成真实/fixture divergence报告后，第二次经批准才把**新** Red Team operation切到 `intel_goal_v1`。`advisory_rework`先作为 fixture/显式批准 cohort验证，不暗中成为默认；
- schema/generated授权不等于任一次 production profile mutation批准；两次 mutation必须分别在聊天中获得明确确认。

### B. Goal epoch、review generation 与数据库状态机

Review不是现有 gate-repair的别名。必须新增 host-owned、DB-backed状态：

```text
GoalEpoch(open)
  -> freeze_for_review(CAS, epoch N sealed)
  -> ReviewGeneration(round R, bundle frozen)
      -> REWORK: DB atomically opens GoalEpoch N+1 on same Controller chain
      -> NEEDS_HUMAN: DB atomically creates typed hold
      -> PASS: DB marks exact review fresh-pass; epoch remains sealed
  -> deterministic finalizer/publication
```

约束：

- `stage_team_request_intel_review`发生在现有 final-submit closeout之前，但 freeze transaction必须原子封住当前 Goal dispatch epoch、证明 Controller/ordinary children/tool calls quiescent，并创建 review generation；review期间不能接受普通动态 worker；
- migration必须替换现有 `enforce_stage_team_plan_contract()`，只允许 exact fresh review `REWORK`凭 review generation/CAS把 `dispatch_epoch`推进 `N→N+1`；不得复用 `stage_team_repair_generations` 或 `reopen_stage_team_leader_after_gate_block`；
- REWORK在同一 transaction：终结 reviewer、写 findings/resolution events、推进 epoch、恢复同一个 Controller WorkItem/Worker/message chain并注入 server-owned finding message；response-loss exact replay返回同一 successor epoch；
- `same finding fingerprint + same material state/action digest` 和 `max_review_rounds`由 DB transaction在锁内判定并原子转 `NEEDS_HUMAN`，不能由 prompt、进程内计数或 caller自报决定；review fuel与 gate-repair fuel分离且写入 operation contract；
- `NEEDS_HUMAN`必须有 typed hold与 fulfillment ledger。增加 `resume_target_intel_goal_after_human(expected_review_id, expected_row_version, fulfillment_kind, authority_ref)`：只在 scope/credential/subject/provider等 requirement由可信入口满足时，CAS关闭 hold、恢复同 chain、打开新 Goal epoch并创建下一 review round；foreign/stale/free-text fulfillment拒绝；
- crash恢复必须选择唯一 Goal owner；ordinary worker、integrated reviewer和 detached Shadow reviewer均不得冒充 leader。

### C. 通用 SubAgent 不等于复用固定 recon prompt

- 模型可见普通派工始终只有 `name + prompt + subject_refs`；模型不能选择 role/kind/tool policy/profile/terminal schema；
- DB可保留 server-stamped technical kind，但它只用于恢复和权限绑定，不是业务 role，也不进入模型 schema/UI分类；
- 普通 Intel worker使用 host-owned neutral Intel worker system prompt；reviewer使用 host-owned neutral review prompt。两者复用同一个通用 SubAgent executor，但不得加载当前 `build_recon_prompt()`中的固定 provider→WHOIS→六轴→submit流程；
- `execution_profile`、`terminal_contract`、neutral prompt version/hash必须落在 immutable WorkItem/operation contract上。Migration必须替换 `enforce_stage_work_item_contract()`，把新列纳入 immutable ROW比较；任何 UPDATE切换 reviewer为普通 worker均在 DB层拒绝；
- authoritative reviewer只暴露 `stage_team_read_review_section` 与 exact-schema `submit_result`；ordinary worker与reviewer的 prompt/tool/terminal profile均由 host从 immutable row恢复，不能仅存在内存 context；
- `observe_shadow` reviewer使用 detached review job/outbox和独立 generic SubAgent chain，不加入 authoritative StageTeam barrier，也不依赖 operation仍停在 Target Intel；其失败只写 shadow failure，不阻塞 legacy stage。

### D. Bundle 必须是一致快照，四段读取必须可证明

Freeze transaction必须：

1. 先 park/fence Goal owner，等待 ordinary children、provider/browser/tool writer全部 terminal；
2. 使用 PostgreSQL `REPEATABLE READ`（必要时对相同 identity的竞争 freeze使用 SERIALIZABLE retry）构造单一 MVCC snapshot；
3. 持久化 operation/org/stage/plan/Goal epoch、各 material source的 revision vector与 high-water；至少覆盖 facts/targets/relations、evidence/artifacts、source query receipts、frontier v2、contradictions、worker requests/outputs、tool calls/results和 capability state；
4. 保存 canonical payload + section hash + final bundle hash；canonicalization只排序 object keys和明确声明为集合的字段，不能排序 action chronology、section ordinal或其它有序 array；
5. 原子创建 integrated reviewer WorkItem，或在 `observe_shadow` 下创建 detached non-barrier review job；
6. finalizer再次重建 revision vector、high-water和 canonical material digest。任何 drift都 supersede旧 PASS并回 Goal/new review，不能继续发布。

四段为：

1. `durable_state`
2. `observable_actions`
3. `frozen_contract`
4. `completion_claim`

`observable_actions`必须包含经过脱敏的 exact dynamic task `name + prompt + subject_refs + result refs`、semantic pivot、provider/browser/tool receipt和失败/重试；prompt hash只做完整性，不能替代审阅内容。不得依赖 transcript、CoT、count-only summary或 `sub_agent_dispatches`作为 authority。

Authoritative reviewer通过 DB cursor逐段读取：只有前一 ordinal的 read receipt成功提交后下一段才解锁；相同 worker/section/hash的 response-loss重放返回相同 payload，越序、foreign worker、不同 hash或 completion claim提前读取均 fail closed。Shadow prompt-only历史不能 promotion为 authoritative PASS证据。

### E. Finding resolution 与三态 verdict

`intel_review.v1`除新 findings/residuals外，必须对 bundle中每条 inherited open material finding逐条提交：

```json
{
  "finding_id": "uuid",
  "disposition": "resolved|still_open|needs_human",
  "resolution_refs": ["evidence/action/query/frontier refs"],
  "reason": "bounded explanation"
}
```

- finding本体不可变；关闭通过 append-only `target_intel_goal_review_finding_resolutions`事件表达，current status由事件投影；不得因下一轮“没有再次提到”而自动关闭；
- `resolved`必须引用 review后新增且属于 exact operation/org/current run的 material delta，并满足上一 finding close condition；
- `still_open`且 frozen capability内仍有 typed可行动作才可 REWORK；recommended action必须带 host-validatable `action_kind/capability_ref/subject_refs/close_condition`，不能只有 prose；
- `needs_human`必须匹配 typed requirement；
- PASS要求所有 inherited/new critical/major finding都有可验证 resolution，不允许 open material finding；minor/advisory只能作为明确 residual；
- verdict record对相同 review/worker/bundle/verdict hash exact replay，任何不同 payload fail closed。

### F. PASS 后绝不再进入 LLM

`intel_goal_v1`严格执行：

```text
fresh reviewer PASS
  -> host loads exact review material
  -> pure deterministic finalizer decision
  -> host generates compatibility slim StageDeliverable/submission
  -> one DB transaction publishes submission + org completion + Handoff + final seal
```

- 不绑定 Controller做新的模型 final turn，不调用 `execute_company_controller_final_turn`，不让模型在审计后修改 completion claim；
- compatibility deliverable完全由 server从 exact review/facts/refs生成，prose/count/model confidence不提供 authority；
- finalizer non-vacuity至少要求：Goal contract和四段完整；存在 current-run、exact org、有效 terminal query/action receipts；receipt引用的 evidence/artifact真实存在且 outcome/landing refs一致；每个 material frontier/contradiction均被host验证为resolved/rejected-noise/third-party，或blocked/unsupported但绑定operation-frozen policy/human waiver及替代路径证据；`needs_human`、pending、retryable error和无waiver的material blocked/unsupported一律不能PASS；同时要求无active authoritative worker/tool、无scope/candidate越权、无open material finding且review/material revision仍fresh；
- “有一条 evidence”“count>0”“模型说完成”“全部 blocked但没检查替代 capability”均不能 PASS；合法零发现必须由有效 current-run查询回执、evidence和闭合 frontier证明；
- publication transaction写 `review_id/bundle_sha256/verdict_sha256/operation_contract_sha256` 到 submission、Handoff和 final-seal attestation，并提供 response-loss exact replay；
- finalizer BLOCK必须原子把旧 PASS标 stale/superseded、写 server finding、恢复同 Goal chain并要求全新 review；不得进入 legacy gate repair，也不得在旧 PASS上就地补 submission。

### G. Legacy frontier只保留，新建 V2 authority

- 不删除或修改 `expansion_queue`现有 unique/状态合同；它继续作为 legacy/Shadow mirror，避免 migration因历史碰撞或旧 binary写入失败；
- 新建 org-scoped `target_intel_goal_frontier_v2`，拥有 immutable semantic identity、status transition、row_version/CAS、provenance和material revision；只有它能进入 `intel_goal_v1` bundle/finalizer；
- mirror失败可观测但不能让 legacy queue反向覆盖 v2 authority；candidate/frontier永远不直接授予 active scope。

---

## 实施前状态

1. 完整读取 `AGENTS.md`、progress/feature、本设计、Plan A证据、本计划和以下模块卡：
   - `docs/modules/backend/golish-agent-kit/db_traits.md`
   - `docs/modules/backend/golish-agent-kit/harness.md`
   - `docs/modules/backend/golish-db.md`
   - `docs/modules/backend/golish-db/repo.md`
   - `docs/modules/backend/golish-agent-app/ai.md`
   - `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
   - `docs/modules/backend/golish-sub-agents/executor.md`
   - `docs/modules/backend/golish/stage_run.md`
   - `docs/modules/frontend/components.md`
   - `docs/modules/frontend/lib.md`
2. 检查两个预留 migration文件仍不存在、timestamp未冲突；若冲突，停止并修订设计/计划，不得自行换号。
3. 确认唯一 active feature规则后，把 `target-intel-goal-loop-audit-cutover-2026-08-02` 改为 `in_progress`，在 progress记录用户授权原文和本轮定向验证。
4. 记录共享 dirty tree；每 Task精确暂存，禁止 `git add -A`。
5. 所有 Cargo build/test/clippy之前执行 `just space-guard`。默认不运行未获授权的 init/precommit/full suites。

---

## Task 1：定义 Review Authority 纯 Rust 合同

**Files:**

- Add: `backend/crates/golish-agent-kit/src/harness/intel_goal_contract.rs`
- Add: `backend/crates/golish-agent-kit/src/harness/intel_goal_review.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/mod.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor_types.rs`
- Test: inline tests in both files

### Step 1：写 RED tests

```rust
#[test]
fn review_bundle_hash_is_stable_and_section_order_is_fixed() {
    let bundle = fixture_bundle();
    assert_eq!(bundle.sections.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![
        IntelReviewSectionKind::DurableState,
        IntelReviewSectionKind::ObservableActions,
        IntelReviewSectionKind::FrozenContract,
        IntelReviewSectionKind::CompletionClaim,
    ]);
    assert_eq!(bundle.canonical_sha256(), fixture_bundle().canonical_sha256());
}

#[test]
fn review_canonicalization_preserves_action_and_section_array_order() {
    let first = fixture_bundle_with_actions(["query-a", "query-b"]);
    let reordered = fixture_bundle_with_actions(["query-b", "query-a"]);
    assert_ne!(first.canonical_sha256(), reordered.canonical_sha256());
}

#[test]
fn missing_operation_contract_is_legacy_and_unknown_mode_fails_closed() {
    assert_eq!(IntelGoalRuntimeMode::for_missing_row(), IntelGoalRuntimeMode::Legacy);
    assert!(IntelGoalRuntimeMode::parse("future_mode").is_err());
}

#[test]
fn review_rework_requires_action_and_close_condition() {
    assert!(IntelReviewVerdict::parse(rework_fixture()).is_ok());
    assert!(IntelReviewVerdict::parse(rework_missing_action()).is_err());
    assert!(IntelReviewVerdict::parse(rework_missing_close_condition()).is_err());
}

#[test]
fn review_pass_rejects_open_major_findings() {
    assert_eq!(
        validate_review_for_finalizer(pass_with_open_major()).unwrap_err().code(),
        "INTEL_REVIEW_OPEN_MATERIAL_FINDING"
    );
}

#[test]
fn pass_must_dispose_every_inherited_material_finding_with_resolution_refs() { /* exact ids */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit -p golish-sub-agents -E 'test(review_bundle_) | test(review_canonicalization_) | test(missing_operation_contract_) | test(review_rework_) | test(review_pass_) | test(pass_must_dispose_)' --status-level fail
```

### Step 3：实现类型

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelReviewSectionKind {
    DurableState,
    ObservableActions,
    FrozenContract,
    CompletionClaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelReviewDecision {
    Pass,
    Rework,
    NeedsHuman,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelGoalRuntimeMode {
    Legacy,
    ObserveShadow,
    AdvisoryRework,
    IntelGoalV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAgentExecutionProfile {
    Worker,
    ReadOnlyReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageAgentTerminalContract {
    WorkerOutputV1,
    IntelReviewV1,
}
```

规则：

- canonical JSON只排序 object key与显式 set-like字段；section/action等有序 array保持原顺序并进入 SHA-256；
- operation contract保存 canonical payload和hash，缺 row只解释为 legacy，unknown mode/version fail closed；
- bundle identity包含 operation/stage execution/unit/plan/org/controller worker/chain/round/state revision；
- completion claim只是一段输入，不产生 facts；
- finding fingerprint由 host以 materiality、subject refs、typed action/capability ref、reason和close condition重算；
- verdict必须逐条 disposition inherited material findings；`resolved`必须带 resolution refs，不能以本轮缺席推断关闭；
- PASS不得携带 open critical/major finding；
- REWORK至少一条 critical/major finding且每条有可执行 action/close condition；
- NEEDS_HUMAN必须声明 `credential|scope_confirmation|subject_confirmation|provider_recovery|review_fixed_point` 中至少一种 requirement；
- execution profile与terminal contract为 host-owned，不反序列化模型自报值。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit -p golish-sub-agents -E 'test(review_bundle_) | test(review_canonicalization_) | test(missing_operation_contract_) | test(review_rework_) | test(review_pass_) | test(pass_must_dispose_) | test(intel_review_)' --status-level fail
cargo clippy -p golish-agent-kit -p golish-sub-agents --lib --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-sub-agents -- --check
```

### Step 5：提交

```bash
git add backend/crates/golish-agent-kit/src/harness/intel_goal_contract.rs backend/crates/golish-agent-kit/src/harness/intel_goal_review.rs backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-sub-agents/src/executor_types.rs
git commit -m "feat(intel): define goal review authority"
```

---

## Task 2：新增 Operation Contract、Review Generation 与 V2 Frontier Additive Schema

**Files:**

- Add: `backend/crates/golish-db/migrations/20260802000001_target_intel_goal_review.sql`
- Add: `backend/crates/golish-db/migrations/20260802000002_target_intel_goal_frontier_scope.sql`
- Add: `backend/crates/golish-db/tests/target_intel_goal_review_migrations.rs`
- Modify: `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- Read for trigger compatibility only: `backend/crates/golish-db/migrations/20260714000003_stage_team_scheduler.sql`；不回改历史 migration，新 migration用 `CREATE OR REPLACE FUNCTION`升级 trigger

### Step 1：先写 migration RED tests

测试必须在全新隔离 PostgreSQL 中断言：

- immutable operation contract、Goal epoch、review generation/sections/findings/resolutions、typed hold/fulfillment、material revision、detached review job与 V2 frontier表及 compound FKs/triggers存在；
- `stage_work_items` 新 columns默认不改变旧 rows；
- 新 execution profile/terminal contract/prompt version列在 WorkItem UPDATE时不可变；
- operation contract UPDATE/DELETE拒绝；历史 operation缺 row确定性走 legacy；
- review只能绑定 TargetIntel plan和同一个 operation/execution/unit/scope/org；
- section read只允许 ordinal 1→2→3→4；
- verdict不能在未完成四段读取时写入；
- review/finding/section read不可删除，sealed payload不可改；
- cross-org/cross-plan/cross-worker绑定失败；
- exact review REWORK是唯一新增的合法 `dispatch_epoch N→N+1` transition；无 review generation、stale version或 gate-repair冒充均失败；
- legacy `expansion_queue` schema、unique和历史 rows逐字保持；两个组织的同 pivot写入新 `target_intel_goal_frontier_v2`互不冲突；
- finding resolution只能追加事件，不能 UPDATE finding本体；typed hold fulfillment要求 exact authority ref与row version。

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test target_intel_goal_review_migrations --status-level fail
```

预期：migration文件/表/columns不存在。

### Step 3：实现 migration 00001（operation/review authority + 安全 trigger）

创建 `target_intel_goal_operation_contracts`。它保存完整 canonical Goal/methodology/tool/provider/browser/budget payload与hash、`runtime_mode`、`completion_authority`和review fuel；`operation_id`唯一且 UPDATE/DELETE一律拒绝。旧 operation不回填；absence由代码解释为 legacy。

创建 `target_intel_goal_epochs`、`target_intel_goal_reviews`、`target_intel_goal_review_section_reads`、`target_intel_goal_review_findings`、`target_intel_goal_review_finding_resolutions`、`target_intel_goal_holds`、`target_intel_goal_hold_fulfillments`、`target_intel_goal_material_revisions` 和 `target_intel_goal_review_jobs`。Integrated review绑定 StageTeam/Controller；`observe_shadow` detached job绑定 sealed legacy snapshot但不加入 StageTeam barrier。

`stage_work_items` additive columns：

```sql
ALTER TABLE stage_work_items
  ADD COLUMN execution_profile TEXT NOT NULL DEFAULT 'worker',
  ADD COLUMN terminal_contract TEXT NOT NULL DEFAULT 'worker_output_v1',
  ADD COLUMN display_name TEXT,
  ADD COLUMN task_prompt_sha256 TEXT,
  ADD COLUMN host_prompt_version TEXT,
  ADD COLUMN host_prompt_sha256 TEXT;
```

检查：

- `execution_profile IN ('worker','read_only_reviewer')`；
- `terminal_contract IN ('worker_output_v1','intel_review_v1')`；
- reviewer必须 `terminal_contract='intel_review_v1'`，普通 worker不得使用该 contract；
- `display_name` trim后1..80字符或NULL；
- `task_prompt_sha256`为 `sha256:` + 64 lowercase hex或NULL；
- 现有 row默认 `worker/worker_output_v1`，无 rewrite业务语义。
- 新 migration必须 `CREATE OR REPLACE FUNCTION enforce_stage_work_item_contract()`，把以上所有新列加入 immutable identity ROW比较；补直接 SQL UPDATE profile/contract/prompt hash被拒测试。
- 新 migration必须 `CREATE OR REPLACE FUNCTION enforce_stage_team_plan_contract()`：保留现有 gate-repair规则，并只新增 exact fresh review generation驱动的 epoch advance；其它 reopen仍拒绝。

`target_intel_goal_reviews`：

- identity：`id, operation_id, stage_execution_id, stage_run_unit_id, scope_snapshot_id, organization_id, team_plan_id, controller_work_item_id, controller_worker_run_id, controller_message_chain_id, round`；
- section payload/hash：`durable_state, durable_state_sha256, observable_actions, observable_actions_sha256, frozen_contract, frozen_contract_sha256, completion_claim, completion_claim_sha256, bundle_sha256`；
- identity额外绑定 `goal_epoch`、`review_generation`、operation contract id/hash和 material revision vector/high-water；
- state：`status IN ('building','frozen','reviewing','rework','pass','needs_human','stale','superseded')`；
- reviewer refs：`reviewer_work_item_id, reviewer_worker_run_id`；
- verdict：`verdict, verdict_sha256, material_state_sha256, material_actions_sha256, created_at, frozen_at, terminal_at, row_version`；
- unique `(team_plan_id, round)` 和 exact compound FKs到 `stage_team_plans`、`stage_work_items`、`stage_worker_runs`；
- DB trigger只允许 server transaction执行 `building→frozen→reviewing→terminal`；terminal payload immutable，stale/superseded通过 append-only transition/event表达。

创建 `target_intel_goal_review_section_reads`：

- `review_id, reviewer_worker_run_id, section_ordinal, section_kind, section_sha256, read_at`；
- PK `(review_id, section_ordinal)`；
- trigger要求 exact reviewer、frozen/reviewing review、ordinal连续、kind固定；
- completion claim只能 ordinal 4。

`target_intel_goal_review_findings` 与 resolution ledger：

- `id, review_id, fingerprint, materiality, subject_refs, reason, recommended_action, close_condition, status, resolution_evidence_ids, resolved_by_review_id, created_at, resolved_at`；
- status `open|resolved|accepted_residual|superseded`；
- fingerprint在同 review唯一；
- finding immutable；后续 review只能向 `target_intel_goal_review_finding_resolutions`追加 `resolved|still_open|needs_human`事件与 exact refs；current status由投影计算；
- critical/major不能 `accepted_residual` 后 final PASS。

### Step 4：实现 migration 00002

- **不 ALTER/删除/替换** legacy `expansion_queue` unique、status或历史 row；
- 新建 `target_intel_goal_frontier_v2`，identity至少含 operation/org/stage execution/Goal epoch/pivot kind/value hash/intent/provenance；
- unique按 `(operation_id, organization_id, semantic_pivot_key)`，两个组织相同 pivot互不冲突；
- status为 `pending|in_progress|resolved|blocked|unsupported|needs_human|rejected_noise|third_party|ambiguous`，每次 transition要求 expected `row_version`；
- material transition在同事务 bump `target_intel_goal_material_revisions.state_revision`；
- legacy queue仅由 best-effort mirror writer更新，mirror失败可观测但不能覆盖 V2 row、scope或PASS authority。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test target_intel_goal_review_migrations --status-level fail
just space-guard
cargo nextest run -p golish-db --test runtime_memory_worker_transactions -E 'test(target_intel_goal_review_)' --status-level fail
cargo clippy -p golish-db --lib --tests -- -D warnings
cargo fmt -p golish-db -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-db/migrations/20260802000001_target_intel_goal_review.sql backend/crates/golish-db/migrations/20260802000002_target_intel_goal_frontier_scope.sql backend/crates/golish-db/tests/target_intel_goal_review_migrations.rs backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs
git commit -m "feat(db): add target intel review authority"
```

---

## Task 3：实现 Repository、CAS 与 Atomic Review Transitions

**Files:**

- Add: `backend/crates/golish-db/src/repo/target_intel_goal_reviews.rs`
- Add: `backend/crates/golish-db/src/repo/target_intel_goal_contracts.rs`
- Add: `backend/crates/golish-db/src/repo/target_intel_goal_frontier.rs`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Modify only for best-effort mirror, never authority/schema: `backend/crates/golish-db/src/repo/expansion_queue.rs`
- Modify: `backend/crates/golish-db/src/repo/operation_state.rs`
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-db/src/repo/stage_teams.rs`
- Modify: `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/start_operation_tool.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/stage_fork.rs`
- Modify: `backend/crates/golish/src/stage_run/runtime_v2.rs`
- Test: `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

### Step 1：写 transaction RED tests

至少包含：

```rust
#[tokio::test]
async fn target_intel_review_freeze_is_atomic_with_reviewer_work_item() { /* all-or-nothing */ }

#[tokio::test]
async fn target_intel_review_section_cursor_rejects_skips_and_foreign_worker() { /* 1..4 */ }

#[tokio::test]
async fn target_intel_rework_reopens_same_controller_chain_and_preserves_findings() { /* CAS */ }

#[tokio::test]
async fn target_intel_pass_rejects_state_or_action_digest_drift() { /* stale */ }

#[tokio::test]
async fn target_intel_needs_human_persists_hold_without_pass_token() { /* hold */ }

#[tokio::test]
async fn target_intel_same_finding_without_material_delta_atomically_holds() { /* DB lock */ }

#[tokio::test]
async fn target_intel_human_fulfillment_resumes_same_chain_once() { /* CAS + replay */ }

#[tokio::test]
async fn target_intel_review_bundle_uses_one_repeatable_read_snapshot() { /* concurrent writer */ }

#[tokio::test]
async fn operation_create_resume_and_fork_use_only_immutable_intel_contract() { /* no profile reload */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test runtime_memory_worker_transactions -E 'test(target_intel_review_) | test(target_intel_rework_) | test(target_intel_pass_) | test(target_intel_needs_human_) | test(target_intel_same_finding_) | test(target_intel_human_fulfillment_) | test(operation_create_resume_and_fork_)' --status-level fail
```

### Step 3：扩展 runtime-memory trait

增加 request/view/method：

```rust
async fn freeze_target_intel_review(
    &self,
    input: FreezeTargetIntelReview,
) -> Result<FrozenTargetIntelReviewView, RuntimeMemoryError>;

async fn read_target_intel_review_section(
    &self,
    input: ReadTargetIntelReviewSection,
) -> Result<TargetIntelReviewSectionView, RuntimeMemoryError>;

async fn record_target_intel_review_verdict(
    &self,
    input: RecordTargetIntelReviewVerdict,
) -> Result<RecordedTargetIntelReviewView, RuntimeMemoryError>;

async fn resume_target_intel_goal_after_rework(
    &self,
    input: ResumeTargetIntelGoalAfterRework,
) -> Result<ResumedTargetIntelGoalView, RuntimeMemoryError>;

async fn resume_target_intel_goal_after_human(
    &self,
    input: ResumeTargetIntelGoalAfterHuman,
) -> Result<ResumedTargetIntelGoalView, RuntimeMemoryError>;

async fn finalize_target_intel_goal_pass(
    &self,
    input: FinalizeTargetIntelGoalPass,
) -> Result<FinalizedStageTeamUnitView, RuntimeMemoryError>;
```

### Step 4：实现 DB transaction

统一锁顺序：operation → immutable Intel contract → Stage Unit → TeamPlan → Goal epoch → Controller WorkItem/Worker → material revision → Review → Reviewer WorkItem/Worker → findings/resolutions/hold/frontier。

- operation create先从已批准 launch policy构造完整 canonical contract，并与 operation row同事务插入；resume/fork只读或显式复制该 row；missing row走 legacy，禁止 current profile fallback；
- freeze先 fence/park Controller并证明 ordinary child/tool quiescent，再在 `REPEATABLE READ` transaction从 DB read model构造四段；caller只传 identity/fence/completion claim，不传 durable facts/actions/contract hash；
- bundle保存完整、脱敏的 dynamic name/prompt/subjects/result refs及 material revision vector/high-water；action chronology不得 canonical sort；
- 同一 round exact replay返回原 review，不重复 WorkItem；任一 payload/hash漂移返回 `target_intel_review_replay_mismatch`；
- freeze和创建 reviewer WorkItem同事务；
- reviewer claim必须匹配 `read_only_reviewer/intel_review_v1`；
- section read插入 receipt后才返回 payload；
- verdict transaction重算 section reads、bundle hash、worker lease/terminal output和finding fingerprint；
- REWORK写 immutable findings/append-only resolution events、park reviewer、通过 review-generation trigger把 Goal epoch/dispatch epoch推进一代，并原子恢复同 Controller chain；
- DB在同一锁内比较 previous fingerprint/material digests与 frozen max rounds；fixed point或fuel耗尽直接写 typed NEEDS_HUMAN，caller无权覆盖；
- PASS只把 review标 terminal；保持 freeze时已封住的 Goal/dispatch epoch，不 reopen、不绑定模型 final submitter，也不在 verdict transaction发布 final；
- NEEDS_HUMAN关闭本轮 reviewer并写可恢复 typed hold，不改变旧 facts；可信 fulfillment CAS后才可同 chain创建 successor Goal epoch；
- authoritative verdict相同 payload/hash response-loss exact replay，不同 payload fail closed；
- V2 frontier `list/claim/transition` 全部按 operation/org/row_version并 bump material revision，不能成为 scope写入；legacy expansion queue只做 mirror；
- `observe_shadow` freeze只入 detached review job/outbox，绝不进入 StageTeam barrier或改变 legacy finalization。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-db --test runtime_memory_worker_transactions -E 'test(target_intel_review_) | test(target_intel_rework_) | test(target_intel_pass_) | test(target_intel_needs_human_) | test(target_intel_same_finding_) | test(target_intel_human_fulfillment_) | test(operation_create_resume_and_fork_)' --status-level fail
just space-guard
cargo nextest run -p golish-agent-app -E 'test(target_intel_review_bridge_)' --status-level fail
cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app --lib --tests -- -D warnings
cargo fmt -p golish-db -p golish-agent-kit -p golish-agent-app -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-db/src/repo/target_intel_goal_contracts.rs backend/crates/golish-db/src/repo/target_intel_goal_reviews.rs backend/crates/golish-db/src/repo/target_intel_goal_frontier.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/expansion_queue.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/stage_teams.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs backend/crates/golish-agent-app/src/ai/start_operation_tool.rs backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs backend/crates/golish-agent-app/src/ai/stage_fork.rs backend/crates/golish/src/stage_run/runtime_v2.rs backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs
git commit -m "feat(intel): persist review cycle transactions"
```

---

## Task 4：实现 Host-controlled Section Reader 与只读 Reviewer Profile

**Files:**

- Modify: `backend/crates/golish-sub-agents/src/executor_types.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/inner.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/prompt_assembly.rs`
- Modify: `backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Test: inline tests in the same files

### Step 1：写工具面 RED tests

```rust
#[test]
fn readonly_reviewer_exposes_only_section_reader_and_submit_result() {
    let tools = tools_for_profile(StageAgentExecutionProfile::ReadOnlyReviewer);
    assert_eq!(tool_names(tools), vec!["stage_team_read_review_section", "submit_result"]);
}

#[test]
fn reviewer_cannot_read_completion_claim_before_prior_sections() { /* typed error */ }

#[test]
fn intel_review_terminal_contract_rejects_prose_and_wrong_schema() { /* barrier */ }

#[test]
fn generic_intel_worker_and_reviewer_use_neutral_host_prompts_not_recon_role_prompt() {
    for prompt in [generic_intel_worker_prompt(), readonly_reviewer_prompt()] {
        assert!(!prompt.contains("complete all six coverage axes"));
        assert!(!prompt.contains("recon_map_assets"));
        assert!(!prompt.contains("submit_stage_deliverable"));
    }
}
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(readonly_reviewer_) | test(reviewer_cannot_) | test(intel_review_terminal_) | test(generic_intel_worker_and_reviewer_)' --status-level fail
```

### Step 3：实现 execution profile

- `BoundWorkerChainContext`携带 host-owned execution profile/terminal contract；
- 普通 Intel Worker使用 host-owned neutral Intel prompt、动态 task prompt、最小 passive工具和 `WorkerOutputV1`；不加载静态 `recon` role prompt/skill；
- Reviewer只给 `stage_team_read_review_section` 与 exact-schema `submit_result`；
- Reviewer使用 host-owned neutral review prompt；`name`仅显示，不能选择权限或方法论；
- profile/terminal/prompt version+hash从 immutable WorkItem与 operation contract恢复，不能只放在进程内 context；
- section reader参数只收 `review_id` 和请求的 `section_kind`，exact worker/round/bundle/fence来自 bound context；
- host每次调用 DB cursor后才返回 section；越序、重复不同hash、foreign/stale均 typed fail closed；
- completion claim读完前禁止 submit_result；
- terminal request使用 `ToolChoice::Required`，唯一 terminal工具为 `submit_result`；
- reviewer prompt先说明审计问题和 verdict契约，不预注入 completion claim；
- reviewer output parser保留 finding/reason/action/close condition，丢弃无权威 prose；
- reviewer任何工具越权尝试写 audit evidence并终止本 attempt为 typed `review_contract_failure`；按 operation-frozen reviewer retry fuel恢复 exact review，耗尽后才可形成 `NEEDS_HUMAN(provider_recovery)`，绝不自动降级普通 Worker。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-sub-agents -p golish-agent-runtime -E 'test(readonly_reviewer_) | test(reviewer_cannot_) | test(intel_review_terminal_) | test(generic_intel_worker_and_reviewer_) | test(bound_terminal_)' --status-level fail
cargo clippy -p golish-sub-agents -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt -p golish-sub-agents -p golish-agent-runtime -- --check
```

### Step 5：提交

```bash
git add backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/tool_setup.rs backend/crates/golish-sub-agents/src/executor/inner.rs backend/crates/golish-sub-agents/src/executor/prompt_assembly.rs backend/crates/golish-sub-agents/src/executor/response_parsing.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs
git commit -m "feat(intel): enforce ordered readonly review"
```

---

## Task 5：把 Review Barrier 接入 Persistent Goal Loop

**Files:**

- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- Modify: `backend/crates/golish/src/stage_run/runtime_v2.rs`
- Test: inline tests in the same files

### Step 1：写 lifecycle RED tests

```rust
#[tokio::test]
async fn company_controller_review_pass_enters_final_turn_only_after_fresh_verdict() { /* PASS */ }

#[tokio::test]
async fn company_controller_review_rework_resumes_same_message_chain() { /* REWORK */ }

#[tokio::test]
async fn company_controller_review_needs_human_stays_resumable_without_final_submitter() { /* hold */ }

#[tokio::test]
async fn company_controller_human_fulfillment_resumes_same_chain_in_successor_epoch() { /* CAS */ }

#[tokio::test]
async fn company_controller_crash_after_freeze_claims_same_reviewer_work_item() { /* recovery */ }

#[tokio::test]
async fn stage_team_resume_selects_unique_controller_with_reviewer_sibling() { /* CLI */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish -E 'test(company_controller_review_) | test(stage_team_resume_selects_unique_controller_with_reviewer_sibling)' --status-level fail
```

### Step 3：实现新 Controller turn

```rust
enum CompanyControllerTurn {
    Dispatched,
    RequestReview,
    AwaitHostFinalization,
}
```

流程：

- `stage_team_request_intel_review`发生在模型 final-submit之前；DB freeze原子 park/fence同一 Controller、封住当前 Goal/dispatch epoch并证明 ordinary work quiescent，不绑定 final submitter；
- scheduler claim动态 reviewer WorkItem并执行只读 contract；
- REWORK调用 DB review-generation transition，把 findings作为 server-owned next message恢复同一 chain并推进 epoch `N→N+1`；
- PASS后 Controller保持 parked/terminal producer状态；runtime直接调用 Task 6 host finalizer/publication seam，**不**绑定 Controller final submitter、**不**进入 `execute_company_controller_final_turn`、不再发起 LLM请求；
- NEEDS_HUMAN保留 Goal/plan/findings/frontier并让 runtime显示等待人工；只有 typed fulfillment CAS可恢复同 chain/new epoch；
- resume时唯一 primary仍是 Company Controller，reviewer/ordinary child都是 sibling；
- 不复用 `reopen_stage_team_leader_after_gate_block`，该函数仍只处理 final submit后的 deterministic Gate repair；
- review fuel与 legacy gate-repair fuel分开；same-fingerprint/no-delta和耗尽判定都在 DB transaction中转 NEEDS_HUMAN；
- `observe_shadow`不走本 integrated loop：它从 detached review outbox执行，任何 verdict/failure均不能等待、恢复或 hold legacy Controller；`advisory_rework`可以恢复 Goal但最终仍走 legacy Gate；`intel_goal_v1`才进入 host finalizer。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-runtime -p golish -E 'test(company_controller_review_) | test(stage_team_resume_selects_unique_controller_with_reviewer_sibling) | test(company_controller_) | test(stage_team_)' --status-level fail
cargo clippy -p golish-agent-runtime -p golish --lib --bins --tests -- -D warnings
cargo fmt -p golish-agent-runtime -p golish -- --check
```

### Step 5：提交

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish/src/stage_run/runtime_v2.rs
git commit -m "feat(intel): route goal completion through review"
```

---

## Task 6：实现 `intel_goal_v1` Host-only Deterministic Finalizer 与 Atomic Publication

**Files:**

- Add: `backend/crates/golish-agent-kit/src/harness/intel_goal_finalizer.rs`
- Modify: `backend/crates/golish-agent-kit/src/harness/mod.rs`
- Modify: `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- Add: `backend/crates/golish-agent-app/src/ai/target_intel_goal_finalizer.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- Test: add `backend/crates/golish-agent-app/tests/target_intel_goal_finalizer.rs`
- Test: modify `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

### Step 1：写 finalizer RED matrix

必须覆盖：

- fresh exact PASS通过；
- reviewer未按四段读取、wrong profile/contract、wrong org/plan/round拒绝；
- review后新增 receipt/evidence/frontier/material action导致 stale拒绝；
- active child/reviewer/tool存在时拒绝；
- foreign/missing/stale evidence拒绝；
- candidate被错误 promotion为active scope拒绝；
- open critical/major finding拒绝；
- NEEDS_HUMAN/REWORK拒绝；
- six-axis technique rows全空但 review/evidence完整时可通过；
- six-axis全绿但无 review时拒绝；
- response-loss exact replay返回同一 Handoff/review hash；drift拒绝。
- 零 evidence/count-only/prose-only completion拒绝；
- 合法零发现必须有 current-run有效 terminal query/action receipts、对应 evidence/artifact和闭合 frontier；
- 任一 material frontier/contradiction仍 pending/in_progress或 blocked但未核对 capability/替代路径时拒绝；
- finalizer BLOCK supersede旧 PASS并恢复同 Goal/new review，不进入 gate repair；
- PASS路径没有任何 provider/LLM调用，且不创建 Controller final submitter。

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_finalizer --status-level fail
just space-guard
cargo nextest run -p golish-db --test runtime_memory_worker_transactions -E 'test(target_intel_goal_finalizer_)' --status-level fail
```

### Step 3：实现 pure finalizer decision

`IntelGoalFinalizerMaterial`只接收 server-loaded operation contract、review、section receipts、material revision/high-water、facts/relations、current-run query/action receipts、evidence/artifacts、V2 frontier/contradictions、finding resolution projection和worker/tool state；不接收模型 deliverable。输出：

```rust
pub enum IntelGoalFinalizerDecision {
    Pass { review_id: Uuid, review_bundle_sha256: String, verdict_sha256: String },
    Block { code: String, reason: String, finding_refs: Vec<Uuid> },
}
```

### Step 4：接入 host-only final-seal/publication transaction

- operation-frozen `intel_completion_authority`分支：`legacy_six_axis_v1`继续旧 Gate；`intel_goal_v1`调用新 finalizer；
- server从 exact immutable operation contract与Review/sections/findings/resolutions/receipts/evidence/workers/V2 frontier重建 material，并复算 MVCC freeze记录的 revision vector/high-water/canonical digests；
- 不读取模型 coverage矩阵决定 PASS；
- server生成 compatibility slim `StageDeliverable`/submission，内容只有 exact server facts/refs/hash，只用于复用现有 Handoff read model；模型 prose/count/confidence不进入 authority；
- `finalize_target_intel_goal_pass`在**一个 DB transaction**中重验 material、插入兼容 submission、org completion、Handoff catalog delivery和 `runtime_memory_final_seal_attestation`，全部绑定 `review_id/bundle_sha256/verdict_sha256/operation_contract_sha256`；
- final seal重验无 newer material write、无 active authoritative worker/tool、无 open material finding、所有 material frontier/contradiction已合法终结，并要求至少一条 current-run有效 receipt/evidence closure；
- response-loss replay重算 material并要求 exact review/final identity；
- finalizer BLOCK在 DB中把 review PASS标 stale/superseded、追加 server-owned finding、恢复同 Goal chain并强制新 review；不调用 `reopen_stage_team_leader_after_gate_block`；
- `harness_submit_tool`在 `intel_goal_v1`拒绝模型直接提交 Target Intel deliverable；`execute_company_controller_final_turn`只服务 legacy/advisory路径；
- legacy TargetIntel、Pentest hard-skip和其它 stage不进入新分支。

### Step 5：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_finalizer --status-level fail
just space-guard
cargo nextest run -p golish-db --test runtime_memory_worker_transactions -E 'test(target_intel_goal_finalizer_) | test(finalize_unit_pass)' --status-level fail
just space-guard
cargo nextest run -p golish-agent-kit -p golish-agent-runtime -E 'test(intel_goal_finalizer_) | test(target_intel_goal_)' --status-level fail
cargo clippy -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-agent-runtime -- --check
```

### Step 6：提交

```bash
git add backend/crates/golish-agent-kit/src/harness/intel_goal_finalizer.rs backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-app/src/ai/target_intel_goal_finalizer.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-app/tests/target_intel_goal_finalizer.rs backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs
git commit -m "feat(intel): finalize reviewed goal authority"
```

---

## Task 7：暴露最小审计 Read Model 与 UI

**Files:**

- Modify: `backend/crates/golish-agent-app/src/ai/commands/stage_team.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/mod.rs`
- Regenerate/Modify: `frontend/lib/generated/StageTeamUnitView.ts`
- Regenerate/Modify: `frontend/lib/generated/StageTeamWorkItemView.ts`
- Regenerate/Add: `frontend/lib/generated/TargetIntelGoalReviewReadView.ts`
- Regenerate/Add: `frontend/lib/generated/TargetIntelGoalReviewFindingView.ts`
- Regenerate/Add: `frontend/lib/generated/TargetIntelGoalReviewResidualView.ts`
- Regenerate/Add: `frontend/lib/generated/TargetIntelGoalHoldView.ts`
- Modify: `frontend/lib/api/stage-team.ts`
- Modify: `frontend/components/Engagement/StageTeamRunView.tsx`
- Modify: `frontend/components/Engagement/StageTeamRunView.test.tsx`
- Test: Rust inline tests in `stage_team.rs`

### Step 1：写 read-model RED tests

后端断言：

- request必须通过 operation/project ownership；
- 返回 frozen runtime mode/authority、Goal status、contract version/hash、current epoch/round、review verdict、bundle hash、finding resolution status/subject/typed action/close condition、residual和typed hold requirement；
- 不返回 section raw payload、provider secret、cookie、CoT或完整页面内容；
- dynamic child显示 `display_name/prompt_sha256/subject_refs`，不把内部 role渲染成业务分类；
- historical review可审计但 current pointer唯一。

前端 Vitest断言 loading/error/empty/Goal active/reviewing/rework/needs-human/pass六态；PASS显示review hash，REWORK显示可执行建议和关闭条件，NEEDS_HUMAN只显示 server-returned typed requirement与可信 fulfillment入口，不提供自由文本绕过CAS。

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-app -E 'test(target_intel_goal_review_read_model_)' --status-level fail
cd ..
pnpm exec vitest run frontend/components/Engagement/StageTeamRunView.test.tsx
```

### Step 3：实现并生成类型

- 在 Rust DTO上使用 `#[derive(ts_rs::TS)]`；
- Goal status、verdict和finding status沿用当前 read-model 的受控字符串表示，不额外生成枚举文件；
- 从仓库根运行唯一正式的 `just gen-types`，只接受上方列出的六个 generated文件发生预期变化，绝不手写 generated文件；
- 前端只经 `frontend/lib/api/stage-team.ts` 调 Tauri，不裸 `invoke`；
- UI把六轴标成 `Legacy projection`，在 `intel_goal_v1` 中不显示为完成百分比；
- `observe_shadow`显示 `Non-blocking observation / legacy Gate authoritative`，`advisory_rework`显示 `Advisory review / legacy Gate authoritative`，authoritative显示 `Reviewed Goal authority`；
- raw bundle只通过已有 run_tree/审计工具按权限查看，不塞进普通 UI。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-app -E 'test(target_intel_goal_review_read_model_) | test(stage_team_read_model)' --status-level fail
cargo clippy -p golish-agent-app --lib --tests -- -D warnings
cargo fmt -p golish-agent-app -- --check
cd ..
just gen-types
pnpm exec vitest run frontend/components/Engagement/StageTeamRunView.test.tsx
pnpm exec biome check frontend/lib/api/stage-team.ts frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx
pnpm typecheck
```

### Step 5：提交

```bash
git add backend/crates/golish-agent-app/src/ai/commands/stage_team.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs frontend/lib/generated/StageTeamUnitView.ts frontend/lib/generated/StageTeamWorkItemView.ts frontend/lib/generated/TargetIntelGoalReviewReadView.ts frontend/lib/generated/TargetIntelGoalReviewFindingView.ts frontend/lib/generated/TargetIntelGoalReviewResidualView.ts frontend/lib/generated/TargetIntelGoalHoldView.ts frontend/lib/api/stage-team.ts frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx
git commit -m "feat(intel): show goal review authority"
```

提交前必须用 `git diff -- frontend/lib/generated` 确认只有生成器输出，没有手写或无关类型漂移。

---

## Task 8：接通三种 Operation-frozen Mode，但保持 Production Profile 原值

**Files:**

- Verify unchanged: `resources/harness/profiles/red_team.json`
- Verify unchanged: `resources/harness/profiles/pentest.json`
- Modify: `resources/harness/stages/target_intel/spec.json`
- Modify: `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/mode.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/start_operation_tool.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`
- Modify: `backend/crates/golish-agent-app/src/ai/stage_fork.rs`
- Modify: `backend/crates/golish-app-core/src/background_jobs.rs`
- Modify: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Add: `backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs`
- Add: `backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs`
- Modify: `scripts/run_tree.py`
- Modify: `scripts/tests/test_run_tree_intel_goal.py`

### Step 1：写 frozen-mode RED tests

```rust
#[test]
fn fixture_launch_freezes_intel_goal_v1_authority() { /* exact snapshot */ }

#[test]
fn existing_shadow_operation_cannot_be_reinterpreted_after_profile_change() { /* immutable */ }

#[test]
fn historical_operation_without_intel_contract_remains_legacy() { /* no backfill */ }

#[test]
fn observe_shadow_review_is_detached_and_never_blocks_legacy_gate_or_stage_advance() { /* outbox */ }

#[test]
fn advisory_rework_uses_goal_loop_but_keeps_legacy_completion_authority() { /* fixture */ }

#[test]
fn pentest_hard_skip_never_freezes_or_dispatches_goal_authority() { /* zero dispatch */ }

#[test]
fn non_target_intel_stages_ignore_intel_completion_authority() { /* unchanged */ }
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover --status-level fail
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_rollout --status-level fail
```

### Step 3：实现 code path，但保持 production profile 原值

- operation创建时原子保存完整 canonical payload的 `runtime_mode=observe_shadow|advisory_rework|intel_goal_v1`、completion authority、review fuel和 contract hashes；
- runtime只读 frozen operation合同；缺 row走 legacy，unknown/corrupt合同fail closed，绝不按当前 profile重解释历史；
- 测试通过显式 fixture launch policy分别启用三种 mode；
- `observe_shadow`在 legacy closeout snapshot后只写 detached review outbox，由 background job用通用只读 SubAgent执行；legacy Gate/final seal/stage advance不等待它，verdict不能 REWORK/hold/pass；
- `advisory_rework`运行 Goal/reviewer并可产生bounded补查，但 completion authority仍是 legacy Gate；
- repository production Red Team profile保持执行本 Task前的值，直到 Task 9取得第一次 profile mutation批准；
- legacy operation、Pentest hard-skip和其它 stages保持现行路径；
- run tree显示 frozen authority和 exact review/final seal link。

### Step 4：运行 GREEN

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover --status-level fail
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_rollout --status-level fail
just space-guard
cargo nextest run -p golish-agent-runtime -E 'test(target_intel_goal_) | test(pentest_target_intel_)' --status-level fail
cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --lib --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -- --check
cd ..
python3 -m unittest scripts.tests.test_run_tree_intel_goal -v
python3 -m py_compile scripts/run_tree.py
```

### Step 5：提交

```bash
git add resources/harness/stages/target_intel/spec.json backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-agent-app/src/ai/commands/mode.rs backend/crates/golish-agent-app/src/ai/start_operation_tool.rs backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs backend/crates/golish-agent-app/src/ai/stage_fork.rs backend/crates/golish-app-core/src/background_jobs.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs scripts/run_tree.py scripts/tests/test_run_tree_intel_goal.py
git commit -m "feat(intel): freeze reviewed completion authority"
```

此 Task不暂存 `red_team.json`；它必须逐字保持执行本 Task前的production值，不能预先假设该值已经是Shadow。

---

## Task 9：第一次批准后仅启用 Non-blocking `observe_shadow`，生成 Promotion 报告

**Files:**

- Add: `docs/validation/2026-08-02-target-intel-goal-shadow-promotion.md`
- Modify after explicit Approval 1 only: `resources/harness/profiles/red_team.json`
- Modify: `backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs`
- Modify: `backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs`
- Modify: `agent-progress.md`
- Modify: `feature_list.json`

### Step 0：等待第一次 production profile mutation批准

向用户展示 Task 1–8 的 migration/trigger/operation-freeze/fixture证据，并明确询问是否批准：**只让批准后新建的 Red Team operation进入 `observe_shadow`**。未得到明确批准时停止；production `red_team.json`不改，feature保持 `in_progress`。

批准后先写 RED test：

```rust
#[test]
fn repository_red_team_profile_enables_nonblocking_observe_shadow_for_new_operations() {
    let profile = load_profile("red_team").unwrap();
    assert_eq!(profile.intel_policy.unwrap().goal_loop.as_deref(), Some("observe_shadow"));
}
```

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover -E 'test(repository_red_team_profile_enables_nonblocking_observe_shadow_for_new_operations)' --status-level fail
```

预期：production profile尚未切换，断言失败。

然后只把 production profile改到 `observe_shadow + legacy_six_axis_v1`。不得在本 Task切 `advisory_rework`或`intel_goal_v1`；旧 operation因无 row/已有 frozen row保持原行为。

### Step 1：运行 deterministic corpus

使用 Plan A fixture、Plan B migration/transaction/finalizer/cutover tests，生成以下比较：

- detached observe-shadow reviewer verdict × legacy Gate verdict逐 run列出，不只给总通过率，并证明 review job延迟/失败不改变 legacy stage完成时间、pass token或Handoff；
- fixture/显式批准 cohort中的 `advisory_rework`逐 run列出补查和legacy Gate结果，但不得把 cohort结果写成production default；
- 每次 REWORK前后 material state/actions hash、增加的 facts/receipts和关闭的 finding；
- 重复无增量 finding转 NEEDS_HUMAN的证据；
- candidate→scope promotion拒绝案例；
- stale verdict、crash、response-loss和concurrent write案例；
- token、provider-call count fixture、wall-clock test cost；
- Pentest/其它 stage零行为变化证据；
- 所有 known residual和未做的真实 provider/browser验收。

### Step 2：运行完整定向验证

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover --status-level fail
just space-guard
cargo nextest run -p golish-db --test target_intel_goal_review_migrations --test runtime_memory_worker_transactions -E 'test(target_intel_)' --status-level fail
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_finalizer --test target_intel_goal_rollout --status-level fail
just space-guard
cargo nextest run -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish -E 'test(semantic_pivot_) | test(target_intel_goal_) | test(intel_review_) | test(pentest_target_intel_)' --status-level fail
just space-guard
cargo clippy -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish --lib --bins --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-db -p golish-agent-app -p golish-pentest-domain -p golish-intel-providers -p golish-recon-app -p golish-sub-agents -p golish-agent-runtime -p golish -- --check
cd ..
pnpm exec vitest run frontend/components/Engagement/StageTeamRunView.test.tsx
pnpm exec biome check frontend/lib/api/stage-team.ts frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx
pnpm typecheck
python3 -m unittest scripts.tests.test_run_tree_intel_goal -v
python3 -m py_compile scripts/run_tree.py
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check -- backend/crates/golish-agent-kit backend/crates/golish-db/migrations/20260802000001_target_intel_goal_review.sql backend/crates/golish-db/migrations/20260802000002_target_intel_goal_frontier_scope.sql backend/crates/golish-db/src/repo/target_intel_goal_contracts.rs backend/crates/golish-db/src/repo/target_intel_goal_reviews.rs backend/crates/golish-db/src/repo/target_intel_goal_frontier.rs backend/crates/golish-db/src/repo/expansion_queue.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/stage_teams.rs backend/crates/golish-db/tests/target_intel_goal_review_migrations.rs backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs backend/crates/golish-agent-app backend/crates/golish-app-core backend/crates/golish-sub-agents backend/crates/golish-agent-runtime backend/crates/golish/src/stage_run/runtime_v2.rs resources/harness/profiles/red_team.json resources/harness/profiles/pentest.json resources/harness/stages/target_intel frontend/lib/generated frontend/lib/api/stage-team.ts frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx scripts/run_tree.py scripts/tests/test_run_tree_intel_goal.py docs/validation/2026-08-02-target-intel-goal-shadow-promotion.md feature_list.json agent-progress.md
```

### Step 3：提交第一次 profile mutation与报告，等待第二次批准

```bash
git add resources/harness/profiles/red_team.json backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs docs/validation/2026-08-02-target-intel-goal-shadow-promotion.md agent-progress.md feature_list.json
git commit -m "feat(intel): observe reviewed goal shadow"
```

向用户展示报告并明确询问第二次、独立的批准：是否把 Red Team**新 operation**切到 `intel_goal_v1`。未批准时：

- `red_team.json`保持 `observe_shadow`；
- feature保持 `in_progress`，notes写“awaiting explicit promotion approval”；
- 不创建真实 operation、不调用 provider、不宣称 cutover完成。

---

## Task 10：经第二次批准后切换 Red Team 新 Operation 到 `intel_goal_v1`

**Files:**

- Modify: `resources/harness/profiles/red_team.json`
- Verify unchanged: `resources/harness/profiles/pentest.json`
- Modify: `backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs`
- Modify: `backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs`
- Modify: all affected module cards and `docs/modules/INDEX.md`
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

### Step 1：记录批准并写 RED profile test

```rust
#[test]
fn repository_red_team_profile_enables_authoritative_intel_goal_for_new_operations() {
    let profile = load_profile("red_team").unwrap();
    let policy = profile.intel_policy.unwrap();
    assert_eq!(policy.goal_loop.as_deref(), Some("intel_goal_v1"));
    assert_eq!(policy.completion_authority.as_deref(), Some("intel_goal_v1"));
}
```

### Step 2：运行 RED

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover -E 'test(repository_red_team_profile_enables_authoritative_intel_goal_for_new_operations)' --status-level fail
```

### Step 3：切 profile

仅把：

```json
"goal_loop": "observe_shadow",
"review_mode": "detached_ordered_read",
"completion_authority": "legacy_six_axis_v1"
```

改为：

```json
"goal_loop": "intel_goal_v1",
"review_mode": "ordered_read",
"completion_authority": "intel_goal_v1"
```

该 profile只为**之后新建**的 operation构造 immutable row；旧/observe-shadow operation因 frozen authority保持原行为。Pentest不改。若第二次批准未明确给出，禁止此 mutation。

### Step 4：运行最终定向回归

```bash
cd backend
just space-guard
cargo nextest run -p golish-agent-kit --test target_intel_goal_cutover --status-level fail
just space-guard
cargo nextest run -p golish-agent-app --test target_intel_goal_finalizer --test target_intel_goal_rollout --status-level fail
just space-guard
cargo nextest run -p golish-agent-runtime -p golish -E 'test(target_intel_goal_) | test(pentest_target_intel_)' --status-level fail
cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish --lib --bins --tests -- -D warnings
cargo fmt -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish -- --check
cd ..
jq empty resources/harness/profiles/red_team.json
jq empty resources/harness/profiles/pentest.json
jq empty feature_list.json
git diff --check -- resources/harness/profiles/red_team.json backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs docs/modules docs/modules/INDEX.md feature_list.json agent-progress.md
```

### Step 5：状态与提交

- 更新模块卡和索引；
- progress记录批准、验证和没有真实外部调用；
- feature verification逐项填 evidence；未授权的真实 provider/entity smoke明确写“未运行”，不伪称；
- 只有所有定向验证通过、profile frozen test通过、Plan A/Plan B证据齐全时改为 `passing`。

```bash
git add resources/harness/profiles/red_team.json backend/crates/golish-agent-kit/tests/target_intel_goal_cutover.rs backend/crates/golish-agent-app/tests/target_intel_goal_rollout.rs docs/modules docs/modules/INDEX.md feature_list.json agent-progress.md
git commit -m "feat(intel): promote reviewed goal authority"
```

---

## 可选实体只读验收（需要单独授权，不影响 fixture 代码完成）

只有用户提供 exact workspace、organization、provider/public-source allowlist和费用上限后，才执行一个新 Red Team operation。验收必须证明：

- operation冻结 `intel_goal_v1`；
- Goal owner和动态 SubAgent真实运行；
- semantic pivot/provider/public-source receipt和evidence落账；
- candidate未越权进入 active scope；
- reviewer按四段读取并产生 verdict；
- REWORK存在时回同 chain；
- finalizer PASS绑定 exact review/Handoff；
- `scripts/run_tree.py --workspace 用户批准的workspace --db --full 用户批准的新session` 可回放；
- 不修改或重解释历史 operation。

没有这项授权时不得自行选目标或调用外部服务；feature evidence应明确为 fixture/isolated-DB acceptance。

---

## 完成标准

Plan B 可以标记 `passing`，当且仅当：

1. 用户明确批准两个 migration、generated IPC和最终 promotion；
2. Review bundle、四段 cursor、verdict、findings和fences是 DB-backed immutable authority；
3. Reviewer是动态通用 SubAgent的 host-owned只读 profile，不是固定业务角色；
4. REWORK回同一 Goal chain，NEEDS_HUMAN可恢复且不产生 pass token；
5. `intel_goal_v1` finalizer不以六轴矩阵决定PASS，但严格验证review/evidence/receipt/scope/freshness/quiescence；
6. final submission/Handoff/final seal绑定 exact review id/hash/verdict；
7. stale verdict、crash、response loss、concurrent write和cross-org嫁接全部 fail closed；
8. dynamic child/review审计在 UI/run tree可见且不泄露raw bundle/secret/CoT；
9. Red Team只对批准后新 operation启用；Shadow/legacy历史/Pentest/其它 stages不变；
10. 所有定向验证、模块卡、feature和progress evidence完整。
