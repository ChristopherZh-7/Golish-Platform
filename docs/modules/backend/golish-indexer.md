# golish-indexer

> **一句话职责**：代码索引基座——owns 索引 trait、状态容器、路径解析、vtcode-indexer 后端实现，外加 codebase 路径解析与 git 工具。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-indexer/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 代码索引、语义代码搜索、codebase 路径解析、git stats/worktree 时
- home 视图/codebase 管理命令相关时

## 职责

提供 `IndexerBackend` trait + `IndexerState` + vtcode 后端实现。无 Tauri 依赖、不依赖 L3 crate（agent/tools 等），是更高层消费者的底座。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `IndexerBackend`(trait) / `IndexerState` / `CodeSearchResult` | 索引核心 |
| `compute_index_dir` / `find_existing_index_dir` / `migrate_index` | 索引目录路径 |
| `expand_home_dir` / `contract_home_dir` / `get_codebase_file_count` | 路径 helper |
| `get_git_stats` / `format_relative_time` | git 工具 |
| `initialize_vtcode_indexer` | vtcode 后端初始化 |
| `types`：`CodebaseInfo` / `ProjectInfo` / `WorktreeCreated` … | 与 Tauri 命令层共享 DTO |

## 依赖

- **内部**：`golish-core`、`golish-settings`

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`、`golish-agent-kit`、`golish-agent-bridge`、`golish-agent-runtime`。

## 关键文件（无目录子模块）

`state.rs`、`paths.rs`、`path_helpers.rs`、`git_helpers.rs`、`vtcode_bridge.rs`、`types.rs`。

## 注意事项 / 坑

- 刻意**不依赖 L3**（agent/tools），保持底座干净；别在这里反向引入上层依赖。
- `types` 里的 DTO 跨 IPC 给前端的，注意 ts-rs 同步（I5）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-indexer
```
