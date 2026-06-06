# golish-session

> **一句话职责**：AI 会话持久化——会话归档、对话日志、transcript 导出，双写文件（via golish-core）+ Postgres 伴生模块。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-session/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改会话归档/列表/加载、会话快照 DTO、双写持久化时
- 会话选择器、最近会话相关时

## 职责

集成 `golish_core::session` 做会话持久化。in-memory 活动会话 + 双写磁盘 + 可选 Postgres 句柄；提供读侧 helper 与 sidecar 元数据抽取供会话选择器。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GolishSessionManager` | 活动会话 + 双写 |
| `find_session` / `list_recent_sessions` / `load_session` | 读侧 helper |
| `SessionPersistence`（`db` 模块） | Postgres 持久化伴生 |
| `GolishMessageRole` / `GolishSessionMessage` / `GolishSessionSnapshot` / `SessionListingInfo` | DTO |

## 依赖

- **内部**：`golish-core`（文件持久化基座）
- **外部**：`rig-core`

> 注：`db` 子模块做 Postgres 持久化，但本 crate 的 `Cargo.toml` 仅依赖 `golish-core` + `rig-core`（Postgres 句柄由调用方传入，未直接依赖 golish-db crate）。

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`、`golish-agent-bridge`。

## 关键文件（无目录子模块）

| 文件 | 作用 |
|---|---|
| `manager.rs` | `GolishSessionManager` 双写 |
| `archive.rs` | 读侧 helper + sidecar 元数据 |
| `types.rs` | DTO |
| `db.rs` | Postgres 持久化伴生 |

## 注意事项 / 坑

- DTO 跨 IPC 给前端会话选择器，注意 ts-rs 同步（I5）。
- 「双写」=文件 + 可选 PG，改持久化注意两侧一致性。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-session
```
