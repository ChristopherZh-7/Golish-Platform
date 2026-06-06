# golish-recon-app / scan_runner

> **一句话职责**：scan-runner 操作的 Tauri 命令薄包装——纯逻辑（WhatWeb/Nuclei/feroxbuster）在 `golish-scan-runner` crate，这里把 `DbState` 适配到库 API，并把扫描存储回调映射到 `targets::db_directory_entry_add`。

- **类型**：目录模块（属于 crate [`golish-recon-app`](../golish-recon-app.md)）
- **路径**：`backend/crates/golish-recon-app/src/scan_runner/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改扫描（WhatWeb/Nuclei/ferox）的 Tauri 命令、进度事件、存储回调映射时

## 职责

scan-runner 命令面：thin `#[tauri::command]` 包装，把 `golish-scan-runner` 的 runner（WhatWeb/Nuclei/feroxbuster）接到 `DbState` + 事件发射；存储回调 adapter 把扫描结果映射到 `crate::targets::db_directory_entry_add`。

## 公开接口

| 符号 | 说明 |
|---|---|
| 扫描 Tauri 命令 | WhatWeb/Nuclei/ferox 启动 + 取消 |
| re-export `runner::{FeroxScanOptions, NucleiScanOptions, WhatWebOptions, ScanResult, ScanProgress, PocMatch}` | 库类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 命令包装 + 存储回调 adapter |

## 依赖

- crate 内 `targets`（存储回调）、app-core（`DbState`/`TauriEventEmitter`）；`golish-scan-runner`

## 注意事项 / 坑

- 纯扫描逻辑在 `golish-scan-runner`（无 Tauri）；本模块只适配 + 存储回调，别把扫描逻辑搬进来。
- 存储回调写 directory entry 经 `targets::db_*`（同 crate 直写 helper）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-recon-app scan_runner
```
