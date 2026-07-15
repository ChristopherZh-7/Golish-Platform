# golish-recon-app / scan_runner

> **一句话职责**：Recon scan-runner 的 Tauri 命令安全边界——先把 caller 的 target/project/URL 绑定到 immutable `TargetWriteGuard`，再把 WhatWeb/feroxbuster 交给 `golish-scan-runner`。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/scan_runner/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Recon 扫描（WhatWeb/ferox）的 Tauri 命令、进度事件、存储回调映射时

## 职责

scan-runner 命令面：`#[tauri::command]` 包装先通过 `authorize_scan_target` 加载 current in-scope/project-bound target guard，并要求 caller `project_path` 精确匹配、请求 URL exact origin 属于 target name/value 或 confirmed-open `ports[].url`；随后把授权快照、`DbState` 和事件发射交给 WhatWeb/ferox runner。存储回调 adapter 把 ferox 结果映射到 `crate::targets::db_directory_entry_add_guarded`。旧手动 Nuclei 匹配/启动/取消 IPC 已移除。

## 公开接口

| 符号 | 说明 |
|---|---|
| 扫描 Tauri 命令 | WhatWeb/ferox 启动 |
| re-export `runner::{FeroxScanOptions, WhatWebOptions, ScanResult, ScanProgress}` | 库类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 命令包装 + 存储回调 adapter |

## 依赖

- crate 内 `targets`（存储回调）、app-core（`DbState`/`TauriEventEmitter`）；`golish-scan-runner`

## 注意事项 / 坑

- 纯扫描逻辑在 `golish-scan-runner`（无 Tauri）；本模块只适配 + 存储回调，别把扫描逻辑搬进来。
- 两条 IPC 不得把 caller 的裸 `target_url/target_id/project_path` 直接传给网络 runner；必须先得到 `AuthorizedScanTarget`。缺 project、out-of-scope、foreign project/origin、未确认端口 origin 全部在任何网络前拒绝。
- 不要在这里恢复 Nuclei 手动命令；AI stage 的 Nuclei capability 属于 `golish-pentest-app`。
- 存储回调写 directory entry 必须经 `targets::db_directory_entry_add_guarded`，不能退回不带 launch witness 的 legacy helper。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app scan_runner
```
