# golish-agent-bridge / agent_bridge

> **一句话职责**：`AgentBridge` 主体——编排 LLM 通信 / 工具执行（HITL）/ 会话历史 / 会话持久化 / 上下文窗口 / loop detection，按关注点拆成 5 个聚焦子系统（EventBus/LlmConfig/Services/AccessControl/Session）+ constructors。

- **类型**：目录模块（属于 crate [`golish-agent-bridge`](../golish-agent-bridge.md)）
- **路径**：`backend/crates/golish-agent-bridge/src/agent_bridge/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `AgentBridge` 构造（`new`/`with_*`）、5 子系统装配、生命周期/派发时
- 改 bridge 的 LLM 配置（provider/model/web search/reasoning effort）、外部服务句柄时

## 职责

`AgentBridge` 是 app↔runtime 的核心句柄，按关注点分解：`BridgeEventBus`（事件发射/seq/frontend-ready 缓冲/coordinator/transcript）、`BridgeLlmConfig`（client + provider/model + provider 特定配置）、`BridgeServices`（DB/PTY/sidecar/indexer/settings/ContextPack provider 可选句柄）、`BridgeAccessControl`（tool policy/HITL/agent mode/loop detection）、`BridgeSession`（历史 + 持久化）。顶层字段是跨切面身份/编排状态（workspace/tool registry/sub-agents/context/MCP）。standalone bridge 持有私有 active request slot；GUI bridge 绑定 `AiState` 的 stable logical-session slot + generation，让 Chat/Task/附件/CLI/history mutation 的 cancel、history、harness side-channel 与 retry budget 在 bridge replacement 前后仍是 single-flight。trusted resume preflight 选出的 whole-record runtime-memory source 也是 request-local 状态：一个 top-level request 只固定一次，bridge prepare 原样传给 runtime，worker/executor 不得逐次重选或 field-merge。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AgentBridge` | 桥接主体 |
| `BridgeBackends` | 后端装配束 |
| `constructors`（`new` / `with_*`） | 构造 impl |
| `execute_with_turn_instructions` | 单 turn 隐藏 system instruction 注入；UI events / sidecar / history 仍记录原始用户 prompt |
| `execute_isolated_with_context` | Task-mode fresh-history 执行，同时显式携带 request-local `SubAgentContext`（用于把顶层 task input 交给 stage-run worker）；depth 必须保持 0 |
| `begin_top_level_request` / `clear_top_level_request_state` | universal fail-fast ownership；成功 acquire 后立即 reset stale cancel，再 scrub；release 前 async cleanup |
| `TopLevelRequestLease` | cloneable request token；last clone Drop 才 Release，供 Task lead→executor handoff |
| `SessionRequestSlot` / `SessionRequestTransitionLease` | 跨 bridge generation 的 stable request authority + init lifecycle reservation；lease 同时校验 slot identity/generation |
| `BridgeEventBus` / `BridgeLlmConfig` / `BridgeServices` / `BridgeAccessControl` / `BridgeSession` | 5 子系统 |
| `set_runtime_memory_repository` | 注入 V2 compound runtime-memory backend；`prepare.rs` 原样快照到每-turn `AgenticLoopContext` |
| `set_resume_runtime_memory_source` | trusted resume preflight 将 `Legacy|V2|LegacyFallback` whole-record source 固定到当前 top-level request；`prepare.rs` 原样快照，request release/scrub 必须清空 |
| `set_knowledge_memory` / `knowledge_memory` | 注入 process-shared canonical UoW，供后续 compound terminalizer 使用；不启动 projector |
| `set_knowledge_context` / `knowledge_context` | 注入 C7 scoped `ContextPackProvider`；`prepare.rs` 只把同一 Arc 快照到每-turn runtime context，不在 bridge 自行检索或缓存跨 operation 数据 |
| `harness_active_operation_id_handle` / `harness_active_org_id_handle` / `harness_active_stage_handle` | harness side-channel handles；工具注册层可读 active operation/stage/org，用于 submit 预检、org 隔离和 wave cutoff |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `AgentBridge` 结构 + 子系统组合 |
| `constructors/` | `new` + `with_*` builder |
| `task_request.rs` | stable session slot/generation、transition、cloneable RAII lease 与真实 bridge state 并发合同测试 |

## 依赖

- crate 内（agent-kit/runtime 经 crate 根 re-export）、`golish-events`（coordinator/transcript）、`golish-session`

## 注意事项 / 坑

