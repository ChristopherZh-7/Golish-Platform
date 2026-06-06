# rig-gemini-vertex / completion

> **一句话职责**：Gemini on Vertex AI 的 `CompletionModel`——`mod.rs` 持 struct + 构造/builder + `StreamingCompletionResponseData`；`convert`（rig↔Gemini 类型纯转换）+ `model_impl`（`rig::completion::CompletionModel` trait：HTTP + SSE→rig 流翻译）。

- **类型**：目录模块（属于 crate [`rig-gemini-vertex`](../rig-gemini-vertex.md)）
- **路径**：`backend/crates/rig-gemini-vertex/src/completion/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Gemini `CompletionModel`（HTTP 调用、SSE→rig 流翻译、ThinkingConfig）、rig↔Gemini 类型转换时

## 职责

实现 Gemini Vertex 的 `CompletionModel`。`mod.rs` 公开 struct + 构造（含 `ThinkingConfig`）；`convert` 纯类型转换（rig ↔ Gemini）；`model_impl` 实现 rig trait（HTTP + SSE→rig stream）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `CompletionModel`（构造 + builder） | Gemini completion 模型 |
| `StreamingCompletionResponseData` | 流式响应数据 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `CompletionModel` struct + 构造 |
| `convert.rs` | rig↔Gemini 纯转换 |
| `model_impl.rs` | rig `CompletionModel` trait impl（HTTP/SSE） |

## 依赖

- crate 内 `client::Client`、`types`（`ThinkingConfig`）；`rig::completion`、`async-stream`

## 注意事项 / 坑

- `convert` 是纯转换（无 IO），`model_impl` 才碰 HTTP——别混。
- 升级 `rig-core` 对齐 `CompletionModel` trait 签名（fork 维护点）。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-gemini-vertex completion
```
