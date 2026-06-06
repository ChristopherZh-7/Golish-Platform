# golish / app

> **一句话职责**：应用 bootstrap & 生命周期——从 `lib.rs::run_gui` 机械拆出的启动阶段（bootstrap / tauri_app / window_lifecycle / menu / mcp_bootstrap / sidecar_bootstrap / workspace），无新逻辑，便于维护 + CLI/headless 复用。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/app/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改应用启动（CLI 参数/telemetry/DB/嵌入式 PG/默认 agent 文件/history）、Tauri builder 装配、窗口生命周期、菜单、MCP/sidecar 启动接线时

## 职责

把原巨石 `run_gui` 拆成独立可读/可测的启动阶段：`bootstrap`（进程级 setup）、`tauri_app::configure_builder`（plugin/managed-state/lifecycle 装配）、`window_lifecycle::handle_run_event`、`menu`、`mcp_bootstrap`/`sidecar_bootstrap`、`workspace`。CLI/headless 可复用 bootstrap helper 而不拉 Tauri builder。

## 公开接口

| 符号 | 说明 |
|---|---|
| `bootstrap`（CLI 参数/rustls/dotenv/telemetry/PG/agent 文件/history） | 进程级启动 |
| `tauri_app::configure_builder` | Tauri Builder 装配 |
| `window_lifecycle::handle_run_event` | 运行事件处理 |
| `menu` / `mcp_bootstrap` / `sidecar_bootstrap` / `workspace` | 菜单 / MCP / sidecar / workspace |

## 关键文件

| 文件 | 作用 |
|---|---|
| `bootstrap.rs` | 进程级 setup helper |
| `tauri_app.rs` | builder 装配 |
| `window_lifecycle.rs` / `menu.rs` | 生命周期 / 菜单 |
| `mcp_bootstrap.rs` / `sidecar_bootstrap.rs` / `workspace.rs` | 子系统启动 |

## 依赖

- crate 内全部子系统；`tauri`（+ 插件）

## 注意事项 / 坑

- **无新逻辑**：是 run_gui 的机械拆分；改启动顺序要兼顾 GUI（run_gui）与 headless（stage_run）两条路径都复用 bootstrap。

## 测试入口

```bash
cd backend && cargo nextest run -p golish app
```
