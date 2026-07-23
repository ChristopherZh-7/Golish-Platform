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
| `GolishError` / `IpcError` / `Result` | Tauri/CLI 边界统一错误（带 `code`，I1）；immutable scope 历史阻止组织删除时返回 `runtime_scope_history_requires_invalidation` |
| `DbState` | 命令接收的窄 DB 状态句柄 |
| `domain::operator::{OperatorId, OperatorChannel, TrustedOperatorPrincipal, TrustedOperatorPrincipalProvider}` | 不可 serde、字段私有的服务端操作员身份；特权请求不能携带 actor UUID |
| `TauriEventEmitter` | 包 `tauri::AppHandle` 的事件发射适配器 |
| `ports`（vuln/platform/pentest/agent/recon） | provider 端服务 ports（S1-2），如 `VaultReadPort`/`PgVaultAdapter`；Vault read 面只保留 project-scoped list/get-secret，旧单值 token resolver 已随授权探针移除 |
| `pty_interactive` | PTY 输出 tap + `run_pty_cmd` Tool 实现；从 spawn 起管理同一个进程，bounded yield 后返回同一 `job_id`；提供 `check_job` / `kill_job` / `wait_for_background_jobs` 控制工具 |
| `runtime`（`TauriRuntime` / `CliRuntime`） | `GolishRuntime` 适配器 |
| `scoping` / `domain` / `state` / `background_jobs` | IDOR 守卫 / domain DTO（ts-rs）/ 状态 / Codex 式受管进程；广播 completion/live output，保留 activity/full spool/typed reconciler |

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

`error.rs`（`GolishError`）、`state.rs`（`DbState`）、`domain/operator.rs`（opaque trusted operator）、`event_emitter.rs`、`scoping.rs`（IDOR 守卫）、`pty_interactive.rs`、`background_jobs.rs`（job snapshot/completion/live output broadcast）。

## 注意事项 / 坑

- **不变量 I1**：`GolishError` 带 `code` 字段，前端按 map 翻译，不靠 HTTP status。
- `ORGANIZATION_DELETE_ACTIVE_STAGE_FORK` 是组织删除 admission 的稳定错误码；payload message 保留 exact operation/stage/task status 供日志追踪，前端用 code 映射可操作提示。
- **不变量 I2**：`scoping` 是 IDOR 守卫，所有 CRUD 验所有权（含批量）。
- **不变量 I5**：`domain` DTO 用 `ts_rs::TS` 同步前端，不要手写第二份。
- foreground PTY 在 task-local tool cancellation 下必须由 manager执行显式kill，然后`wait_terminal`等待child wait与stdout/stderr drain完成，再remove job并返回typed cancelled；不能在kill signal后立刻丢弃wrapper future。`ForegroundOnly`保留tool attribution但不加入session background-close barrier，elapsed time本身绝不kill。
- `TrustedOperatorPrincipal` 不实现 `Serialize` / `Deserialize` 且字段私有；Candidate review、Cleanup waiver、Report finalize 必须从服务端 provider 解析当前 principal，绝不能接受 request 中的 `actor_id` / `decided_by` / `finalized_by`。
- **禁止向上依赖 `golish`**（会成环）；巨石 `AppState` 故意留在 `golish` crate。
- `run_pty_cmd` / `pentest_run` 从 spawn 起就是同一种受管进程。首选 `yield-time_ms`，旧 `background:true` 只映射成短 startup yield；yield结束仍存活时返回兼容的`status:"backgrounded"`和同一`job_id`，没有detach、重启或“转后台”动作。foreground-only保持当前tool future，但等待时长不构成终止条件。终止只来自operator/AI `kill_job`、session/tool cancellation、spawn/wait/output failure或自然退出。
- production attributed spawn 走 `try_spawn_for_session_and_tool`；携带 Candidate attempt context 时返回 typed `ATTACK_VERIFIER_FOREGROUND_REQUIRED`，绝不进入进程内 background job map。无 Candidate context 的 legacy spawn 保持不变。
- `background_jobs`只有direct child已wait且stdout/stderr两个pump都EOF后才标terminal；带typed reconciler的job还必须先完成业务落地才广播completion。每流最多32MiB server-owned spool提供完整parse/hash输入，512KiB tail只供UI；spool缺失/截断、pump失败或孙进程继承pipe超过1s drain都fail-closed。terminal仍是`unreconciled`，bridge完成note/UI/trace后才`mark_reconciled`。
- `cmd.spawn()` 本身失败也必须立即广播一个 `Failed` terminal completion，不能只改内存 state。`JobCompletion` / `JobOutputChunk` 的 broadcast clones 共享 processing claim，供 bridge-generation 重叠订阅 exactly-once 消费；新增 listener 不得复制 payload 后重建独立 claim。
- 正常closeout不让模型无界循环调用`wait_for_background_jobs`；`submit_stage_deliverable`只做1秒观察，不设置进程deadline。仍有job时返回elapsed、stdout/stderr累计字节与last-output age；`check_job(job_id, yield-time_ms)`也只做一次output-sensitive bounded read，quiet不是卡死证据。AI/用户结合存活、工作量和输出活动选择继续等或显式`kill_job`。`JobState`保留shared completion claim与unreconciled terminal，lag replay不重复副作用。
- 用户停止/关闭 AI session 时，调用方应通过 `background_jobs::manager().kill_running_for_session(session_id)` 杀掉该 session 归因的后台 job；`kill()` 使用持久 notify permit，避免刚 spawn 就 kill 的竞态丢通知。
- `JobCompletion` 只携带 stdout/stderr tail 给 UI/agent note，但 manager `snapshot()` 保留完整 capped stdout/stderr；结构化落库或 coverage outcome 补写应优先读 snapshot，不能只解析 completion tail。completion 也携带 launching tool context 的 `organization_id`，供后台 EAS 扫描结果按 org 入库。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core
```
