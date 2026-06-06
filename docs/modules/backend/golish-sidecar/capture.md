# golish-sidecar / capture

> **一句话职责**：sidecar 与 agentic loop 的事件捕获桥——`CaptureContext` per-turn 状态机，关联 tool 请求↔结果并转成 `SessionEvent`，含文件路径/重命名/输出抽取、tool 读写分类、unified diff 生成、参数摘要。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/capture/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agentic loop 被动捕获（tool 请求/结果关联成 `SessionEvent`）时
- 改文件路径/重命名抽取、tool 读/写/编辑分类、write/edit 的 diff 生成时

## 职责

`CaptureContext` 是 per-turn 状态机：把 loop 里的 tool request 与后续 result 配对，抽取涉及的文件/输出，分类工具动作（read/write/edit），对写/编辑生成 unified diff，最终转发为 `SessionEvent` 给 sidecar 处理器。

## 公开接口

| 符号 | 说明 |
|---|---|
| `CaptureContext` | per-turn 捕获状态机（关联 tool 请求↔结果） |

## 关键文件

| 文件 | 作用 |
|---|---|
| `context.rs` | `CaptureContext` 状态机 |
| `extractors.rs` | 从 tool args/results 抽文件路径/重命名/输出 |
| `tool_classification.rs` | tool 名 → read/write/edit 分类 |
| `diff.rs` | write/edit 的 unified diff 生成 |
| `format.rs` | 参数摘要 / decision-type 推断 / 截断 |

## 依赖

- crate 内 `events`（`SessionEvent`）；`golish-core`（事件类型）

## 注意事项 / 坑

- 是**被动捕获**：不改 agent 行为，只观测 loop 事件——别让它对 loop 产生副作用。
- tool 分类（read/write/edit）驱动后续 diff/commit 边界，新增工具要在 `tool_classification` 登记。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar capture
```
