# Per-bridge universal top-level request single-flight

> Extended by
> [`2026-07-10-session-generation-and-history-recovery.md`](2026-07-10-session-generation-and-history-recovery.md):
> GUI ownership now lives in a stable logical-session slot across bridge generations, and
> isolated Task history is recoverable after abort/panic.
>
> 状态：Accepted（2026-07-10）。最初只保护 heavyweight Task；独立审查发现
> Chat、Task/profile lead、附件和历史 clear/restore 仍可与 Task 并发，因此同日扩展为
> 同一 `AgentBridge` 的 universal top-level request owner。
>
> 本设计不改 DB schema、durable worker chain、stage gate 或授权合同。

## 1. 问题与结论

同一个 bridge 的 agent loop 共享 `conversation_history`、`cancelled`、
`pending_plan_request` 和 `harness_active_*` / `harness_submit_only` /
`harness_last_deliverable`。只在 `BridgeAgentExecutor::new` 拦第二个 heavyweight Task
仍有四个缺口：

1. Stop 后前端立即允许下一次发送；新的 Chat/lead/附件 depth-0 loop 会把
   `cancelled=true` 清回 false，使旧 Task 复活。
2. Task/profile lead 在 handoff 到 orchestrator 之前已运行完整 loop 并改历史/
   `pending_plan_request`，太晚才 acquire Task gate。
3. `execute_isolated_with_context` 的 history take/restore 与并发 Chat 的 final-history
   replace 会互相覆盖。
4. Task 正常或异常结束后，bridge 上可能残留 stage/authz/org/operation/submit-only/
   deliverable；Rust `Drop` 不能 await 清理这些 `tokio::RwLock`。

结论：GUI text、附件、普通 CLI、headless stage-run，以及 conversation history
clear/restore 都先获取同一 universal fail-fast owner。Task/profile lead→orchestrator 传递
同一可共享 token，不递归 acquire。只有成功取得新 owner 才能 reset cancellation；raw
agent execution 不再自行 reset。

## 2. Ownership 与生命周期

```text
top-level entry
  -> begin_top_level_request
       compare_exchange(false -> true)
       busy: return; no cancel reset/history/sidechannel mutation
       owner: reset stale cancellation immediately -> async stale-state scrub
  -> Chat / attachment / Task-profile lead / CLI
       lead calls start_operation:
         clone same lease -> BridgeAgentExecutor::from_request
         first Task upgrade resets StageRunReentryGuard once
       reflector / retries / subtasks reuse same executor/token
  -> while lease is still held: async request-state cleanup
  -> last lease clone Drop -> Release store(false)
```

`TopLevelRequestLease` 包装 `Arc<...Inner>`；gate 只在最后一个 clone Drop 时释放。
token 内的 `task_initialized` CAS 保证同一 request 第一次升级成 Task 时刷新 retry
budget，后续 lead handoff / nested executor 不再次 reset 已耗尽的 guard。

Acquire/Release ordering 只表达同 bridge 的 owner happens-before；失败路径用 Relaxed，
因为 busy caller 不读取或修改 owner-protected 状态。

## 3. Cancel 与 cleanup

- `reset_cancelled()` 只在 `begin_top_level_request` 成功后立即调用；必须早于 async
  scrub，确保 scrub 期间到达的 Stop 保持 `true`。raw
  `execute_with_context_inner` / multi-modal execution 不再按 depth 清 cancel。
- 正常 GUI Task 在 orchestrator `run/resume.await` 后、fallback Chat 前清 harness state；
  顶层 GUI/附件/CLI/stage-run return 前再次做幂等 cleanup。
- cleanup 清 stage/authz/org/operation/submit-only/forced-tool/deliverable 和
  `pending_plan_request`，但不清 durable conversation history、profile 或 worker chain。
- future drop/panic 无法 async cleanup；last-token Drop 先释放 owner，下一次 acquisition
  在执行/reset 前做相同 scrub，保证 stale state 不泄漏给新 request。

## 4. 入口合同

- GUI `send_ai_prompt_session`：mode 分流前 acquire；Chat、direct Task、resume 和 flexible
  lead 共用同 token。
- GUI attachments：payload 执行前 acquire。
- conversation history clear/restore：写 history 前 acquire；busy 时不改历史。
- ordinary CLI `execute_once`：启动 event receiver 前 acquire，避免 busy 没有 terminal event
  时 CLI 永久等待。
- headless `stage_run::orchestrate`：acquire 后用 `BridgeAgentExecutor::from_request`。
- `BridgeAgentExecutor::new` 仅是 standalone async convenience；已拥有 request 的生产入口
  必须用 `from_request`，不得递归 acquire。

## 5. 不变量

1. busy B 不能 reset cancel/retry guard，不能触达 history 或 request sidechannels。
2. Stop 后只有旧 owner 完全退出、last lease Drop、新 request acquire 成功，才可清 cancel。
3. lead→orchestrator、fallback、reflector 和 subtasks 共用一个 owner。
4. 同一 token 的 Task 初始化最多一次；A exhausted 时 nested executor 不得重开 budget。
5. success/error/cancel/future drop/unwind 都由 last-token RAII release。
6. 正常返回在 release 前 async cleanup；异常遗留由 next-acquire scrub。
7. 不同 bridge 仍可并行。

## 6. 测试与验收

- gate 纯测试：busy、clone transfer、一次性 Task 初始化、error/cancel/unwind release。
- 真实 `AgentBridge` 状态测试：busy 不重置 owner cancel/sidechannels；history clear busy
  fail-fast；abandoned owner 后 next acquire scrub + reset；lead token 可交给 executor 且不自锁。
- 编译 GUI text/attachments/history commands、ordinary CLI 与 stage-run。
- targeted nextest、cargo check、Clippy `-D warnings`、rustfmt 与 `git diff --check` 全绿。
