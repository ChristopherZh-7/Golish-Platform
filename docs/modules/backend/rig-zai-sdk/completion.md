# rig-zai-sdk / completion

> **一句话职责**：Z.AI API 的 `CompletionModel`——`mod.rs` 持 struct + `StreamingResponseData`/`StreamingUsage`；`conversion`（rig↔Z.AI 消息/工具纯转换）+ `runtime`（`completion()`/`stream()` 打 `/chat/completions`）。

- **类型**：目录模块（属于 crate [`rig-zai-sdk`](../rig-zai-sdk.md)）
- **路径**：`backend/crates/rig-zai-sdk/src/completion/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Z.AI `CompletionModel`（`/chat/completions` 调用、SSE 流 + tool call 累积）、rig↔Z.AI 转换时

## 职责

实现 Z.AI 的 `CompletionModel`。`mod.rs` 公开 struct + 流式最终 payload 类型（`StreamingResponseData`/`StreamingUsage` + `GetTokenUsage`）；`conversion` 纯 rig↔Z.AI 消息/工具转换；`runtime` 实现 rig trait 的 `completion()`/`stream()`（打 `/chat/completions`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `CompletionModel` | Z.AI completion 模型 |
| `StreamingResponseData` / `StreamingUsage` | 流式最终 payload + 用量 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `CompletionModel` + 流式 payload 类型 |
| `conversion.rs` | rig↔Z.AI 纯转换 |
| `runtime.rs` | `completion()`/`stream()`（HTTP/SSE） |

## 依赖

- crate 内 `client::Client`、`text_tool_parser`（伪 XML tool call）；`rig::completion`

## 注意事项 / 坑

- 部分 GLM 模型经**伪 XML tool call**（见 `text_tool_parser`）；runtime 流式需累积 tool call。
- `conversion` 纯转换，`runtime` 才碰 HTTP；thinking 始终开启（见 crate 卡）。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-zai-sdk completion
```
