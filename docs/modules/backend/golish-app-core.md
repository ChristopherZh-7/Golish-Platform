# golish-app-core

> **一句话职责**：应用边界**共享类型层**（L5）——统一错误 `GolishError`、窄状态句柄 `DbState`、`TauriEventEmitter`、provider 端服务 ports、`pty_interactive`、`GolishRuntime` 适配器、IDOR scoping 守卫。让每个 per-domain app crate 不依赖巨石 `golish` 就能写 Tauri command。

- **类型**：crate（Layer 5 · 应用共享边界）
- **路径**：`backend/crates/golish-app-core/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何 `*-app` crate 的 Tauri command，需要 `GolishError` / `DbState` / `TauriEventEmitter` 时
- 改统一错误码契约（I1）、IDOR scoping 守卫（I2）、跨服务 ports（VaultReadPort 等）时
- 改 `run_pty_cmd` 工具 / PTY 输出 tap（`pty_interactive`）或 `TauriRuntime`/`CliRuntime` 时

## 职责

持有所有 per-domain app crate 共需的边界类型，使其能定义 `#[tauri::command]` 而不依赖巨石 `golish`。位于 L5：在被它聚合错误的 domain 服务（L2/L3）之上、在 per-domain app crate 与 `golish` binary 之下。**不得依赖 `golish`**（否则成环）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `GolishError` / `IpcError` / `Result` | Tauri/CLI 边界统一错误（带 `code`，I1） |
| `DbState` | 命令接收的窄 DB 状态句柄 |
| `TauriEventEmitter` | 包 `tauri::AppHandle` 的事件发射适配器 |
| `ports`（vuln/platform/pentest/agent/recon） | provider 端服务 ports（S1-2），如 `VaultReadPort`/`PgVaultAdapter` |
| `pty_interactive` | PTY 输出 tap + `run_pty_cmd` Tool 实现；启动后台 job 时捕获当前 agent tool context；提供 `check_job` / `kill_job` / `wait_for_background_jobs` 后台控制工具 |
| `runtime`（`TauriRuntime` / `CliRuntime`） | `GolishRuntime` 适配器 |
| `scoping` / `domain` / `state` / `background_jobs` | IDOR 守卫 / domain DTO（ts-rs）/ 状态 / 后台任务；后台任务广播 completion + live stdout/stderr chunks |

## 依赖

- **内部**：`golish-db`、`golish-pty`、`golish-tools`、`golish-skills`、`golish-pentest`、`golish-vuln-intel`、`golish-scan-runner`、`golish-core`
- **外部**：`tauri`、`ts-rs`、`sqlx`、`reqwest`、`atty`

## 被谁依赖 / 改动影响面

`golish-vuln-app`、`golish-recon-app`、`golish-pentest-app`、`golish-agent-app`、`golish-platform-app`、`golish`。**所有 app crate 的公共地基**，改 `GolishError`/`DbState` 影响面极大。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `ports/` | 跨服务端口 trait + adapter（S1-2） | [→](golish-app-core/ports.md) |
| `runtime/` | `TauriRuntime` / `CliRuntime` 适配器 | [→](golish-app-core/runtime.md) |
| `domain/` | 共享 domain DTO（ts-rs 导出） | [→](golish-app-core/domain.md) |

## 关键文件

`error.rs`（`GolishError`）、`state.rs`（`DbState`）、`event_emitter.rs`、`scoping.rs`（IDOR 守卫）、`pty_interactive.rs`、`background_jobs.rs`（job snapshot/completion/live output broadcast）。

## 注意事项 / 坑

- **不变量 I1**：`GolishError` 带 `code` 字段，前端按 map 翻译，不靠 HTTP status。
- **不变量 I2**：`scoping` 是 IDOR 守卫，所有 CRUD 验所有权（含批量）。
- **不变量 I5**：`domain` DTO 用 `ts_rs::TS` 同步前端，不要手写第二份。
- **禁止向上依赖 `golish`**（会成环）；巨石 `AppState` 故意留在 `golish` crate。
- `run_pty_cmd` / `pentest_run` 的 `background:true` 不是零等待返回：`pty_interactive` 会先做一个短启动确认窗口，窗口内的参数/运行时错误必须同步返回给 agent；只有确认仍在运行后才返回 `status:"backgrounded"`。
- `wait_for_background_jobs` 是显式等待工具：只等当前 AI session 归因的后台 job，并返回完成 job 的 stdout/stderr tail + exit code，让模型在 `submit_stage_deliverable` 前能先读结果。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core
```
