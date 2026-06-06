# golish-agent-app / conversation_store

> **一句话职责**：前端会话与时间线持久化的 Tauri 命令——用 PostgreSQL 存储取代 `workspace.json` 读写（会话列表、加载、workspace 偏好）。

- **类型**：目录模块（属于 crate [`golish-agent-app`](../golish-agent-app.md)）
- **路径**：`backend/crates/golish-agent-app/src/conversation_store/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改前端会话列表/时间线持久化、workspace 偏好读写的 Tauri 命令时
- 排查从 `workspace.json` 迁到 PG 的会话/偏好数据时

## 职责

提供会话与时间线的 PG-backed Tauri 命令（取代旧 `workspace.json`）。DTO 镜像 `repo::conversation_store` 的 SELECT 列序（`ConvListRow` / `WorkspacePrefsRow`），命令取窄 `DbState`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `#[tauri::command]` 会话/偏好命令 | list / load / 偏好读写 |
| 会话/偏好 DTO | 镜像 `repo::conversation_store` 行形状 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | DTO + Tauri 命令 |

## 依赖

- crate 内 `error::GolishError`、`state::DbState`；`golish-db`（`repo::conversation_store`）、`serde`、`chrono`

## 注意事项 / 坑

- DTO 的字段顺序**镜像 SELECT 列序**：改 repo 查询列顺序要同步这里的元组类型，否则映射错位。
- 取窄 `DbState`（非 AppState）；走 repo scoped CRUD（I2）。
- **不变量 I4/I5**：命令命名 `<domain>_<verb>_<object>`；wire DTO 应考虑 ts-rs 同步。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-app conversation_store
```
