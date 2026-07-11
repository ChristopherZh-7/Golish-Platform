# Per-bridge universal top-level request single-flight 实现计划

> **目标：** 同一 `AgentBridge` 的所有 top-level agent/history 写请求 fail-fast
> single-flight；Task/profile lead→orchestrator 复用 owner，Stop 不可被并发 Chat 复活，
> request-local harness sidechannel 不跨 request 泄漏。

**架构：** `TopLevelRequestGate` 用 AtomicBool Acquire/Release；cloneable
`TopLevelRequestLease` 只在 last-drop release，并用 token-local CAS 让 Task retry guard 每
request 只 reset 一次。成功 acquisition 后立即 reset stale cancel，再 async scrub（让
scrub 期间的新 Stop 保持 true）；正常结束持
lease cleanup，future drop/panic 由 next-acquire scrub 兜底。

## Task 1：先锁定 owner/token 合同

**文件：** `backend/crates/golish-agent-bridge/src/agent_bridge/task_request.rs`

1. 写 gate tests：并发 B busy；clone transfer 不 early-release；nested Task 初始化不二次
   reset；error/cancel/unwind last-drop release。
2. 实现 `TopLevelRequestGate` / `TopLevelRequestLease` 与 token-local
   `task_initialized`。
3. 验证：

```bash
cd backend && cargo nextest run -p golish-agent-bridge task_request --status-level fail
```

## Task 2：把 cancel 与 harness state 纳入 owner

**文件：**

- `backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs`
- `backend/crates/golish-agent-bridge/src/agent_bridge/execution.rs`
- `backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs`

1. `begin_top_level_request`：先 acquire，立即 `reset_cancelled()`，随后清旧 request
   state；不得在 async scrub 后 reset，以免吞掉并发 Stop。
2. `clear_top_level_request_state` 只接受属于该 bridge 的 lease，清 harness/pending-plan
   sidechannels。
3. 删除 raw depth-0 execution 中的 cancel reset，避免 nested fallback 复活 Stop。
4. 用真实 bridge 测 busy 不改状态、next-acquire scrub/reset、history clear busy。

## Task 3：接通所有生产入口且不递归 acquire

**文件：**

- `backend/crates/golish-agent-bridge/src/bridge_executor/mod.rs`
- `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`
- `backend/crates/golish-agent-app/src/ai/commands/session.rs`
- `backend/crates/golish/src/cli/runner.rs`
- `backend/crates/golish/src/stage_run/mod.rs`

1. GUI text 在 mode/lead 分流前 acquire；token 贯穿 direct Task/resume/flexible lead。
2. `BridgeAgentExecutor::from_request` 接收 token；首个 Task upgrade reset guard，nested
   handoff 不 reacquire。
3. attachments 与 history clear/restore 先 acquire；busy 不触达 payload execution/history。
4. ordinary CLI 在启动 terminal-event receiver 前 acquire；stage-run acquire 后 from_request。
5. orchestrator 返回、fallback Chat 前清 harness state；所有 top-level return 前 cleanup。

## Task 4：文档与 scoped 验证

更新 bridge/agent-app/stage-run/CLI 模块卡和 `docs/modules/INDEX.md`，然后执行：

```bash
cd backend && cargo nextest run -p golish-agent-bridge --status-level fail
cd backend && cargo check -p golish-agent-bridge -p golish-agent-app -p golish
cd backend && cargo clippy -p golish-agent-bridge -p golish-agent-app -p golish --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

不运行 live DB，不改 `feature_list.json` / `agent-progress.md`，不 stage/commit/push。
