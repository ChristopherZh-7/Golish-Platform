# golish-app-core / runtime

> **一句话职责**：`GolishRuntime` 平台适配器——`TauriRuntime`（GUI，包 `tauri::AppHandle` 发事件）+ `CliRuntime`（headless CLI），让上层用统一 runtime 接口发事件而不绑死 Tauri。

- **类型**：目录模块（属于 crate [`golish-app-core`](../golish-app-core.md)）
- **路径**：`backend/crates/golish-app-core/src/runtime/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改事件发射 runtime（GUI 经 Tauri vs headless CLI）、`RuntimeEvent` 发射时
- headless（`golish --stage-run`）与 GUI 共用事件链时

## 职责

提供 `golish_core::runtime::GolishRuntime` 的两个实现：`TauriRuntime`（持 `AppHandle`，emit 到前端）和 `CliRuntime`（headless，渲染到 stdout）。让 agent/terminal 事件链对 GUI/CLI 透明。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TauriRuntime`（`new(app_handle)`） | GUI 事件发射 |
| `CliRuntime` | headless CLI 事件 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `TauriRuntime` / `CliRuntime` 实现 |

## 依赖

- `golish-core`（`GolishRuntime` trait + `RuntimeEvent`）、`tauri`、`atty`（CliRuntime 检测交互 stdin）

## 注意事项 / 坑

- `TauriRuntime` 解耦了 AppState（M3 改 take `pty_output_tap` 参数）；构造时别又把 AppState 塞回来。
- CliRuntime 是 headless runner（`golish --stage-run`）的事件出口；改事件链要兼顾两实现。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-app-core runtime
```
