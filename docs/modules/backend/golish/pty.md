# golish / pty

> **一句话职责**：PTY 薄包装——`pub use golish_pty::*` 兼容性 re-export（基础设施在 `golish-pty` crate）。

- **类型**：目录模块（属于 crate [`golish`](../golish.md)）
- **路径**：`backend/crates/golish/src/pty/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 排查 `crate::pty::*` 路径来源时（实际类型在 `golish-pty`）

## 职责

对 `golish-pty` 基础设施 crate 的薄兼容性包装，仅 `pub use golish_pty::*`，让主 crate 内 `crate::pty::*` 路径继续可用。PTY 实际实现见 [`golish-pty`](../golish-pty.md)（manager/grid/parser/shell）。

## 公开接口

| 符号 | 说明 |
|---|---|
| re-export `golish_pty::*` | `PtyManager` / 终端类型等 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `pub use golish_pty::*` |

## 依赖

- `golish-pty`

## 注意事项 / 坑

- 纯 re-export：改 PTY 行为去 `golish-pty`，别在此加逻辑。
- PTY 的 Tauri 命令在 `commands::proc`（git_pty facade）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish pty
```
