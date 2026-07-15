# Active Recon 目标范围一次确认实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。用户已明确禁止 `init.sh`；本计划只跑聚焦验证，也不创建 commit。

**目标：** Target Intel 完成后让用户确认本次发现的精确目标子集一次，确认成功即自动进入 EAS，不再出现第二个通用 approval。

**架构：** 在 `DbRepoProvider` 增加 operation-bound target scope review 事务接口；orchestrator 在 TargetIntel→EAS 边界发 `scope_review` 并严格校验原样子集；前端给这个等待态单独命名。直接 EAS 入口和历史目标复用继续 fail closed。

**技术栈：** Rust 2021、async-trait、sqlx/Postgres、Tokio、React 19、TypeScript、Vitest。

## 文件结构

- `backend/crates/golish-agent-kit/src/db_traits/repo.rs`：review DTO 及 fail-closed repository port。
- `backend/crates/golish-agent-kit/src/task_orchestrator/active_recon_scope.rs`：canonicalization、AskHuman review、authorization readiness。
- `backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`：注册专用子模块。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：在 TargetIntel→EAS 边界调用专用流程并跳过重复 phase approval。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`：orchestrator RED/GREEN 回归。
- `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`：候选读取、事务确认、恢复快照实现。
- `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`：repository trait delegation。
- `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`：映射 `waiting_target_scope`。
- `frontend/components/AIChatPanel/StageMarker.tsx`：目标范围等待态图标/颜色。
- `frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx`：用户可见文案回归。
- `docs/modules/backend/golish-agent-kit/task_orchestrator.md`、`docs/modules/backend/golish-agent-app/ai.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`：同步 system-of-record。

## 任务 1：先写专用边界的失败测试

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs`

**步骤：**

1. 扩展 `MemRepo`，分别记录 current-operation Target Intel 候选、已确认 snapshot 和 apply 调用。
2. 添加测试：TargetIntel→EAS 没有 trusted target 时发 `AskHumanRequest { input_type: "scope_review" }` 和 `waiting_target_scope`。
3. 添加测试：返回原样非空子集时只 apply 一次、boundary `Allowed`、不发 generic confirmation approval。
4. 添加测试：新增/改写/空列表/Skip 时 `Held` 且不 apply。
5. 添加测试：direct EAS entry 不触发 scope review。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(active_recon_scope_review)' --status-level fail
```

预期：实现前新增测试失败，错误指向缺少 repository port/status/专用边界。

## 任务 2：实现 repository port 与纯校验

**文件：** `backend/crates/golish-agent-kit/src/db_traits/repo.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/active_recon_scope.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/mod.rs`

**步骤：**

1. 增加 `ActiveReconScopeReviewApproval`，字段固定为 `request_id`、`presented`、`selected`。
2. trait 默认候选读取返回空；apply 与 authorization read 返回 `ACTIVE_RECON_SCOPE_REPO_UNAVAILABLE`，避免测试替身把不可用误报为空授权。
3. 实现 canonical set 函数，identity 为 `(target_type, canonical_value, scope)`，拒绝重复、非法类型、非法 scope。
4. 实现 AskHuman JSON array context、decision.reason 解析、非空原样子集校验。
5. 成功 apply 后把当前 invocation authority 设为 true；失败发 `waiting_target_scope` 并保持 stage。

**验证：** 重跑任务 1 命令，预期新增测试全部通过。

## 任务 3：实现 Postgres 原子固化与恢复验证

**文件：** `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`

**步骤：**

1. 候选 SQL join `operation_state`，要求 exact operation/org、`current_stage='target_intel'`，并由 `source='asset_intel' AND updated_at >= stage_started_at` 的 refreshed row 打开 review；展示集合覆盖该 org 当前 trusted + asset-intel `scope='in'` denominator。
2. apply 开事务并 `FOR UPDATE` 锁 operation；重读候选并与 presented canonical set 完全一致。
3. 校验 selected 为 presented 的非空原样子集；按 exact `(organization_id, target_type, value, source)` 更新 selected 为 `customer_provided/in`，未选择为 `out`。
4. 同事务写 `state_blob.active_recon_target_scope` schema v1 和 `audit_log` action `active_recon_target_scope_approved`，然后 commit。
5. authorization read 解析 snapshot，并要求 operation/org 和当前 `scoping_target_snapshot` 与 selected exact-match。
6. 加 SQL contract tests，断言 operation/org/stage/window/source predicates 和 transaction update predicates 都存在。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app -E 'test(active_recon_scope)' --status-level fail
```

预期：target scope bridge tests 全绿。

## 任务 4：接入 TargetIntel→EAS 并删除重复审批

**文件：** `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**步骤：**

1. successor 是 EAS 时调用 `ensure_active_recon_target_scope(task_id)`。
2. 返回 ready 后立刻 `PhaseGateDecision::Allowed`，不落入 `request_phase_approval`。
3. 返回 hold 时保留 Target Intel interruption；其它 phase boundary 不变。
4. direct EAS preflight 继续只调用 trusted-target readiness，不打开 provider review。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(active_recon_scope_review) | test(pre_eas) | test(two_level_phase_gate)' --status-level fail
```

预期：专用范围确认测试和既有 phase gate 回归全绿。

## 任务 5：修正前端等待语义

**文件：** `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts`、`frontend/components/AIChatPanel/StageMarker.tsx`、`frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx`

**步骤：**

1. 把 `waiting_target_scope` 作为持久 stage marker 状态。
2. label 映射为 `Review scan targets`，detail 使用后端的精确 blocker 文案。
3. marker 使用与 waiting approval 相同的暂停图标/amber 外观，但不显示 approval 文案。
4. 测试发 task_progress event 并断言新 label/status；同时保留旧 waiting approval 断言。

**验证：**

```bash
pnpm exec vitest run frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx frontend/components/AIChatPanel/AskHumanInline.test.tsx frontend/components/AIChatPanel/ScopeReviewTable.test.tsx
pnpm exec biome check frontend/components/AIChatPanel/hooks/useAiChatEvents.ts frontend/components/AIChatPanel/StageMarker.tsx frontend/components/AIChatPanel/hooks/useAiChatEvents.test.tsx
```

预期：聚焦 Vitest 和 Biome 全绿。

## 任务 6：同步模块卡并做限定收尾

**文件：** 三张模块卡、`docs/modules/INDEX.md`、`feature_list.json`、`agent-progress.md`

**步骤：**

1. 记录一次确认、operation-bound 恢复与 fail-closed 限制。
2. 把 focused command、exit code、测试数和未跑 broad gate 的原因写入 progress/evidence。
3. feature 保持唯一 `in_progress`；没有 `just precommit` 证据时不改为 passing。

**验证：**

```bash
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
git diff --check
```

预期：exit 0；不运行 `init.sh`、`just precommit` 或全量测试。
