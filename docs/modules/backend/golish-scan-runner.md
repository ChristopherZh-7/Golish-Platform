# golish-scan-runner

> **一句话职责**：扫描器调度引擎——WhatWeb 指纹、Nuclei 定向扫描（指纹→PoC 匹配）、feroxbuster 目录爆破的统一 dispatch；无 Tauri 依赖。

- **类型**：crate（Layer 3 领域）
- **路径**：`backend/crates/golish-scan-runner/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 WhatWeb/Nuclei/feroxbuster 扫描执行、指纹→PoC 匹配、扫描进度事件时

## 职责

GUI/AI 调用的各类 pentest 扫描器的调度表。进度通过 `golish_core::EventEmitterHandle` 发出（前端壳提供 `TauriEventEmitter` adapter）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_whatweb` / `WhatWebOptions` | WhatWeb 指纹 |
| `run_nuclei_targeted` / `match_pocs_for_target` / `NucleiScanOptions` | Nuclei 定向扫描 + PoC 匹配 |
| `run_feroxbuster` / `FeroxScanOptions` | 目录爆破 |
| `ScanStorage` / `ScanProgress` / `ScanResult` / `PocMatch` | 存储/进度/结果 |

## 依赖

- **内部**：`golish-core`、`golish-db`、`golish-shell-exec`

## 被谁依赖 / 改动影响面

`golish`、`golish-app-core`、`golish-recon-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `nuclei/` | Nuclei 定向扫描 + 指纹→PoC 匹配引擎 | [→](golish-scan-runner/nuclei.md) |

## 关键文件

`whatweb.rs`、`feroxbuster.rs`、`helpers.rs`、`storage.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- 无 Tauri 依赖：进度走 `EventEmitterHandle`，别直接耦合 Tauri。
- 扫描产物应能落进 evidence（与 golish-pentest 的 ledger 协作，I7）。
- 相关：`docs/superpowers/plans/scan-workflow-implementation.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner
```
