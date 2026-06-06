# golish / projects

> **一句话职责**：项目配置 Tauri 命令薄包装——纯逻辑（config CRUD、文件存储、目录结构）在 `golish-projects` crate，这里只提供 Tauri 命令包装 + re-export 库类型。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/projects/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改项目配置的 Tauri 命令（创建/打开/列举项目、`.golish/` 目录）时

## 职责

`golish-projects`（项目存储）的 Tauri 命令面：thin `#[tauri::command]` 包装库 API。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | 项目配置 Tauri 命令 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub mod commands` |
| `commands/` | 项目命令 |

## 依赖

- `golish-projects`、`tauri`

## 注意事项 / 坑

- 纯逻辑在 `golish-projects`（无 Tauri）；本模块只适配，别把 config CRUD/文件布局搬进来。

## 测试入口

```bash
cd backend && cargo nextest run -p golish projects
```
