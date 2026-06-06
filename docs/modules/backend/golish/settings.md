# golish / settings

> **一句话职责**：设置薄包装——re-export `golish-settings` + Tauri 命令（因依赖 AppState 留主 crate，避免循环依赖）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/settings/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改设置相关 Tauri 命令（读写 settings、provider 配置等）时

## 职责

`golish-settings`（核心设置逻辑）的薄包装 + Tauri 命令面。`commands` 留主 crate（避免与 AppState 循环依赖），`pub use golish_settings::*` 暴露核心类型。

## 公开接口

| 符号 | 说明 |
|---|---|
| `commands` | 设置 Tauri 命令 |
| re-export `golish_settings::*` | `SettingsManager` / schema 等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | re-export + `pub mod commands` |
| `commands/` | 设置 Tauri 命令 |

## 依赖

- `golish-settings`、`tauri`

## 注意事项 / 坑

- 核心逻辑在 `golish-settings`；命令留主 crate 仅因 AppState 依赖。改 schema 去 `golish-settings::schema`（bump `SCHEMA_VERSION` + 迁移）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish settings
```
