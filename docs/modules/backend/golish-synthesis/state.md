# golish-synthesis / state

> **一句话职责**：`state.md` 合成——`StateSynthesizer` trait + `create_state_synthesizer` 工厂，按 `SynthesisBackend` 选后端：template（规则、无 LLM）/ openai / grok / vertex（Claude on Vertex）。

- **类型**：目录模块（属于 crate [`golish-synthesis`](../golish-synthesis.md)）
- **路径**：`backend/crates/golish-synthesis/src/state/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `state.md` 合成的输入/输出类型、`StateSynthesizer` trait 或后端选择工厂时
- 加新合成后端（除 template/openai/grok/vertex 外）时

## 职责

定义 `state.md` 合成的 `StateSynthesisInput`/输出类型、`StateSynthesizer` trait、`create_state_synthesizer` 工厂；具体后端在兄弟文件：template（无 LLM 规则默认）、openai（OpenAI 兼容）、grok（xAI）、vertex（Anthropic Claude on Vertex）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `StateSynthesizer`（trait）/ `create_state_synthesizer` | 合成接口 + 后端工厂 |
| `StateSynthesisInput` | 输入（current_state / event_type / event_details / files） |
| `TemplateStateSynthesizer` / `OpenAiStateSynthesizer` / `GrokStateSynthesizer` / `VertexAnthropicStateSynthesizer` | 4 个后端实现 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 输入/输出类型 + trait + 工厂 |
| `template.rs` | 规则默认（无 LLM） |
| `openai.rs` / `grok.rs` / `vertex.rs` | LLM 后端 |

## 依赖

- crate 内 `config`（`SynthesisBackend`/`SynthesisConfig`）、`prompts`（`STATE_UPDATE_USER_PROMPT`）；`anyhow`

## 注意事项 / 坑

- template 后端**不需要 LLM**，是无网/降级路径；后端选择失败应能回退 template。
- 与 sidecar `processor::synthesis` 配合（sidecar 调用本 crate 合成 state.md）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-synthesis state
```
