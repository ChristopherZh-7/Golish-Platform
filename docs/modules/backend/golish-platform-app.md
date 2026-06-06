# golish-platform-app

> **一句话职责**：**platform 服务**的 per-domain Tauri command crate（crate-per-service M5 末叶）——凭据保险库（vault，加密存储）、审计/活动时间线（audit）、项目速记（notes）、终端会话录制（recordings）。

- **类型**：crate（Layer 5+ · per-domain app · DAG 叶子）
- **路径**：`backend/crates/golish-platform-app/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改跨域平台服务 Tauri command（凭据库、审计时间线、笔记、终端录制）时
- 改 vault 加密存储 / `vault_validate` 凭据探测、audit 跨服务读时间线时

## 职责

支撑 workspace UI 的跨域平台服务命令面（crate-per-service 拆分的末叶 M5）。每个命令取窄 `golish_app_core::DbState`，不取巨石 `golish::AppState`。`audit.rs` 的跨服务读（`passive_scans`=recon、`agent_logs`/`search_logs`=agent）走共享 `golish_db::repo::*`（L2），故本 crate **零 sibling 依赖**、保持干净叶子。

## 公开接口 / 关键类型

| 模块 | 说明 |
|---|---|
| `vault` | 凭据保险库（R7 对齐的密钥存储，静态加密；`vault_validate` 探测凭据） |
| `audit` | 审计日志 + 跨服务活动时间线 |
| `notes` | 项目速记（暴露 ts-rs wire 类型） |
| `recordings` | 终端会话录制 |

## 依赖

- **内部**：`golish-app-core`、`golish-core`（vault obfuscate/deobfuscate + 时间助手）、`golish-db`（repo 层 + 跨服务读）
- **外部**：`tauri`、`ts-rs`、`sqlx`、`reqwest`

## 被谁依赖 / 改动影响面

仅 `golish`（通过 `commands_facade::{vault, workspace}` 聚合）。是 platform 命令面的唯一宿主，零 sibling 依赖。

## 子模块（目录模块，各有卡片）

本 crate 无目录子模块（全是单文件模块），故无子卡。

## 关键文件

| 文件 | 作用 |
|---|---|
| `vault.rs` | 凭据库（加密存储 + 探测） |
| `audit.rs` | 审计 + 跨服务时间线（含 raw sqlx allowlist） |
| `notes.rs` | 速记（ts-rs wire 类型） |
| `recordings.rs` | 终端录制（含 raw sqlx allowlist） |

## 注意事项 / 坑

- **不变量 I2**：vault/notes 等 CRUD 验资源所有权（IDOR），`scoping` 守卫来自 app-core。
- **raw SQL allowlist（P0-3）**：`audit.rs` / `recordings.rs` 用裸 `sqlx`，已登记在 `check_repo_ownership.py` ALLOWLIST（layer A）；跨服务读切到 ports（`AgentLogReadPort` + recon `passive_scans` port）是后续 milestone（layer B），改这两文件别破坏 allowlist 约束。
- **不变量 I5**：`notes.rs` 暴露 ts-rs wire 类型给前端。
- vault 是敏感数据，改加密/混淆逻辑属高风险，遵守 AGENTS.md §2.7。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-platform-app
```
