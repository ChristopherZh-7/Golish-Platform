# golish-pty

> **一句话职责**：PTY / 终端管理——伪终端会话生命周期、转义序列解析（OSC/CSI）、shell 集成（OSC 133）、备用屏检测、cwd 跟踪。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-pty/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改终端会话（创建/读写/resize/终止）、终端渲染网格、转义序列解析、shell 集成时
- 终端 TUI 应用显示、cwd 跟踪问题时

## 职责

管理伪终端会话并把终端输出解析成可渲染的网格。支持 OSC 133 shell 集成、备用屏幕缓冲检测（识别 TUI）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `PtyManager` / `PtySession` | 会话管理与句柄 |
| `GridManager` / `GridTerminal` / `GridUpdate` / `RowUpdate` / `GridCell` … | 终端渲染网格 |
| `TerminalParser` / `OscEvent` | 转义序列解析 |
| `PtyError` | 错误 |

## 依赖

- **内部**：`golish-core`、`golish-platform`、`golish-settings`

## 被谁依赖 / 改动影响面

`golish`、`golish-pentest-app`、`golish-agent-app`、`golish-app-core`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `manager/` | 会话生命周期管理 | [→](golish-pty/manager.md) |
| `grid/` | 终端渲染网格/单元格 | [→](golish-pty/grid.md) |
| `parser/` | OSC/CSI 转义序列解析 | [→](golish-pty/parser.md) |
| `shell/` | shell 集成 | [→](golish-pty/shell.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `error.rs` | `PtyError` |

## 注意事项 / 坑

- 网格更新（`GridUpdate`/`RowUpdate`）是跨 IPC 给前端渲染的，改结构注意 ts-rs 同步（I5）+ 前端终端组件。
- 相关：`docs/tab-completion.md`、`docs/browser-dev.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-pty
```
