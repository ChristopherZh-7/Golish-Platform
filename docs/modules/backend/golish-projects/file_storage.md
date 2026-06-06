# golish-projects / file_storage

> **一句话职责**：混合 DB+文件系统架构的项目文件存储——原始捕获文件（JS/HTML/HTTP dump/工具输出/证据）落 `{project_root}/.golish/` 下规范目录，结构化元数据进 PG 并存 `file_path` 引用。

- **类型**：目录模块（属于 crate [`golish-projects`](../golish-projects.md)）
- **路径**：`backend/crates/golish-projects/src/file_storage/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `{project_root}/.golish/` 目录布局（captures/tool-output/scripts/evidence/analysis/temp）时
- 改原始文件落盘/读取（crud_ops）或项目导入导出（import_export）时

## 职责

实现项目级文件存储：原始捕获文件按规范路径落 `{project_root}/.golish/`（`captures/{host}/{port}/{type}` · `tool-output/{tool}/{ts}_{target}` · `scripts/{category}` · `evidence/{finding_id}` · `analysis/{host}` · `temp/`），结构化元数据进 PG 用 `file_path` 引用。

## 公开接口

| 符号 | 说明 |
|---|---|
| `crud_ops::*` | 文件 CRUD（落盘/读取/删除） |
| `import_export::*` | 项目导入导出 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 目录布局常量 + 类型 |
| `crud_ops.rs` | 文件 CRUD |
| `import_export.rs` | 导入导出 |

## 依赖

- `tokio`/std 文件系统、`serde`、`anyhow`；`golish_core::paths`（project root 解析）

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`（部分 API 为未来用）。
- **混合架构约定**：原始 bytes 落文件、结构化进 DB（存 `file_path`）——别把大文件塞 DB。
- 路径基于 `{project_root}/.golish/`；改布局要同步上层（recon-app `asset_intel`、pentest-app `pentest_bridge js_collect` 都往这里写）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-projects file_storage
```
