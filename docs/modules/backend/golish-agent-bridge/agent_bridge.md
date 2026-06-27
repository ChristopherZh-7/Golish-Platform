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

`AgentBridge` 是 app↔runtime 的核心句柄，按关注点分解：`BridgeEventBus`（事件发射/seq/frontend-ready 缓冲/coordinator/transcript）、`BridgeLlmConfig`（client + provider/model + provider 特定配置）、`BridgeServices`（DB/PTY/sidecar/indexer/settings 可选句柄）、`BridgeAccessControl`（tool policy/HITL/agent mode/loop detection）、`BridgeSession`（历史 + 持久化）。顶层字段是跨切面身份/编排状态（workspace/tool registry/sub-agents/context/MCP）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AgentBridge` | 桥接主体 |
| `BridgeBackends` | 后端装配束 |
| `constructors`（`new` / `with_*`） | 构造 impl |
| `execute_with_turn_instructions` | 单 turn 隐藏 system instruction 注入；UI events / sidecar / history 仍记录原始用户 prompt |
| `BridgeEventBus` / `BridgeLlmConfig` / `BridgeServices` / `BridgeAccessControl` / `BridgeSession` | 5 子系统 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `AgentBridge` 结构 + 子系统组合 |
| `constructors/` | `new` + `with_*` builder |

## 依赖

- crate 内（agent-kit/runtime 经 crate 根 re-export）、`golish-events`（coordinator/transcript）、`golish-session`

## 注意事项 / 坑

- 5 子系统是有意分解（隔离关注点）；加字段先想归哪个子系统，别全堆顶层。
- 事件经 `BridgeEventBus` → coordinator（单任务）发射；别绕过 coordinator 直发。
- `prepare.rs` 渲染 prompt 时，Task/Profile 的 lead turn（`harness_active_stage=None`）不应告诉模型有 sub-agent dispatch；只有真正进入 harness stage 后才把 sub-agent 能力写进 prompt。否则 lead turn 会被提示去走不存在的 stage/sub-agent 工具面。
- `execute_with_turn_instructions` 只把本 turn 附加说明拼进 system prompt；`AiEvent::UserMessage`、sidecar capture、conversation history 都必须继续使用原始 prompt。Task/Profile lead policy 走这里，避免把控制指令显示成用户消息。
- `prepare.rs` 按 `GOLISH_RUNTIME_SUPERVISOR`（兼容旧 `GOLISH_EXECUTION_MENTOR`）给 runtime 注入 `ExecutionMonitor`：默认 hard RuntimeSupervisor（`just dev` 直接启用），`shadow` 只记录结构化决策，`soft`/`on` 注入策略指令，`off`/`false`/`0` 关闭。这个默认会增加额外 LLM 调用，改默认值必须有明确产品理由。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-bridge agent_bridge
```
