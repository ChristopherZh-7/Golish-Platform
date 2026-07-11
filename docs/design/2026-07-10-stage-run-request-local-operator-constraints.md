# Stage-run request-local operator constraints

> 状态：Accepted（2026-07-10，Enumeration live closeout 的 resume steering 合同补强）。
>
> 本设计只改 Task resume 的请求上下文传递；不改 DB schema、durable
> worker chain、stage gate、授权或 request-scoped retry budget。

## 1. 问题与结论

新 operation 会把顶层 GUI Task / headless CLI `-e` 写入 `tasks.input`。
旧 resume 路径虽然接收 `TaskOrchestrator::resume(..., user_message, ...)`，但
`run_executor_driven` 每次只从 `tasks.input` 重读初始文本。因此：

```text
初始请求 A -> durable tasks.input A
新用户续跑 B -> resume(B) -> ExecutionContext.task_input 仍是 A
```

这会丢掉“继续，但这 5 个 exact origin 不要调 producer”之类当前
operator constraint，而 `BridgeAgentExecutor -> SubAgentContext.original_request ->
stage_run worker objective` 后续 seam 虽已接通，拿到的却是 stale A。

修复合同是将“durable operation objective”与“本次 top-level request input”分开：

| 入口 | durable `tasks.input` | 本次 `ExecutionContext.task_input` |
|---|---|---|
| 新 GUI Task | A | A |
| headless CLI `-e` | A | A |
| resume，非空 B | A（不改写） | B |
| resume，空/全空白 B | A（不改写） | A（明确 fallback） |

## 2. 传递链

```text
TaskOrchestrator::run / run_stage
  request_input_override=None
  -> durable tasks.input

TaskOrchestrator::resume(user_message=B)
  request_input_override=Some(B)
  -> nonblank B, otherwise durable tasks.input

ExecutionContext.task_input
  -> BridgeAgentExecutor::primary_loop_context
  -> SubAgentContext.original_request
  -> stage_run::build_org_objective
  -> bounded lower-priority operator-constraint block
```

`request_input_override` 是 Rust 调用栈上的 request-local 值，不写入
`tasks`、`operation_state.state_blob`、session preference 或 worker-chain key。一次
resume 中的 stage retries 共用同一个 `ExecutionContext`，因而始终看到同一个 B。

## 3. Worker chain 与 reentry guard

- `stage_run_workers[stage][org_id]` 的 chain id 不变；resume 仍加载原 worker
  history，再把新 worker objective（包含 B）追加为最新 user turn。
- `BridgeAgentExecutor::new` 仍是新 top-level Task 请求的 retry-budget 边界；
  新 continuation 重置 `StageRunReentryGuard`，同请求的 reflector/gate retries 共用
  该 guard。
- operator input 不得重置 guard、更换 chain id 或清理 checkpoint。
- guard 目前保存在 session `AgentBridge` 上，依赖同一 bridge 的 Task
  request 串行化。同 session 并发创建两个 `BridgeAgentExecutor` 会互相
  reset/close guard，并且 isolated history/side-channel 也本就不支持该并发。
  本改动不扩大该并发模型；若后续开放同 session 并发，必须先加
  per-bridge single-flight 或把 guard 完全下沉到 executor-owned request context。

## 4. 安全优先级

resume B 仍是未信任的 operator text，不是授权源：

- stage 来自 active harness stage；org 来自 bound engagement subtree；target /
  exact-origin denominator 来自 DB。
- tool allowlist、authorization ceiling、read-only/exact-origin guard 和 evidence/gate
  合同优先于 B。
- B 在 worker objective 里使用 JSON string 引用、Unicode-safe 有界截断，且
  只允许收紧方法；扩 scope、换 stage/org、放宽授权、伪造 terminal
  均必须被忽略并由 runtime/gate 阻断。
- 空 B 回退 A，避免把 worker 的顶层目标意外清空。

## 5. 失败与恢复

本改动不改 `execute_isolated_with_context` 的 history save/restore。普通成功、
LLM/tool error 和用户 Stop 均按现有路径返回后恢复 history。新 input 没有
durable 写入，所以该次 resume 失败后，下一个新请求可以用新 C，空 C
则再次回退 A。

## 6. 测试与验收

1. resolver 测试锁定 `A + resume B -> B`，且 B 不混入 A。
2. resolver 测试锁定 fresh `None -> A` 与 blank resume `" \n\t" -> A`。
3. bridge 测试锁定 `ExecutionContext.task_input -> SubAgentContext.original_request`。
4. runtime worker-objective 测试锁定 B 被引用、A 不出现，UTF-8/超长/扩权
   文本仍受有界与 non-overridable contract 保护。
5. targeted `nextest` / `cargo check` / `clippy -D warnings` / `fmt --check` /
   `git diff --check` 全绿。不运行 live DB 或改 feature/progress。
