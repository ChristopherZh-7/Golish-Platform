# golish-recon-app / targets

> **一句话职责**：target & directory-entry 数据层——core target DTO（`Target`/`TargetType`/`Scope`/`TargetStatus`）+ `ReconUpdate`/`DirectoryEntry` + 纯 DB helper（无 Tauri 注解，供命令与其它模块直写）+ `#[tauri::command]` 入口。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/targets/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 target/directory-entry 的 CRUD 命令、DB helper、recon 扩展扫描 payload 时

## 职责

target 域的数据层。`types` core DTO + DB row 适配；`recon` 扩展扫描 payload（`ReconUpdate`）+ `DirectoryEntry`；`db` 纯 DB helper（无 Tauri，供命令 + 直写方共用）；`cmds`/`directory` Tauri 命令入口。

## 公开接口

| 符号 | 说明 |
|---|---|
| `cmds::*` / `directory::*`（Tauri 命令） | target / directory entry 管理；带 target_id 的 directory list 走 current-owner repo read，旧 project row 不随 moved target 暴露 |
| `db::*`（纯 DB helper） | 无注解 DB 写（供直写方复用） |
| `Target` / `TargetType` / `Scope` / `TargetStatus` / `ReconUpdate` / `DirectoryEntry` | DTO |

## 关键文件

| 文件 | 作用 |
|---|---|
| `cmds.rs` / `directory.rs` | Tauri 命令 |
| `db.rs` | 纯 DB helper |
| `types.rs` / `recon.rs` | core DTO / recon payload |

## 依赖

- crate 内 app-core（`DbState`）、`golish-db`（repo::targets）；`ts-rs`

## 注意事项 / 坑

- 部分 DTO 与 `golish-app-core::domain::targets`（跨服务共享）对应——跨服务读写走 ports，本地命令走 repo。
- `directory_entry_list(target_id=...)` 不能调用裸 `directory_entries::list_by_target`；必须要求 current target 仍 in-scope 且 row project 与 current target project 一致。无 target_id 的 project-wide list 保持显式 `project_path` 语义。
- `db` helper 无 Tauri 注解，是 recon 内/scan_runner 回调的直写点；active scan 回调必须用 `db_directory_entry_add_guarded`，把 launch `TargetWriteGuard` 传到 repo 同事务锁校验；改签名要查所有调用方。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app targets
```
