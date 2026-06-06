# golish-pty / manager

> **一句话职责**：`PtyManager`——拥有活动 PTY 会话并暴露 read/write/resize/destroy/list 公开 API，内部含 UTF-8 边界缓冲、事件 emitter（经 `GolishRuntime`）、会话创建与 stdin-wait 检测。

- **类型**：目录模块（属于 crate [`golish-pty`](../golish-pty.md)）
- **路径**：`backend/crates/golish-pty/src/manager/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 PTY 会话生命周期（创建/读/写/resize/销毁/列举）时
- 改 UTF-8 边界缓冲（多字节字符跨读切分）、reader→emitter 通道时
- 改 PTY 事件发射（`PtyEventEmitter`/`RuntimeEmitter`）或 stdin-wait 检测时

## 职责

`PtyManager` 拥有所有活动会话并暴露公开 API。内部拆：`utf8`（UTF-8 不完整缓冲 + `OutputMessage` 通道包封）、`emitter`（`PtyEventEmitter` trait + `RuntimeEmitter` 经 `GolishRuntime` 转发）、`core`（`PtyManager`/`PtySession` 生命周期 + 读写 API）、`session_create`、`stdin_wait_detector`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `PtyManager` | 会话管理主体 + 读/写/resize/销毁/列举 API |
| `PtySession` | 单 PTY 会话 |
| `CommandBlockEvent` | emitter 命令块事件 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `core.rs` | `PtyManager`/`PtySession` + 生命周期 + 读写 API |
| `utf8.rs` | UTF-8 不完整缓冲 + `OutputMessage` |
| `emitter.rs` | `PtyEventEmitter` trait + `RuntimeEmitter` |
| `session_create.rs` / `stdin_wait_detector.rs` | 会话创建 / stdin 等待检测 |

## 依赖

- `GolishRuntime`（事件发射）、`portable-pty`/平台 PTY、`tokio`

## 注意事项 / 坑

- **UTF-8 边界缓冲**：PTY 字节流可能在多字节字符中间切断，`utf8` 缓冲不完整序列；改读取路径别破坏它（否则乱码）。
- 事件经 `GolishRuntime` 抽象发射（非直接 Tauri），保持 crate 可测/可 headless。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-pty manager
```
