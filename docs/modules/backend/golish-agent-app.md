# golish-agent-app

> **一句话职责**：**agent 服务**的 per-domain Tauri command crate（crate-per-service M4）——持有 agent runtime 状态、命令/桥接，以及 process-shared canonical Memory Fabric 与 scoped ContextPack DB adapters。

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
| `AiState` | per-session agent bridges + stable session request slots/generation/lifecycle 运行时状态 |
| `AgentState` | agent 命令接收的窄 managed state |
| `DbTrustedOperatorPrincipalProvider` | 从唯一 active `operator_principals` 行构造 desktop/CLI opaque principal；API 无 caller-selected id |
| `knowledge_graph_query_scoped` / `knowledge_graph_rebuild_scope` | authoritative temporal graph IPC；server-derived local principal + stable project/exact org DB binding |
| `PgKnowledgeMemory` / `KnowledgeMemoryRuntime` | canonical UoW + outbox/document/embedding/graph DB adapters；runtime handle 由 desktop/CLI composition root exactly once 启停 |
| `PgKnowledgeContextAdapter` | DB ownership + server-owned principal policy + local temporal graph 组成的 C7 scoped ContextPack provider |
| `ai_get_stage_team_read_model` / `ai_resolve_stage_team_recovery` | DB-authoritative Stage Team hierarchy 与 LocalDesktop exact-CAS unknown-tool recovery；绝不提供 replay tool 入口 |
| `attack_list_verification_queue` / `attack_resolve_candidate_recovery` | Candidate TerminalIntent/action/recovery/Wave 与 pending FactDelta enrichment 的安全 typed read/mutation surface |
| `ai_session_not_initialized_error` | 会话未初始化错误构造 |
| `ai`（`commands/` `db_bridge/` `tracking_bridge/`） | command handlers + AppState-free 桥接 |
| `conversation_store` | agent 拥有的会话存储 |
| re-export `golish_app_core::{error, runtime}` | 让搬来的 `ai/` 文件保持 `crate::error::*` 路径 |

## 依赖

- **内部**：`golish-app-core`、`golish-agent-bridge`、`golish-agent-kit`、`golish-agent-runtime`、`golish-sub-agents`、`golish-db`、`golish-graphiti`、`golish-memory-app`、`golish-memory-domain`、`golish-sidecar`、`golish-mcp`、`golish-pentest`、`golish-projects`、`golish-session`、`golish-prompts`、`golish-events`、`golish-models`、`golish-context`、`golish-indexer`、`golish-pty`、`golish-llm-providers`、`golish-settings`、`golish-core`
- **外部**：`tauri`、`rig-core`、`graph-flow`、`sqlx`、`ts-rs`、`dotenvy`

## 被谁依赖 / 改动影响面

仅 `golish`（通过 `commands_facade` 聚合到 `generate_handler!`）。是 agent 命令面的唯一宿主。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `ai/` | agent 命令 + 各 bridge + harness 工具 | [→](golish-agent-app/ai.md) |
| `conversation_store/` | 会话/时间线 PG 持久化命令 | [→](golish-agent-app/conversation_store.md) |

## 关键文件

`state.rs`（`AiState` / `AgentState`，含 trusted principal provider）、`operator_principal.rs`（DB-backed actor provider）、`ai/mod.rs`（facade）。

## 注意事项 / 坑

- **不变量 I4**：命令命名 `<domain>_<verb>_<object>`（如 `ai_send_prompt`）。
- **不变量 I5**：`agents.rs` / `check_recon_tools_cmd` 暴露 ts-rs wire 类型给前端。
- privileged command 必须调用 `AgentState.operator_principal_provider.current(channel)`；请求 DTO 中出现 actor identity 属于安全回归。
- temporal graph organization request 只携带 stable `project_scope_id + organization_id_at_time`；后端还必须验证 active project row 及 exact live-path/sealed-snapshot binding。global-sanitized 也必须先解析 active local principal。
- `golish-mcp` 是 path 依赖（非 workspace.dep），跨 workspace 引用注意。
- 命令面经 `golish::commands_facade::{ai, workspace}` glob 暴露给 `generate_handler!`，新增命令要确认 facade 能 glob 到 `__cmd__$name`。
- `AiState` 的 request slot 必须跨 `AgentBridge` generation：init 在构建前 fail-fast reserve，同 session concurrent init 不排队；shutdown 先 invalidate/remove 再 cancel returned Arc。GC 必须同时确认 wrapper slot 和内部 `SessionRequestSlot` 都没有 late bridge/request lease 引用；否则 same-id 新 slot 会绕过仍在 unwind 的 old owner。busy shutdown 当下不能回收时，后续 init 的 opportunistic sweep 只按相同安全条件清 tombstone。
- `ai/commands/bridge_config.rs` 用同一个 `GolishDbRepoProvider` Arc 同时装配 generic DB reads 与 `RuntimeMemoryRepository`，并把后者注入 bound-chain persistence/AgentBridge；V2 worker 不得回退 raw chain SQL 或 shared deliverable sink，legacy chat 保持原路径。
- `AgentState.knowledge_memory` 与 process supervisor 必须来自同一个 adapter Arc；`bridge_config.rs` 只注入 UoW handle。P1 final-seal/P2 Attempt 在各自 DB compound transaction 接 inner seam 前，不能宣称 canonical producer atomic 闭环完成。
- `PgKnowledgeMemory` assertion promoter 对每个非空 catalog route 使用显式 authority policy；Candidate/Post-Exploit/Cleanup terminal event 的 derived Assertion 只能信任 envelope + sealed frozen scope，严格 payload 只承载事实内容。只有 reason-only blocked Candidate（persisted blocker reason、无 audit evidence、无 FactDelta）在通过 exact sealed operation/project/snapshot/org/source authority 后允许 intentional suppression；无 authority 不得借 suppression 绕过。Candidate `fact_delta_count > 0` 在 typed evidence-role 字段落地前 fail closed；其他投影不得由测试手工 ACK `succeeded_suppressed` 代替。
- `bridge_config.rs` 的 ContextPack provider 必须来自当前 `DbState` pool；request/model 不传 actor、project path 或 trusted context。检索失败不得接 legacy global fallback。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app
```
