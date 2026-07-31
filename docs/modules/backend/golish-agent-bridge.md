# golish-agent-bridge

> **一句话职责**：Tauri 应用与 agent runtime 之间的**桥接层**——`AgentBridge` 生命周期/派发 + request single-flight + runtime-memory/canonical UoW/scoped ContextPack provider 装配。

- **类型**：crate（Layer 4c · agent 桥接）
- **路径**：`backend/crates/golish-agent-bridge/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `AgentBridge` 的构造/生命周期/派发、`BridgeBackends` 装配时
- 改 bridge 侧的 HITL / tool policy / session / context 装配（`bridge_*`）时
- 改 `bridge_executor`（依赖 `AgentBridge` 的 orchestrator 实现）或 prompt `contributors` 时

## 职责

把 app 层与底层 agent runtime 解耦：owns `AgentBridge` 结构体及其 `bridge_*` 伙伴、`bridge_executor` 编排实现（从 `golish-ai` 在 A1-3 抽出）。为兼容历史 `crate::xxx` 路径，在 crate 根 re-export 了 `golish-agent-kit` / `golish-agent-runtime` 的大量模块（`agentic_loop`、`tool_*`、`planner` 等）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `AgentBridge` | 桥接主体（生命周期 + 派发 + generation-bound request owner） |
| `SessionRequestSlot` / `SessionRequestTransitionLease` | 跨 bridge generation 的 stable logical-session request authority / init reservation |
| `BridgeBackends` | bridge 后端装配束 |
| `set_tracker_session_uuid` | Task/stage 执行前把 durable `sessions.id` 绑定到共享 `DbTracker` identity；已存在的 tracker clones 同步可见 |
| `set_runtime_memory_repository` | 安装 V2 compound runtime-memory backend，并透传到 agentic loop |
| `set_hypothesis_analysis_runtime` | 安装Plan B closed `HypothesisAnalysisStageRuntime` Arc并逐turn快照透传；bridge不构造repo/provider、不选择rollout authority |
| `set_knowledge_memory` / `knowledge_memory` | 注入/读取 process-shared canonical Memory Fabric UoW；只传 handle，不拥有 supervisor worker |
| `set_knowledge_context` / `knowledge_context` | 注入/读取 scoped ContextPack provider；per-turn 只 clone Arc，不缓存跨 operation pack |
| `bridge_executor` | 依赖 `AgentBridge` 的编排器实现 |
| `contributors` | prompt 贡献者 |
| re-export：`agent_kit::{tool_execution, tool_executors, planner, hitl, tool_policy, …}` | 兼容旧 `crate::` 路径 |
| re-export：`agent_runtime::agentic_loop` / `prompts::{prompt_registry, system_prompt}` | 兼容旧路径 |

## 依赖

- **内部**：`golish-agent-kit`、`golish-agent-runtime`、`golish-memory-app`、`golish-prompts`、`golish-events`、`golish-context`、`golish-sub-agents`、`golish-llm-providers`、`golish-tools`、`golish-session`、`golish-indexer`、`golish-core`、`golish-settings`
- **外部**：`rig-core`、`rig-anthropic-vertex`、`rig-openai-responses`、`tokenx-rs`

## 被谁依赖 / 改动影响面

`golish-agent-app`（持有 per-session `AgentBridge`）、`golish`。是 app 层接入 agent 的唯一入口。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `agent_bridge/` | `AgentBridge` 主体 + 5 子系统 + 构造器 | [→](golish-agent-bridge/agent_bridge.md) |
| `bridge_executor/` | `AgentExecutor` 实现（orchestrator↔bridge） | [→](golish-agent-bridge/bridge_executor.md) |

## 关键文件

`bridge_context.rs`、`bridge_hitl.rs`、`bridge_policy.rs`、`bridge_session.rs`、`contributors.rs`（均为单文件模块）。

## 注意事项 / 坑

- crate 根的大量 `pub use` 是**有意的兼容垫片**（迁移自 `golish-ai`），不要误删——删了会断开历史 `crate::agentic_loop` 等路径。
- 桥接层不应直接持有 Tauri 类型业务逻辑；Tauri command 在 `golish-agent-app`。
- 同一 logical session 的 GUI text/attachments、Chat/Task/profile lead、CLI/stage-run 和 history clear/restore 共用 stable slot 的 universal fail-fast owner；GUI bridge replacement 只能推进 generation，不能创建独立 gate。Task handoff 必须用同一 token 构造 `BridgeAgentExecutor`，不能绕过 ownership 或递归 acquire，否则 cancel/history/harness side-channel 隔离失效。
- GUI TaskMode 解析出 chat key 对应的 durable `sessions.id` 后，必须在任何 stage executor/tool dispatch 前调用 `set_tracker_session_uuid`。该绑定更新的是所有 `DbTracker` clones 共享的身份，不是只改 bridge 内的一份值拷贝；否则工具生命周期会落到随机 tracker session，而 gate 在 durable session 下查不到记录并持续 BLOCK。
- `BridgeAgentExecutor` 必须把共享 `StageRunReentryGuard` 的 exhausted 状态通过 `AgentExecutor::stage_run_retry_budget_exhausted` 暴露给 TaskOrchestrator；同一 top-level request 的 reflector 不能只看到工具级 `reentry_blocked` 后继续自动重启。只有新的 `TopLevelRequestLease` 首次初始化 Task 才 reset guard，所以显式用户 continuation 仍获得 fresh bounded budget，nested/automatic pass 不得重置。
- `BridgeServices.knowledge_memory` 只是 transaction port。任何 constructor/session replacement/configure 调用都不能在 bridge crate 启 projector；process owner 只在 desktop/CLI DB-ready composition root。
- `BridgeServices.knowledge_context` 只是依赖注入 capability；不能接收 caller-forged trusted authorization，也不能在 provider 缺失时回退 legacy global knowledge。
- Plan B production composition由`golish-agent-app`完成；bridge只保存/传播同一个closed runtime Arc。Arc缺失时Registry dispatch fail closed，存在时也不能按profile/default改写operation-frozen canonical writer；Stage Team control-plane、Gate/canonical truth与Plan C/D authority保持分离。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-bridge
```
