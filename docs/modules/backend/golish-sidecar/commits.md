# golish-sidecar / commits

> **一句话职责**：L2 暂存提交——把提交存成标准 git format-patch 文件（可 `git am` 应用），`PatchManager` 做文件系统 CRUD + 应用，附带 golish 专属 metadata sidecar 文件。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/commits/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改暂存补丁的 git format-patch 格式化/解析、`git am` 应用时
- 改补丁 metadata（`PatchMeta`/`BoundaryReason`）或 diff 生成时

## 职责

把 sidecar 捕获的改动暂存为标准 git format-patch 文件（`patches/staged/`），`PatchManager` 提供文件系统 CRUD + 经 `git am` 应用，并存 golish 专属 metadata sidecar（如边界原因）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `PatchManager` | 补丁文件系统 CRUD + `git am` 应用 |
| `PatchMeta` / `StagedPatch` / `BoundaryReason` | 补丁元数据 / 暂存补丁 / 边界原因 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `manager.rs` | `PatchManager`（CRUD + `git am`） |
| `format.rs` | git format-patch 文本格式化 + 解析 |
| `diff.rs` | diff 生成（字符串 + git-backed） |
| `types.rs` | `PatchMeta`/`BoundaryReason`/`StagedPatch` + slug helper |

## 依赖

- `git`（format-patch / am，经进程）；`anyhow`

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`——commit 暂存系统已实现但**尚未集成**进主流程；改它先确认调用方。
- 补丁是标准 git format-patch（可 `git am`）：保持格式合规，否则应用失败。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar commits
```
