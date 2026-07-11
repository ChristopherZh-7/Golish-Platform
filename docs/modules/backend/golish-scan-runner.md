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
| `authorize_scan_target` / `AuthorizedScanTarget` | legacy GUI scan 的 current-owner + exact-origin 启动授权快照 |
| `ScanStorage` / `ScanProgress` / `ScanResult` / `PocMatch` | 存储/进度/结果 |

## 依赖

- **内部**：`golish-core`、`golish-db`、`golish-pentest-domain`（exact Web Origin）、`golish-shell-exec`

## 被谁依赖 / 改动影响面

`golish`、`golish-app-core`、`golish-recon-app`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `nuclei/` | Nuclei 定向扫描 + 指纹→PoC 匹配引擎 | [→](golish-scan-runner/nuclei.md) |

## 关键文件

`authorization.rs`、`whatweb.rs`、`feroxbuster.rs`、`helpers.rs`、`storage.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- 无 Tauri 依赖：进度走 `EventEmitterHandle`，别直接耦合 Tauri。
- 扫描产物应能落进 evidence（与 golish-pentest 的 ledger 协作，I7）。
- `run_whatweb` / `run_nuclei_targeted` / `run_feroxbuster` 只接受 `AuthorizedScanTarget`，不能重新引入裸 target id/project。调用方预授权后，runner 完成 tool lookup/参数准备，再在 guarded audit 前复核一次、每次 command spawn 紧前再复核同一个 raw witness；任一 target org/project/scope/name/value/ports 漂移必须 0 spawn。
- 输出也沿用 launch guard：WhatWeb fingerprint batch、Nuclei finding+passive log、ferox directory entry+敏感 finding、started→completed/failed scan audit 都在各自短事务先锁 target；scanner 输出 URL 必须仍是同 exact origin。非零退出、exit=0 但 stderr 有 runtime/network failure、JSONL 畸形都不能变成 clean empty/success。
- caller process override 面 fail-closed：WhatWeb 拒绝 proxy/extra_args，并固定 `--follow-redirect=never --max-redirects=0`，避免已授权 origin 用 30x 让真实请求越界；Nuclei 拒绝 proxy/template_path/extra_args/positive tags、路径或 wildcard template id，固定 `-dr -ni -dut`；ferox absolute/network-path base 不能跨 origin，自定义 wordlist 仅允许 canonical `workspace/1.txt` 或 `workspace/.golish/wordlists/**` regular file。
- `match_pocs_for_target` 是真实 Nuclei 网络扫描的模板准备边界：只读 current-owner fingerprints；启动时捕获 `TargetWriteGuard`，legacy target-field backfill 走单事务 guarded fingerprint batch，模板查询结束再次复核同一 guard。target org/project/scope/name/value/ports 任一漂移都必须 fail-closed，不得用旧 workspace 指纹返回模板。
- 相关：`docs/superpowers/plans/scan-workflow-implementation.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner
```
