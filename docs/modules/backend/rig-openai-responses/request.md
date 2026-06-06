# rig-openai-responses / request

> **一句话职责**：OpenAI Responses API 的请求构建——纯转换（无 HTTP/无流式）：把 rig `CompletionRequest` + 模型配置变成 `CreateResponse`；含聊天历史转换、工具映射、reasoning/temperature 配置、无状态多轮（`encrypted_content` + `store:false`）。

- **类型**：目录模块（属于 crate [`rig-openai-responses`](../rig-openai-responses.md)）
- **路径**：`backend/crates/rig-openai-responses/src/request/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 rig→Responses API 的请求构建（历史/工具/reasoning/temperature/无状态多轮）时
- 改 `additional_params["reasoning"]` 的后期 override 时

## 职责

纯同步数据转换（大多单测在此）：`builder::build_request` 编排历史转换 + 工具映射 + reasoning/temperature + 无状态多轮（`encrypted_content`/`store:false`）；`conversion` per-message 转 `InputItem`；`reasoning::apply_additional_params_reasoning` 按调用 late-override effort/summary。

## 公开接口

| 符号 | 说明 |
|---|---|
| `build_request`（builder） | 顶层请求构建入口 |
| `convert_user_content` / `convert_assistant_content_to_items` / `convert_tool_definition`（conversion） | 消息→InputItem |
| `apply_additional_params_reasoning`（reasoning） | reasoning 后期 override |

## 关键文件

| 文件 | 作用 |
|---|---|
| `builder.rs` | `build_request` 编排 |
| `conversion.rs` / `reasoning.rs` | 消息转换 / reasoning override |

## 依赖

- `async-openai`（`CreateResponse`/`InputItem`）、`rig`、`golish-json-repair`

## 注意事项 / 坑

- **纯转换无 HTTP/无流式**：单测多在此（确定性）；别在此发请求。
- 无状态多轮用 `encrypted_content` + `store:false`（OpenAI 不存上下文）；reasoning effort 可 per-call override。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-openai-responses request
```
