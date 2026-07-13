# golish-projects

> **一句话职责**：项目配置存储与文件管理——项目生命周期（增删改查）+ `{project_root}/.golish/` 下的磁盘目录结构（captures/tool output/evidence/scripts/analysis）；无 Tauri 依赖。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-projects/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改项目创建/加载/保存/删除、`ProjectConfig`、`{root}/.golish/` 磁盘结构、workspace 时

## 职责

owns 项目生命周期与 on-disk 目录结构。应用层提供薄 `#[tauri::command]` 包装。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `ProjectConfig` | 项目配置 struct |
| `create_project` 系列：`load_project` / `save_project` / `delete_project` / `list_projects` / `load_workspace` / `save_workspace` | CRUD |
| `PentestProjectConfig`（`file_storage`） | on-disk 文件管理 |
| `ReservedReportArtifact`（`file_storage`） | 跨 Unix/Windows 的 content-addressed report blob + publication 期 per-key advisory-lock reservation；Drop 才释放 |

## 依赖

- **内部**：无（零内部 golish 依赖）

## 被谁依赖 / 改动影响面

`golish`、`golish-pentest-app`、`golish-recon-app`、`golish-agent-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `file_storage/` | `{root}/.golish/` 下文件管理 | [→](golish-projects/file_storage.md) |

## 关键文件

`schema.rs`（`ProjectConfig`）、`storage.rs`（CRUD）。

## 注意事项 / 坑

- 磁盘结构在 `{project_root}/.golish/`（captures/tool output/evidence/scripts/analysis）；与 `~/.golish/projects/<slug>/` 的项目注册区分。
- `ProjectConfig` 跨 IPC 给前端，注意 ts-rs 同步（I5）。
- Report artifact 路径在 Unix 以 dirfd/`*at` 固定，在 Windows 以 retained capability handle 固定；两端都拒绝 symlink/reparse ancestor，并以 hard-link 实现 atomic put-if-absent。Windows 普通/lock handle 不共享 delete，hash 后及 promotion 返回前复核 name→handle identity/content，orphan 删除走 verified-handle disposition。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-projects
```
