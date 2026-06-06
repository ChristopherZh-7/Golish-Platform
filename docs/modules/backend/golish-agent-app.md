# golish-agent-app

> **一句话职责**：**agent 服务**的 per-domain Tauri command crate（crate-per-service M4）——持有 agent runtime 状态（`AiState` per-session bridges + 窄 `AgentState`）、`ai/` command 子树与 AppState-free 桥接、`conversation_store`。

- **类型**：crate（Layer 5+ · per-domain app）
- **路径**：`backend/crates/golish-agent-app/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 agent 相关 Tauri command（`ai/commands/*`）、agent 会话状态（`AiState`/`AgentState`）时
- 改 AppState-free 桥接（`db_bridge` / `tracking_bridge` / `ai/mod.rs` facade）、`conversation_store` 时
- 排查 agent 命令拿不到 per-session `AgentBridge`、会话未初始化错误时

## 职责

agent 服务的命令面与运行时状态宿主。`AiState` 持有 per-session `AgentBridge` 句柄；`AgentState` 是 agent 命令接收的窄状态（取代巨石 `golish::AppState`）。M4-proper 把 `ai/` 子树（command handlers + 各桥接）和 `conversation_store` 搬到这里，`golish` 仅留薄 shim re-export，命令面走 `commands_facade::{ai, workspace}`。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `AiState` | per-session agent bridges 运行时状态 |
| `AgentState` | agent 命令接收的窄 managed state |
| `ai_session_not_initialized_error` | 会话未初始化错误构造 |
| `ai`（`commands/` `db_bridge/` `tracking_bridge/`） | command handlers + AppState-free 桥接 |
| `conversation_store` | agent 拥有的会话存储 |
| re-export `golish_app_core::{error, runtime}` | 让搬来的 `ai/` 文件保持 `crate::error::*` 路径 |

## 依赖

- **内部**：`golish-app-core`、`golish-agent-bridge`、`golish-agent-kit`、`golish-agent-runtime`、`golish-sub-agents`、`golish-db`、`golish-graphiti`、`golish-sidecar`、`golish-mcp`、`golish-pentest`、`golish-projects`、`golish-session`、`golish-prompts`、`golish-events`、`golish-models`、`golish-context`、`golish-indexer`、`golish-pty`、`golish-llm-providers`、`golish-settings`、`golish-core`
- **外部**：`tauri`、`rig-core`、`graph-flow`、`sqlx`、`ts-rs`、`dotenvy`

## 被谁依赖 / 改动影响面

仅 `golish`（通过 `commands_facade` 聚合到 `generate_handler!`）。是 agent 命令面的唯一宿主。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `ai/` | agent 命令 + 各 bridge + harness 工具 | [→](golish-agent-app/ai.md) |
| `conversation_store/` | 会话/时间线 PG 持久化命令 | [→](golish-agent-app/conversation_store.md) |

## 关键文件

`state.rs`（`AiState` / `AgentState`）、`ai/mod.rs`（facade）。

## 注意事项 / 坑

- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`（如 `ai_send_prompt`）。
- **不变量 I5**：`agents.rs` / `check_recon_tools_cmd` 暴露 ts-rs wire 类型给前端。
- `golish-mcp` 是 path 依赖（非 workspace.dep），跨 workspace 引用注意。
- 命令面经 `golish::commands_facade::{ai, workspace}` glob 暴露给 `generate_handler!`，新增命令要确认 facade 能 glob 到 `__cmd__$name`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app
```
