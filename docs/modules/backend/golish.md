# golish

> **一句话职责**：**组合根 + Tauri 桌面应用**（apex binary）——bootstrap Tauri/CLI 运行时、装配全局 `AppState`、通过 `commands_facade` + `commands_registry.rs` 把 ~300 个 `#[tauri::command]` 接到前端，同时提供 headless CLI 模式。

- **类型**：crate（Layer 6 · 组合根 / 应用入口）
- **路径**：`backend/crates/golish/`（`bin golish` + `lib golish_lib`）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加新 Tauri command（按 `docs/development.md` 五步走：函数→facade `pub use`→registry→前端 wrapper→ts-rs）时
- 改应用 bootstrap（CLI 参数、telemetry、嵌入式 PG、默认 agent 文件、history）、Tauri builder 装配、窗口生命周期时
- 改全局 `AppState`、CLI/REPL headless 模式、stage_run、MCP/sidecar 装配时

## 职责

整个仓库的组合根：依赖所有 per-domain app crate（vuln/recon/pentest/agent/platform）+ 基础设施 crate + agent 栈，把它们 wire 成一个 Tauri 应用。`run_gui()` 是 GUI 入口，`main.rs` 同时支持 CLI。巨石 `AppState` 故意留在本 crate（聚合 AI/indexer/settings/sidecar 等 golish 内部子系统）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_gui()` | Tauri GUI 入口（bootstrap → configure_builder → install_handlers → run） |
| `install_handlers`（来自 `commands_registry.rs`，`include!`-d） | `tauri::generate_handler![...]` ~300 命令 |
| `commands_facade::<domain>`（ai/pentest/vuln_intel/vault/workspace…18 个） | 各域**权威命令面** re-export（glob `__cmd__$name`） |
| `app`（`bootstrap` / `tauri_app` / `window_lifecycle` / `menu` / `mcp_bootstrap` / `sidecar_bootstrap`） | 进程级 setup + DB-ready Memory Supervisor + builder/退出生命周期 |
| `cli`（`args` / `bootstrap` / `repl` / `runner`） | headless CLI / REPL 模式（clap） |
| `state`（`db` / `pty` / `mcp` / `sidecar` / `telemetry`） | 全局 `AppState` 各子状态 |
| `tools` / `pentest_tool_factory` / `stage_run` / `runtime` | 工具装配 / pentest 工具工厂 / stage 运行 / runtime 适配 |
| `reporting_artifact_store` | 把 Reporting artifact port 绑定到 server-resolved project root，并拥有 GUI/CLI 共享 orphan GC runtime |

## 依赖

- **内部**：**几乎所有 crate**——5 个 `*-app`（vuln/recon/pentest/agent/platform）+ `golish-app-core` + agent 栈（kit/runtime/bridge/sub-agents）+ 基础设施（db/graphiti/indexer/pty/sidecar/context/session/settings/models/tools/skills/mcp/llm-providers/synthesis/prompts/events/projects/scan-runner/pentest/vuln-intel/intel-providers/integrations/js-analyzer/auth-probe/artifacts/cli-output/core/platform）+ `vtcode-indexer`
- **外部**：`tauri`（+ dialog/shell/notification 插件）、`clap`、`rig-core`、`graph-flow`、`opentelemetry*`（Langfuse）、`sqlx`、`ts-rs`、`rustls`、`notify`、`nucleo-matcher`

## 被谁依赖 / 改动影响面

**无**（apex binary，DAG 顶点，无人依赖它）。但它依赖一切，故任何下游 crate 的破坏性改动最终汇聚到这里编译失败。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `app/` | bootstrap + Tauri builder + 生命周期 | [→](golish/app.md) |
| `commands/` | golish-staying 命令（fs/proc/project/ui） | [→](golish/commands.md) |
| `commands_facade/` | 18 个域的权威命令面 re-export | [→](golish/commands_facade.md) |
| `cli/` | headless CLI / REPL | [→](golish/cli.md) |
| `stage_run/` | headless 单/区间阶段实跑器 | [→](golish/stage_run.md) |
| `state/` | 全局 `AppState` + 窄子状态 | [→](golish/state.md) |
| `telemetry/` | OpenTelemetry / Langfuse | [→](golish/telemetry.md) |
| `history/` | 命令/终端历史 | [→](golish/history.md) |
| `mcp/` | MCP Tauri 命令 | [→](golish/mcp.md) |
| `models/` | 模型注册表命令 | [→](golish/models.md) |
| `settings/` | 设置薄包装 + 命令 | [→](golish/settings.md) |
| `sidecar/` | sidecar 薄包装 + 命令 | [→](golish/sidecar.md) |
| `indexer/` | 索引薄包装 + 命令 | [→](golish/indexer.md) |
| `projects/` | 项目配置命令薄包装 | [→](golish/projects.md) |
| `pty/` | PTY 薄包装（re-export） | [→](golish/pty.md) |
| `tools/` | 工具薄包装（re-export） | [→](golish/tools.md) |
| `db/` | DB adapter 占位（当前空） | [→](golish/db.md) |

## 关键文件

`main.rs`（CLI/GUI 分派）、`lib.rs`（`run_gui` + `include!("commands_registry.rs")`）、`commands_registry.rs`（giant `generate_handler!`）、`pentest_tool_factory.rs`、`runtime.rs`、`window_state.rs`、`compat.rs`、`ai.rs`（re-export 到 `golish-agent-app` 的薄 shim）。

## 注意事项 / 坑

- **不变量 I4 / AGENTS.md §2.2**：命令命名 `<domain>_<verb>_<object>`；**禁止**直接在 `commands_registry.rs` 加 `use crate::foo::commands::*;` glob，必须走 `commands_facade/<domain>.rs`。加/改命令只动两个文件：命令 home 模块 + 对应 facade 文件，且 registry 块保持字母序。
- **不变量 I5**：跨 IPC 类型用 ts-rs 同步，别手写两份。
- `commands_registry.rs` 是 `include!` 进 `lib.rs` 的，因为 `#[tauri::command]` 的 `__cmd__$name` 宏 `#[macro_export]` 到 crate 根。
- `AppState` 故意留在本 crate（聚合内部子系统）；app crate 取窄 `DbState`。
- Memory Supervisor 只由 GUI/CLI composition root 启停；AppState 持有 cancel/join owner，`AgentState`/`AgentBridge` 只拿同 adapter 的 UoW Arc。
- Reporting artifact store factory 只在 composition root 解析 canonical project root；IPC/model 不得传路径或 content key。`ProjectReportArtifactStore::promote` 把底层 `ReservedReportArtifact` 包成 `ArtifactPublicationReservation`，让 `ReportFinalizer` 持 per-content lock 到 DB attach 提交。GUI/CLI 都在 DB ready 后启动同一 GC 语义，退出时必须在 pool 前 shutdown。Orphan GC 必须先按 `canonical_project_path` 分组，union 同一路径全部 active/retired `project_scope_id` 的 DB referenced content keys 后只扫物理目录一次，不能让 sibling scope 的局部 reference set 互删 retained blob；底层 file-storage 在 Unix 对 symlink/binding swap、在 Windows 对 symlink/reparse/junction/handle replacement 统一 fail closed，并在同一 content lock 后重查 grace。

## 测试入口

```bash
cd backend && cargo nextest run -p golish
# 或全套：just precommit
```
