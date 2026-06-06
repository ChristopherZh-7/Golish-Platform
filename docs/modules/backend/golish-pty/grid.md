# golish-pty / grid

> **一句话职责**：Phase B 虚拟终端网格——包 `alacritty_terminal` 在后端维护完整网格状态机（光标/alt-screen/scrollback/SGR），按帧产出 diff 给前端 `GridTerminal.tsx` 渲染 TUI（vim/htop/less），绕开 xterm.js 的 Windows WebView2/WebGL 黑屏 bug。

- **类型**：目录模块（属于 crate [`golish-pty`](../golish-pty.md)）
- **路径**：`backend/crates/golish-pty/src/grid/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改后端网格状态机（喂 PTY 字节进 alacritty `Term`）、快照/diff wire 格式时
- TUI 应用（vim/htop）渲染异常、Windows WebView2 黑屏相关时
- 改 `GridManager` 的 per-session 终端生命周期时

## 职责

后端维护每个 PTY 会话一个 `GridTerminal`（alacritty `Term` 状态机），`write` 喂原始字节，`snapshot_full`/`snapshot_diff` 产出可序列化结构发前端。设计见 `docs/design/2026-05-15-grid-terminal-phase-b.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `GridManager`（`new` / `get_or_create` / `get` / `dispose`） | per-session `GridTerminal` 管理（活在 `Arc` 后） |
| `GridTerminal`（`write` / `snapshot_full` / `snapshot_diff` / `resize`） / `GridDims` | 单会话网格状态机 |
| `GridUpdate` / `RowUpdate` | 发前端的 wire 结构 |
| `Cell` / `CellAttrs` / `Color` / `Cursor` / `CursorStyle` | 网格单元类型 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `terminal.rs` | `GridTerminal` + `GridDims`（alacritty 状态机） |
| `snapshot.rs` | `GridUpdate`/`RowUpdate` wire 格式 |
| `cell.rs` | 单元/颜色/光标类型 |

## 依赖

- `alacritty_terminal`（`Term` 状态机）、`vte`（解析）、`parking_lot`（Mutex）

## 注意事项 / 坑

- `GridManager` 故意不实现 `Clone`——它活在 `Arc` 后与 PTY emitter 线程共享。
- wire 格式（`GridUpdate`）是与前端 `GridTerminal.tsx` 的协议，改它要同步前端。
- alt-screen/TUI 渲染要全转义序列；与 `parser/` 的 alt-screen 过滤逻辑配合。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-pty grid
```
