> Superseded by `2026-08-11-investigation-detail-read-identity.md` for the production read route.

# Investigation `stage_run` 详情真实身份实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让运行中和恢复后的 unified Investigation `stage_run` 使用真实外层 tool request identity，并可靠进入 exact Investigation Workspace。
**架构：** runtime 将当前 `tool_id` 注入 unified Investigation stage identity，在 frozen authority 验证后发布 request-scoped progress pointer；terminal result保留同一 selector。前端继续使用既有 exact resolver，拒绝任何 identity drift。
**技术栈：** Rust 2021、Tokio event channel、React 19、TypeScript、Zustand、Vitest。

## 文件结构

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：真实 owning request、首帧 progress、terminal selectors 与 unit regressions。
- 修改 `frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx`：运行中 exact route regression。
- 修改 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`：同步详情身份契约。
- 修改 `feature_list.json`、`agent-progress.md`：状态与可重放证据。

## 任务 1：锁定 RED

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 新增 unit test，调用 stage identity helper 时传 `call_exact_outer`，断言：

```rust
assert_eq!(identity.owning_stage_run_request_id, "call_exact_outer");
```

2. 新增 pure progress-event regression，断言 Investigation 事件包含 exact operation、execution、unit 与 `call_exact_outer::team::<org>`。
3. 运行：

```bash
cd backend && just space-guard && cargo nextest run -p golish-agent-runtime -E 'test(unified_investigation_stage_identity_uses_outer_tool_request) | test(unified_investigation_progress_uses_outer_tool_request)' --status-level fail
```

预期：实现前编译或断言失败，记录 RED。

## 任务 2：最小后端实现

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

1. 将 helper 改为显式接收 owning request：

```rust
fn unified_investigation_stage_identity(
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    owning_stage_run_request_id: &str,
) -> UnifiedInvestigationStageIdentity
```

2. 给 `execute_unified_investigation_stage_run` 增加 `tool_id: &str`；所有 nested/rearm identity 复用已经验证的 `stage_identity.clone()`。
3. 在 closure read 后：replay 对所有 Team 发 `passed`，fresh run 发 `running`。parent id 固定为：

```rust
format!("{tool_id}::team:{}", team.unit.organization_id)
```

4. terminal success JSON 加入：

```rust
"operation_id": operation_id,
"stage_execution_id": stage_execution_id,
"stage_run_request_id": tool_id,
```

5. 重新运行任务 1 命令，预期 2/2 通过。

## 任务 3：前端 route regression

**文件：** `frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx`

1. 保留现有 exact/live-only/conflict 用例；新增真实 `call_...` selected request + running progress row 测试，断言 production adapter mount。
2. 运行：

```bash
pnpm exec vitest run frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx
```

预期：全部通过，且 conflict 用例仍保持 unavailable。

## 任务 4：定向质量门禁与文档收尾

**文件：** 上述实现、测试、模块卡、feature/progress。

1. 运行 scoped Rust 检查：

```bash
cd backend && just space-guard && cargo clippy -p golish-agent-runtime --lib --no-deps -- -D warnings
rustfmt --edition 2021 --check backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs
```

2. 运行 scoped frontend 检查：

```bash
pnpm exec biome check frontend/components/ToolCallDetailView/ToolCallDetailView.investigation.test.tsx
pnpm typecheck
```

3. 运行 JSON/diff/active-feature 检查并记录命令、退出码和关键输出。按 AGENTS §0.1 不运行未获授权的 init/precommit/全仓 suite。
4. 不自动 commit；将所有修改与当前实体旧 synthetic run 的兼容边界写入 `agent-progress.md`。
