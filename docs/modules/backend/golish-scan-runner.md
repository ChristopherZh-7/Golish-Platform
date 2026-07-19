# golish-scan-runner

> **一句话职责**：受控 Recon 扫描与 Nuclei 选模基础设施——执行 WhatWeb/feroxbuster，并为 stage-owned Nuclei adapter 只读选择安全模板；无 Tauri 依赖。

- **类型**：crate（Layer 3 领域）
- **路径**：`backend/crates/golish-scan-runner/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 WhatWeb/feroxbuster 扫描执行、指纹→Nuclei template 选择、扫描进度事件时

## 职责

WhatWeb/feroxbuster 的 guarded runner，以及供 AI stage adapter 使用的只读
fingerprint→Nuclei template selector。进度通过 `golish_core::EventEmitterHandle`
发出（前端壳提供 `TauriEventEmitter` adapter）。旧手动 Nuclei IPC/runner 已移除；
Nuclei 的执行、解析与 evidence landing 由 `golish-pentest-app` 的 stage adapter 负责。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_whatweb` / `WhatWebOptions` | WhatWeb 指纹 |
| `select_nuclei_templates_for_origin` | current-owner + exact-origin fingerprint → 安全、去重的 Nuclei template 选择（只读） |
| `NucleiTemplateSelection` / `NucleiTemplateRationale` | template id 与具体 fingerprint/PoC 选择理由 |
| `run_feroxbuster` / `FeroxScanOptions` | 目录爆破 |
| `authorize_scan_target` / `AuthorizedScanTarget` | legacy GUI scan 的 current-owner + exact-origin 启动授权快照 |
| `ScanStorage` / `ScanProgress` / `ScanResult` | 存储/进度/结果 |

## 依赖

- **内部**：`golish-core`、`golish-db`、`golish-pentest-domain`（exact Web Origin）、`golish-shell-exec`

## 被谁依赖 / 改动影响面

`golish`、`golish-app-core`、`golish-recon-app`、`golish-pentest-app`（只读 Nuclei selector）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `nuclei/` | current-owner fingerprint → 安全 Nuclei template selector | [→](golish-scan-runner/nuclei.md) |

## 关键文件

`authorization.rs`、`whatweb.rs`、`feroxbuster.rs`、`helpers.rs`、`storage.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- 无 Tauri 依赖：进度走 `EventEmitterHandle`，别直接耦合 Tauri。
- 扫描产物应能落进 evidence（与 golish-pentest 的 ledger 协作，I7）。
- `run_whatweb` / `run_feroxbuster` 只接受 `AuthorizedScanTarget`，不能重新引入裸 target id/project。调用方预授权后，runner 完成 tool lookup/参数准备，再在 guarded audit 前复核一次、每次 command spawn 紧前再复核同一个 raw witness；任一 target org/project/scope/name/value/ports 漂移必须 0 spawn。
- 输出也沿用 launch guard：WhatWeb fingerprint batch、ferox directory entry+敏感 finding、started→completed/failed scan audit 都在各自短事务先锁 target；scanner 输出 URL 必须仍是同 exact origin。非零退出、exit=0 但 stderr 有 runtime/network failure都不能变成 clean empty/success。
- caller process override 面 fail-closed：WhatWeb 拒绝 proxy/extra_args，并固定 `--follow-redirect=never --max-redirects=0`，避免已授权 origin 用 30x 让真实请求越界；ferox absolute/network-path base 不能跨 origin，自定义 wordlist 仅允许 canonical `workspace/1.txt` 或 `workspace/.golish/wordlists/**` regular file。
- `select_nuclei_templates_for_origin` 先解析 current target guard 和 exact `web_origin_id`，只读 `fingerprint_origin_observations` 关联的 fingerprint；选择前后复核同一 `TargetWriteGuard`，不允许 target-global fallback。它不 backfill、不开进程、不写 Finding。只接受本地 KB 中 `poc_type=nuclei`、`source=nuclei_template`、strict CVE id 与 `cve` tag 同时成立，且明确为 HTTP/legacy requests/SSL、不混入 code/headless/file/network/DNS/workflow 等协议的记录；template id 必须等于 CVE id，限 ASCII `[A-Za-z0-9._-]`、1..=128 bytes，并按 id 去重。fingerprint name/version、combined escaped regex、PoC、selection（与 adapter 同为 256）及 rationale 均有硬上限，超限报错而非静默截断。Nuclei 真正执行只能走 stage-owned adapter。
- 相关：`docs/superpowers/plans/scan-workflow-implementation.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-scan-runner
```