- 5 子系统是有意分解（隔离关注点）；加字段先想归哪个子系统，别全堆顶层。
- `BridgeServices.runtime_memory` 与 generic `DbTracker`/`chain_persistence` 职责不同：前者提供 V2 原子 seed/claim/heartbeat/tool/chain mutation。production bridge 三者一起装配；eval/legacy context 可显式不提供 runtime-memory backend。
- `resume_runtime_memory_source` 不是 session preference，也不是 worker-local fallback 开关。只有 trusted task-resume preflight 可以调用 setter；同一 request 的 graph/Worker/chain read 必须全部使用该完整来源。`prepare.rs` 只能复制，nested executor/worker 不得重选；top-level owner 的 normal cleanup 与下一次 acquisition scrub 都必须清成 `None`，防来源泄漏到下一请求。
- `BridgeServices.knowledge_memory` 与 process supervisor 职责分离：它只 clone 同一 `KnowledgeUnitOfWork` Arc；per-session `BridgeBackends` 不得创建 runtime、cancellation token 或 join handle。
- `BridgeServices.knowledge_context` 只是 provider capability 的依赖注入；bridge 不接受 caller 构造的 trusted auth，不缓存 ContextPack，也不得在缺 provider/identity 时回退 legacy global memories/wiki。
- 事件经 `BridgeEventBus` → coordinator（单任务）发射；别绕过 coordinator 直发。
- `prepare.rs` 渲染 prompt 时，Task/Profile 的 lead turn（`harness_active_stage=None`）不应告诉模型有 sub-agent dispatch；只有真正进入 harness stage 后才把 sub-agent 能力写进 prompt。否则 lead turn 会被提示去走不存在的 stage/sub-agent 工具面。
- `execute_with_turn_instructions` 只把本 turn 附加说明拼进 system prompt；`AiEvent::UserMessage`、sidecar capture、conversation history 都必须继续使用原始 prompt。Task/Profile lead policy 走这里，避免把控制指令显示成用户消息。
- `prepare.rs` 按 `GOLISH_RUNTIME_SUPERVISOR`（兼容旧 `GOLISH_EXECUTION_MENTOR`）给 runtime 注入 `ExecutionMonitor`：默认 hard RuntimeSupervisor（`just dev` 直接启用），`shadow` 只记录结构化决策，`soft`/`on` 注入策略指令，`off`/`false`/`0` 关闭。这个默认会增加额外 LLM 调用，改默认值必须有明确产品理由。
- `harness_forced_tool` 是 TaskOrchestrator 到 runtime 的短生命周期 side-channel，目前用于裸 resume 直进 `stage_run`；只由 `bridge_executor` 写入/清空，`prepare.rs` 只负责快照进 `AgenticLoopContext`，不要把它做成长期 session preference。
- `stage_run_reentry_guard` 是 Task request-scoped circuit breaker：top-level owner 第一次升级为 Task 时 reset 一次；同 token 的 lead handoff / nested executor 不得二次 reset，否则 A 的第 3 次 per-org BLOCK 会被错误重开。
- universal gate 必须在 GUI mode 分流、附件、普通 CLI、stage-run 和 conversation history clear/restore 之前 acquire。busy contender 不得 reset cancel、take/replace history 或写 sidechannel。Stop 后 raw depth-0 execution 也不得自行 reset cancel；只有新的 successful acquisition 能开新 cancellation epoch。
- normal return 在 lease 仍持有时 async 清 stage/authz/org/operation/submit-only/forced-tool/deliverable/pending-plan；future drop/unwind 无法 async Drop，因此 next acquisition 在任何执行前重复 scrub。不要清 durable history/profile/worker chain。
- `execute_isolated_with_context` 与 `execute_isolated` 使用同步 recovery slot 保护 durable history：`mem::take` 后不得跨 await 才发布 backup；normal success/error/Stop 主动恢复，abort/panic 留给 next acquisition 在 execution 前恢复。恢复必须先 await history write lock、再从 recovery slot take，避免恢复 future 被 cancel 时再次丢历史。额外 context 只能携带 request-local 数据，不能改变 Task Primary 的 depth=0 工具面。
- `execute_with_context_inner` 的 primary turn 成功后必须直接保留成功结果，不能再 poll provider-dispatch-heavy 的 fallback future。debug GUI 的 TaskOrchestrator/bridge future 本身已有深层 poll 栈；把成功结果也送进 `maybe_failover_to_fallback_model` 会额外叠加约 10 MiB 生成栈帧并打穿 32 MiB Tokio worker stack。只有真实 `Err(primary_error)` 可以进入 failover；不能用继续增大 `RUST_MIN_STACK` 代替这个分支边界。
- GUI bridge replacement 不能创建独立 gate 绕过 owner：每个 bridge 绑定 stable session slot 的 exact generation。init transition 与 request 共用 in-flight bit；shutdown invalidate generation 后，已 clone 但尚未 begin 的 old bridge 永久 stale。foreign-generation lease 不得构造 executor 或清另一 bridge state。
- request acquisition 用 cancellation epoch 封住 gate CAS→reset 窗口：CAS 后到达的 Stop 不能被 reset 覆盖。bridge generation retirement 与普通 Stop 分离；replacement 时 new bridge 继承 old 的 `pending_background` Arc，使 old listener drain 的 completion note 仍由新 bridge 下轮消费。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-bridge agent_bridge
```
