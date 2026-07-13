# golish / commands_facade

> **一句话职责**：命令门面——每个 `<domain>.rs` 用 `pub use` re-export 该域所有 `#[tauri::command]`，是「域 X 暴露什么命令」的**单一事实源**；`commands_registry.rs` 用 `use commands_facade::*` 让 `generate_handler!` 在调用点解析到扁平标识符。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/commands_facade/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改/删任何 Tauri 命令（必须在对应 facade 文件 `pub use`）时
- 加新命令域（新 `<domain>.rs` + `pub mod`）时

## 职责

把分散在各 home 模块/各 app crate 的 `#[tauri::command]` 按域 re-export，给 `commands_registry.rs` 的 `generate_handler!` 提供可解析的扁平标识符。`generate_handler!` 是 proc-macro，看不穿 `pub use A::*`，故 registry 仍列扁平名，但「域暴露什么」的事实源在此。

## 公开接口

| 域文件（18 个） | 说明 |
|---|---|
| `ai` / `pentest` / `vuln_intel` / `findings` / `evidence` / `methodology`（实为各处） | agent/pentest/vuln 命令 |
| `attack` | Candidate review list/decide/resume 与 Attempt list 四个 durable 命令 |
| `reporting` | DB-authoritative build/read/history/artifact 与 explicit final publication 命令 |
| `asset_intel` / `intel_providers` / `integrations` / `organization_recon` | recon 命令 |
| `vault` / `wiki` / `sidecar` / `settings` / `indexer` / `mcp` / `git_pty` / `workspace` | 平台/工具命令 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub mod <domain>` 声明（18 个） |
| `<domain>.rs` | 该域 `pub use` 命令 re-export |

## 依赖

- 各 home 模块 + 各 per-domain app crate（glob 其 `__cmd__$name`）

## 注意事项 / 坑

- **AGENTS.md §2.2 / I4**：加命令只动两个文件（命令 home 模块 + 对应 facade 文件）；**禁止**直接在 `commands_registry.rs` 加 `use crate::foo::commands::*` glob。
- facade glob 必须能解析到 `#[tauri::command]` 生成的 `__cmd__$name` 宏（macro_export 到 crate 根）。
- authoritative temporal graph 命令当前经 `ai.rs` 暴露：`knowledge_graph_query_scoped` / `knowledge_graph_rebuild_scope`；不要与 legacy `kg_*` 混为同一授权语义。

## 测试入口

```bash
cd backend && cargo nextest run -p golish commands_facade
```
