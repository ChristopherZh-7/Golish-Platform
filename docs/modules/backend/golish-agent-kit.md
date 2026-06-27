# golish-agent-kit

> **一句话职责**：agent runtime 的**底层构件**（Layer 4a）——工具执行路由/执行器、loop detection、task orchestrator、HITL 审批、tool policy、planner、agent 侧 DB/memory tracking、llm-client 装配，以及 stage harness gate。

- **类型**：crate（Layer 4a · agent 底层）
- **路径**：`backend/crates/golish-agent-kit/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改工具执行路由（`route_tool_execution`）、具体工具执行器、tool 定义/preset、`DefaultToolProvider` 时
- 改 loop detection、task orchestrator（PentAGI 式多阶段）、HITL 审批、tool policy、planner 时
- 改 agent 侧记忆（memory_file/gatekeeper）、内存 DB tracking、`llm_client` provider 装配时
- 改 stage harness gate（`harness/gate`、`harness/graph_engine`）时

## 职责

提供 agent runtime 的下层基础设施层（从 `golish-agent-loop` 在 A2 改名）。高层流式 loop 在兄弟 crate `golish-agent-runtime`（L4b），分家是为了不让改 loop 逻辑重编这层更大的基础设施。harness gate 复用 `golish-pentest` 的 evidence ledger 类型。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `route_tool_execution` / `ToolExecutionContext` / `ToolExecutionResult` | 共享工具执行分发器 |
| `tool_executors`（memory/web/security/knowledge_base…） | 具体执行器 |
| `get_tool_definitions_*` / `ToolPreset` / `ToolSelectionConfig` | 工具 schema + preset 选择 |
| `DefaultToolProvider` | `ToolProvider` 默认实现 |
| `task_orchestrator` / `planner` / `hitl` / `loop_detection` / `tool_policy` / `system_hooks` | 编排 / 计划 / 审批 / 防循环 / 策略 / 钩子 |
| `harness`（`gate` / `graph_engine`） | stage harness gate（Phase 1c） |
| `db_traits` / `db_tracking` / `db_shim` / `memory_*` | repo/tracking 抽象 + 长期记忆；`OrgScopeUnit` / `org_subtree_units` 给 stage fan-out 提供 root subtree 权威组织集合 |
| `SharedComponentsConfig` / `ExecutionMode` / `AgentMode`（re-export） | llm-client 配置 / 执行模式 |

## 依赖

- **内部**：`golish-core`、`golish-settings`、`golish-tools`、`golish-events`、`golish-context`、`golish-indexer`、`golish-llm-providers`、`golish-sub-agents`、`golish-prompts`、`golish-pentest`、`golish-json-repair`
- **外部**：`rig-core`、`rig-anthropic-vertex`、`rig-openai-responses`、`tokenx-rs`

## 被谁依赖 / 改动影响面

`golish-agent-runtime`、`golish-agent-bridge`、`golish-agent-app`、`golish`。是 agent 栈的地基，改公开 trait/类型影响整条 agent 链路。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `task_orchestrator/` | 多阶段任务编排（harness 驱动） | [→](golish-agent-kit/task_orchestrator.md) |
| `tool_execution/` | 工具路由分发器 | [→](golish-agent-kit/tool_execution.md) |
| `tool_executors/` | 具体工具执行器 | [→](golish-agent-kit/tool_executors.md) |
| `llm_client/` | per-provider 组件构建 | [→](golish-agent-kit/llm_client.md) |
| `harness/` | stage harness gate（Phase 1c） | [→](golish-agent-kit/harness.md) |
| `planner/` | 计划管理（PlanManager） | [→](golish-agent-kit/planner.md) |
| `hitl/` | HITL 审批记录/学习 | [→](golish-agent-kit/hitl.md) |
| `loop_detection/` | 循环检测保护 | [→](golish-agent-kit/loop_detection.md) |
| `system_hooks/` | 可扩展 hook 系统 | [→](golish-agent-kit/system_hooks.md) |
| `tool_policy/` | 工具策略访问控制 | [→](golish-agent-kit/tool_policy.md) |
| `tool_definitions/` | 工具定义/选择/sanitize | [→](golish-agent-kit/tool_definitions.md) |
| `db_traits/` | DB trait 抽象（解耦 sqlx） | [→](golish-agent-kit/db_traits.md) |
| `db_tracking/` | 后台 DB 跟踪（fire-and-forget） | [→](golish-agent-kit/db_tracking.md) |

## 关键文件

`db_shim.rs`、`execution_mode.rs`、`memory_file.rs`、`memory_gatekeeper.rs`、`sidecar_trait.rs`、`tool_provider_impl.rs`。

## 注意事项 / 坑

- **不变量 I7**：harness gate 依赖 evidence ledger（类型来自 `golish-pentest`），阶段产物必须有证据。
- 与 `golish-agent-runtime` 是**有意分家**（A2）：底层基础设施在此，流式 loop 在那；改这层会触发下游重编。
- `db_traits::DbRepoProvider::org_subtree_units` 是 root-bound `stage_run` 的 scope truth 入口；默认实现只能给测试 double 兜底，生产实现必须返回 DB root+descendants 的 id/name/parent_id，避免续跑时靠模型重建 org 列表漏资产。
- crate 级 `#![allow(too_many_arguments / needless_borrow / manual_async_fn)]` 是有意保留（宽 context 透传 / object-safe trait）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit
```
