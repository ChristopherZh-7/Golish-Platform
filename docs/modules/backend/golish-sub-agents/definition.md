# golish-sub-agents / definition

> **一句话职责**：sub-agent 定义/上下文/registry——`SubAgentDefinition`（自定义 system prompt + 工具限制）、`SubAgentContext`、`SubAgentRegistry`、`SubAgentResult`、`AgentSource`（BuiltIn / File）。

- **类型**：目录模块（属于 crate [`golish-sub-agents`](../golish-sub-agents.md)）
- **路径**：`backend/crates/golish-sub-agents/src/definition/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 定义结构（prompt/工具限制/来源）、registry 注册/检索、agent 间上下文传递时
- 改 `AgentSource`（内置 vs .md 文件加载）语义时

## 职责

定义 sub-agent 基础设施：`SubAgentDefinition`（专门化 agent + 工具限制）、`SubAgentRegistry`（注册/检索）、`SubAgentContext`（执行时传递的状态）、`SubAgentResult`（结果）、`AgentSource`（BuiltIn = Rust 硬编码 worker/memorist/reflector；File = 磁盘 .md）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SubAgentDefinition` | 定义（system prompt + 工具限制） |
| `SubAgentRegistry` | 注册/检索 |
| `SubAgentContext` / `SubAgentResult` | 执行上下文 / 结果 |
| `AgentSource`（BuiltIn / File） | 定义来源 |
| `MAX_AGENT_DEPTH` | 嵌套深度上限 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 全部定义/registry/context/result/source 类型 |
| `tests.rs` | 单测 |

## 依赖

- `serde`、`std::collections`；crate 内被 `executor`/`defaults` 消费

## 注意事项 / 坑

- `MAX_AGENT_DEPTH` 限嵌套递归——sub-agent 调 sub-agent 时尊重，防失控。
- BuiltIn agents（worker/memorist/reflector）是系统级；File agents 从 `.md` 加载（见 `file_loader`/`discovery`）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents definition
```
