# golish-sidecar / processor

> **一句话职责**：sidecar 事件处理器——异步处理 `SessionEvent`，更新 `state.md`（会话上下文）+ `patches/staged/`（L2 提交补丁），含 LLM 合成（state/标题/commit 消息）、git status/diff 助手、去重。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/processor/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sidecar 事件异步处理（`ProcessorTask` 路由、`state.md` 更新、补丁生成）时
- 改 LLM 合成（state.md / 标题 / commit 消息）或 git status/diff 助手时
- 改 per-session 去重（`DUPLICATE_WINDOW_SECS`）时

## 职责

接收 `ProcessorTask`，路由 `SessionEvent` 到文件/日志/state 更新：维护 per-session 内存状态 + 去重，更新 `state.md`，按 commit 边界生成 staged 补丁，并经 `golish-synthesis` 做 LLM 合成。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ProcessorTask` | 发给处理器的任务枚举 |
| （`Processor` 由 `state/` 持有并驱动） | 异步处理循环 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `event_handler.rs` | `handle_event` / `handle_end_session` 路由 |
| `session_state.rs` | per-session 内存状态 + 去重 + 文件跟踪 |
| `synthesis.rs` | `state.md` / 标题 / commit 消息 LLM 合成 |
| `git.rs` | git status/diff 助手 |
| `patches.rs` | staged 补丁生成编排（`generate_patch`） |

## 依赖

- crate 内 `events`/`commits`；`golish-synthesis`（`SynthesisConfig`）、`tauri::AppHandle`、`tokio::mpsc`

## 注意事项 / 坑

- 异步管道（`mpsc`）：处理器在后台任务跑，别在事件路径做阻塞/长耗同步调用。
- 去重窗口 `DUPLICATE_WINDOW_SECS` 防重复事件刷 state；改它注意误删合法事件。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar processor
```
