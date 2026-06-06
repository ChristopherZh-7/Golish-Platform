# golish-agent-runtime / test_utils

> **一句话职责**：agent 系统的测试工具——mock LLM/流式实现 + helper，用于测 agentic loop、HITL 审批流、工具路由；feature `test-utils`（或 `cfg(test)`）下编译。

- **类型**：目录模块（属于 crate [`golish-agent-runtime`](../golish-agent-runtime.md)，`test_utils.rs` + `test_utils/`）
- **路径**：`backend/crates/golish-agent-runtime/src/test_utils.rs`（+ `test_utils/`）
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 写/改 agent loop、HITL、工具路由的单测，需要 mock LLM/流式响应时
- 给下游 crate（feature `test-utils`）提供 `TestContextBuilder` 等 mock 时

## 职责

提供 mock LLM client、可编排的流式响应（`RawStreamingChoice`/工具调用）、`TestContextBuilder` 等 helper，让 loop/HITL/路由可在无真实 LLM/网络下确定性测试。仅在 `cfg(test)` 或 feature `test-utils` 下编译。

## 公开接口

| 符号 | 说明 |
|---|---|
| mock LLM / 流式响应构造器 | 可编排的假 completion/stream |
| `TestContextBuilder`（feature `test-utils`） | 下游测试上下文构建 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `test_utils.rs` | mock 实现 + helper（模块入口） |
| `test_utils/` | 子模块（按需拆分的 mock 组件） |

## 依赖

- `rig`（completion/streaming/message）、`futures`；`cfg(test)` 下用 `golish-agent-kit`/`golish-core`/`golish-llm-providers`/`golish-tools`

## 注意事项 / 坑

- **仅测试编译**：`test-utils` feature 才暴露给下游；普通 release 不付出成本（`tempfile` optional）。
- 这是测试基础设施，不是生产代码——别在生产路径引用。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime --features test-utils test_utils
```
