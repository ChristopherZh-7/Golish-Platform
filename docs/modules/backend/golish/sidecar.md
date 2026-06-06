# golish / sidecar

> **一句话职责**：sidecar 薄包装——re-export `golish-sidecar` + Tauri 命令（因依赖 AppState 留主 crate）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/sidecar/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sidecar 相关 Tauri 命令（启停、会话、状态）时

## 职责

`golish-sidecar`（会话管理 + 上下文捕获）的薄包装 + Tauri 命令面。`commands` 留主 crate（AppState 依赖），`pub use golish_sidecar::*` 暴露核心类型。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | sidecar Tauri 命令 |
| re-export `golish_sidecar::*` | `SidecarState` / `SidecarStatus` 等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export + `pub mod commands` |
| `commands/` | sidecar Tauri 命令 |

## 依赖

- `golish-sidecar`、`tauri`、`state::SidecarManaged`

## 注意事项 / 坑

- 核心逻辑在 `golish-sidecar`（capture/processor/session/state）；命令留主 crate 仅因 AppState 依赖 + 启动接线在 `app::sidecar_bootstrap`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish sidecar
```
