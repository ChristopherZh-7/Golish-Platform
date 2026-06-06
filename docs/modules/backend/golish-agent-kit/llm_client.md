# golish-agent-kit / llm_client

> **一句话职责**：agent 系统的 LLM client 抽象——re-export `golish-llm-providers` 类型 + 提供 per-provider `create_*_components` 构建器（供 `agent_bridge::constructors`）+ `LlmClientFactory`（缓存 sub-agent 模型 override）。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/llm_client/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 per-provider 组件构建（`create_*_components`）、`AgentBridgeComponents` 装配时
- 改 sub-agent 模型 override 缓存（`LlmClientFactory`）或 `SharedComponentsConfig` 时

## 职责

把 `golish-llm-providers` 的 client 创建包装成 agent bridge 所需的组件束。`providers` 每 provider 一个 `create_*_components` builder（flatten 到 `crate::llm_client::*`）；`factory` 缓存 sub-agent 模型 override。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SharedComponentsConfig` | 共享初始化输入配置 |
| `AgentBridgeComponents` | 每个 provider builder 的公共返回类型 |
| `LlmClientFactory` | 缓存 sub-agent 模型 override |
| `create_*_components`（openai/anthropic/ollama/…） | per-provider 组件构建器 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 共享 helper + `SharedComponentsConfig`/`AgentBridgeComponents` |
| `providers/` | 每 provider 一个 builder（flatten） |
| `factory.rs` | `LlmClientFactory` |

## 依赖

- crate 内 `hitl::ApprovalRecorder`；`golish-llm-providers`、`golish-tools`（`ToolRegistry`）、4 个 rig fork

## 注意事项 / 坑

- builder flatten 到 `crate::llm_client::*` 保持调用方路径稳定；加 provider 沿用。
- 能力检测应走 `golish-models`/`golish-llm-providers`（注册表），别字符串匹配。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit llm_client
```
