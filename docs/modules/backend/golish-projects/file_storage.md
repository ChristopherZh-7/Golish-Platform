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

实现项目级文件存储：原始捕获文件按规范路径落 `{project_root}/.golish/`（`captures/{host}/{port}/{type}` · `tool-output/{tool}/{ts}_{target}` · `scripts/{category}` · `evidence/{finding_id}` · `analysis/{host}` · `temp/`），结构化元数据进 PG 用 `file_path` 引用。Reporting 复用同一 seam，在 `.golish/reports/.staging` 暂存并原子 promote 到 `.golish/reports/blobs/<content-key>`；路径校验不在 agent/composition 层复制。

## 公开接口

| 符号 | 说明 |
|---|---|
| `crud_ops::*` | 文件 CRUD（落盘/读取/删除） |
| `import_export::*` | 项目导入导出 |
| `stage_report_artifact` / `verify_report_artifact` / `read_verified_report_artifact` / `discard_staged_report_artifact` | 报告 blob 的暂存、校验读取与 staging 清理；读取只在 named-entry/handle identity、digest、length 全部复核后返回 bytes；Unix 使用 anchored dirfd/`*at`，Windows 使用 retained capability directory handles |
| `promote_report_artifact` → `ReservedReportArtifact` | 在 per-content advisory lock 内 hard-link put-if-absent、校验并刷新 blob mtime；返回的 reservation 必须持有到 DB artifact attach 完成 |
| `gc_report_artifacts` | grace-period orphan staging/blob GC；每个待删 content key 取得同一锁后重新检查 grace，Unix 用 `unlinkat`，Windows 用已验证 handle 的 `FileDispositionInfo` 删除 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 目录布局常量 + 类型 |
| `crud_ops.rs` | 文件 CRUD |
| `report_artifacts_unix.rs` | Unix report 专用 dirfd/openat/linkat/unlinkat/fstatat、binding revalidation、flock reservation 与 GC |
| `report_artifacts_windows.rs` | Windows report 专用 cap-std/cap-primitives capability handles、reparse/junction 拒绝、no-delete-share 文件绑定、hard-link publication、fs2 reservation 与 handle-based GC |
| `import_export.rs` | 导入导出 |

## 依赖

- `tokio`/std 文件系统、`serde`、`anyhow`；Unix 使用 `libc` 的 `openat/mkdirat/linkat/unlinkat/fstatat/flock/futimens`；Windows 使用 `cap-std`/`cap-primitives`、`fs2` 与 Win32 handle identity；`golish_core::paths`（project root 解析）

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`（部分 API 为未来用）。
- **混合架构约定**：原始 bytes 落文件、结构化进 DB（存 `file_path`）——别把大文件塞 DB。
- 路径基于 `{project_root}/.golish/`；改布局要同步上层（recon-app `asset_intel`、pentest-app 的 JS/API browser capture、`golish/reporting_artifact_store` 都消费这里的约定）。
- Report content key、format、revision id 均经过窄校验；caller 不能传绝对路径、`..` 或任意扩展名。
- Composition root 传入已经解析的 server-owned canonical project root。Unix 存储层仍从原始绝对路径的 `/` 开始逐组件 `openat(O_DIRECTORY|O_NOFOLLOW)`，不做 `symlink_metadata → canonicalize` 检查窗口；之后 stage/promote/verify/discard/GC 全部使用已打开 dirfd，并以 `fstatat(AT_SYMLINK_NOFOLLOW)` 对比 device/inode binding，project root 或 parent pathname 被 rename/symlink 替换时 fail closed。
- Windows 存储层从卷根开始逐组件 no-follow 打开并保留全部目录 capability handle，拒绝 symlink、mount-point/junction 等所有 reparse point；保留的 ancestor handle 禁止运行期 rename/delete replacement。普通 artifact/lock handle 显式不共享 `FILE_SHARE_DELETE`；hash 后复核 filename→Win32 handle identity，promotion 删除 staging 后还会以 read-only-share handle 重开 blob、核对原 identity 并重算 hash/length；discard/GC 只对已复核且持有 `DELETE` access 的同一 handle 设置 delete disposition，禁止关闭后按名字删除。
- `.golish/reports/.locks/sha256/<hash>.<ext>.lock` 同时使用进程内 keyed mutex 与跨进程独占文件锁（Unix `flock` / Windows `fs2`）。Windows reservation 额外保留 lock filename、handle 与 volume/file-index identity，并在每个 mutation 边界复核 name→same handle。publisher 从 promote 前持有到 DB attach 后；旧 orphan 被复用时在锁内刷新 mtime。GC 即使拿到 stale DB reference snapshot，也必须等待同锁并在删除前重新检查 mtime/grace。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-projects file_storage
```
