# golish-sidecar / state

> **一句话职责**：sidecar 状态子系统——`SidecarState` 拥有处理器运行时 + 活动会话跟踪 + Tauri 事件发射，`SidecarStatus` 给 UI 的公开快照；方法分 `lifecycle`（new/init/status/shutdown/config）与 `sessions`（start/resume/end/list/capture）。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/state/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sidecar 顶层状态（启停、活动会话切换、config getter/setter、Tauri 事件发射）时
- 改会话操作（start/resume/end/list/find/capture/上下文获取）时
- 前端拿到的 `SidecarStatus` 字段不对时

## 职责

`SidecarState` 是 sidecar 对外的运行时句柄：持有 `Processor`、跟踪当前活动会话、向前端发 Tauri 事件。`SidecarStatus` 是给 UI 的状态快照。方法分 `lifecycle`（生命周期/配置/app handle/事件）和 `sessions`（会话操作）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SidecarState` | 顶层运行时（processor + 活动会话 + 事件发射） |
| `SidecarStatus` | UI 公开快照（active_session/session_id/enabled/sessions_dir/workspace_path） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `SidecarState`/`SidecarStatus` + 内部状态 |
| `lifecycle.rs` | new/init/status/shutdown/config/app_handle/事件发射 |
| `sessions.rs` | start/resume/end/list/find/capture/上下文获取 |

## 依赖

- crate 内 `config`（`SidecarConfig`）/`processor`（`Processor`）；`tauri::AppHandle`、`std::sync::RwLock`

## 注意事项 / 坑

- 内部状态用 `RwLock`（同步锁）；别在持锁期间 `.await` 长耗操作。
- `SidecarStatus` 是 wire 快照给前端，改字段要同步前端消费。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar state
```
