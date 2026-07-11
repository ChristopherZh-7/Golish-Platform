# Stage-run request-local operator constraints 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 GUI Task resume 的本次非空 steering 文本传入 stage-run worker，同时保留 durable 初始 task input、worker chain 和 request-scoped reentry guard。

**架构：** `TaskOrchestrator::resume` 把 `user_message` 作为 `run_executor_driven` 的 request-local override；纯 helper 选择“非空 override，否则 durable original”后构造 `ExecutionContext.task_input`。Bridge 和 runtime 继续使用现有 `task_input -> original_request -> bounded worker objective` seam，不写 DB 或 checkpoint。

**技术栈：** Rust 2021、async trait orchestration、cargo nextest、rustfmt、Clippy、Markdown 模块卡。

## 文件结构

- `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`：fresh/resume 调用边界，仅 resume 传入本次 `user_message`。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：request-local input resolver 与 `ExecutionContext` 组装。
- `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs`：声明 `task_input` 的 fresh/resume 语义。
- `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs`：现有 `task_input -> original_request` 回归锚点。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：现有 bounded worker-objective 回归锚点。
- `docs/modules/backend/golish-agent-kit/task_orchestrator.md`：编排层 request-local resume 合同。
- `docs/modules/backend/golish-agent-bridge/bridge_executor.md`：bridge 传递与 durable/request-local 分层合同。

## Task 1：用纯 resolver 锁定 A/B/fallback 语义（TDD）

**文件：**
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`

**步骤 1：写失败测试。**

```rust
#[test]
fn request_local_resume_input_overrides_durable_original_without_merging() {
    let durable = "A: original operation".to_string();
    let resumed = "B: do not call producers for five roots";
    let resolved = resolve_request_local_task_input(durable, Some(resumed));
    assert_eq!(resolved, resumed);
    assert!(!resolved.contains("A:"));
}

#[test]
fn request_local_resume_input_fresh_and_blank_fall_back_to_durable_original() {
    let durable = "A: original operation".to_string();
    assert_eq!(resolve_request_local_task_input(durable.clone(), None), durable);
    assert_eq!(
        resolve_request_local_task_input(durable.clone(), Some("  \n\t")),
        durable
    );
}
```

**步骤 2：验证 RED。**

```bash
cd backend && cargo nextest run -p golish-agent-kit request_local_resume_input --status-level fail
```

预期：因 resolver 尚不存在而编译失败。

**步骤 3：实现最小 resolver。**

```rust
fn resolve_request_local_task_input(
    durable_task_input: String,
    request_input_override: Option<&str>,
) -> String {
    request_input_override
        .filter(|input| !input.trim().is_empty())
        .map(str::to_string)
        .unwrap_or(durable_task_input)
}
```

**步骤 4：验证 GREEN。** 重跑步骤 2，预期 2 passed、0 failed。

**提交：** 本工作树由主任务统一收口；本计划执行不单独 stage/commit。

## Task 2：接通 fresh/resume 请求边界

**文件：**
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs`

**步骤 1：扩展 executor-driven 入口。**

```rust
pub(crate) async fn run_executor_driven(
    &mut self,
    task_id: Uuid,
    queue: &[PlannedSubtask],
    executor: &dyn AgentExecutor,
    resume: bool,
    request_input_override: Option<&str>,
) -> anyhow::Result<String>
```

从 repo 读出 `durable_task_input`，仅用 Task 1 resolver 决定本次
`ExecutionContext.task_input`，不调用 task update SQL。

**步骤 2：锁定两个调用点。**

```rust
// New GUI Task / headless CLI path
self.run_executor_driven(task.id, &queue, executor, false, None).await;

// Existing-operation GUI resume path
self.run_executor_driven(
    task_id,
    &queue,
    executor,
    true,
    Some(user_message),
).await;
```

**步骤 3：更新 DTO 文档。** `ExecutionContext.task_input` 明确为本次
request-local 值：fresh=A，resume nonblank=B，resume blank=A，durable row 不变。

**步骤 4：验证传递 seam。**

```bash
cd backend && cargo nextest run -p golish-agent-kit request_local_resume_input --status-level fail
cd backend && cargo nextest run -p golish-agent-bridge primary_loop_context_carries_cli_and_gui_top_level_requests --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime enumeration_objective_receives_bounded_operator_unreachable_root_constraints --status-level fail
```

预期：全部 passed、A/B resolver 不混入，bridge 保留 depth=0，worker objective
含 B/operator block。

**提交：** 不单独 stage/commit；与主任务的完整验证证据一起收口。

## Task 3：合同文档与 scoped 验证

**文件：**
- 修改 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改 `docs/modules/backend/golish-agent-bridge/bridge_executor.md`
- 创建 `docs/design/2026-07-10-stage-run-request-local-operator-constraints.md`
- 创建 `docs/superpowers/plans/2026-07-10-stage-run-request-local-operator-constraints.md`

**步骤 1：记录不变量。** 文档必须明确：`tasks.input` 不改写、blank
fallback、worker chain/checkpoint 不变、guard 仅由新 executor 重置、operator text
不能改变 stage/org/scope/auth/gate。

**步骤 2：运行 targeted 验证。**

```bash
cd backend && cargo fmt --all -- --check
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime --status-level fail
cd backend && cargo check -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-bridge -p golish-agent-runtime --all-targets -- -D warnings
git diff --check
```

预期：命令 exit 0，0 failed，0 warnings，无 whitespace error。不运行 live DB，
不修改 `feature_list.json` / `agent-progress.md`。

**步骤 3：交付。** 报告修改文件、验证命令和剩余并发风险；不 stage/
commit/push。

**提交：** 无；交由主任务统一收口。
