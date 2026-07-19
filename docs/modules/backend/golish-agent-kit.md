# golish-agent-kit

> **一句话职责**：agent runtime 的**底层构件**（Layer 4a）——工具路由、orchestrator、HITL/tool policy、DB tracking、llm-client、stage gate，以及 prompt-safe ContextPack data renderer。

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
| `harness`（`gate` / `graph_engine` / `handoff_catalog` / `knowledge_context`） | stage harness gate + canonical handoff keys + prompt-safe ContextPack renderer；Candidate classifier 将冻结 observation/hash 映射为 exact replay capability；`StageSpec.runtime_memory` 声明 Unit owner/final-seal，`StageSpec.team_scheduler` 以 closed policy声明 V2 sibling roles、K、dynamic request 与 risk lane；Vuln final seal 用一个可重算的 `TechniqueOutcomeSet` 证明完整 outcome 集合，独立 Finding 仍单独引用 |
| `db_traits` / `db_tracking` / `db_shim` / `memory_*` | repo/tracking 抽象 + 长期记忆；runtime-aware tool start 固定 exact operation task owner；org-bound evidence append显式携带 trusted organization witness；RuntimeMemory trait暴露 Stage Team plan/claim/output/barrier/repair/operator recovery、Candidate TerminalIntent/barrier/recovery，以及 immutable `StageForkCreate` exact-lineage contract |

Company Controller StageSpec 的 C/G/K 是真实并发权威；历史 `max_dynamic_requests` 只保留在冻结序列化形状中支持旧 TeamPlan exact replay，不再代表 child lifetime admission。完成权威仍是 DB worklist/evidence/Gate，不是 child 数量。
| `SharedComponentsConfig` / `ExecutionMode` / `AgentMode`（re-export） | llm-client 配置 / 执行模式 |
| `SessionCaptureBackend` | per-bridge sidecar lifecycle/capture；restore 支持 end/find/resume/start，禁止回退到 app-global sidecar |
| `DbFlowCheckpointer` / `TaskOrchestrator::run_from_stage` | whole-record graph adapter：trusted resume或stage fork可从指定 stage进入同一 graph；V2 source save no-op、load 只用 relational cursor，legacy source 才读写 `graph_flow` |

## 依赖

- **内部**：`golish-core`、`golish-settings`、`golish-tools`、`golish-events`、`golish-context`、`golish-indexer`、`golish-llm-providers`、`golish-sub-agents`、`golish-prompts`、`golish-pentest`、`golish-memory-domain`、`golish-memory-app`、`golish-json-repair`
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
- ContextPack renderer 只能消费已授权 pack 并输出 escaped data；不得把检索内容变成 ToolDefinition/ToolChoice/authz，也不得解引用 VaultRef。
- 与 `golish-agent-runtime` 是**有意分家**（A2）：底层基础设施在此，流式 loop 在那；改这层会触发下游重编。
- `db_traits::DbRepoProvider::org_subtree_units` 是 root-bound `stage_run` 的 scope truth 入口；默认实现只能给测试 double 兜底，生产实现必须返回 DB root+descendants 的 id/name/parent_id，避免续跑时靠模型重建 org 列表漏资产。
- `DbRepoProvider::scoping_passive_recon_organization_authorized` 是 Scoping pre-freeze 子公司 evidence 的窄授权 seam：生产实现必须重验 exact operation/stage/root/latest human choice，默认实现恒拒绝。runtime 不能把模型传入的 organization UUID 直接当作 evidence owner，也不能把该 seam 扩展给其他 recon action 或主动阶段。
- crate 级 `#![allow(too_many_arguments / needless_borrow / manual_async_fn)]` 是有意保留（宽 context 透传 / object-safe trait）。
- `SessionCaptureBackend` 是 bridge-owned session truth；新增实现必须实现 legacy match + resume，full restore 不能绕过 trait 操作另一个全局实例。
- `V2Only` 与已整源选择 V2 的 `DualWriteV2Preferred` 都以 relational runtime 为恢复源。metalcraft graph 起点只能从 persisted `current_stage` 构造为空默认状态，不能读取、修复或回写 legacy `state_blob`；preferred legacy fallback 必须显式标成 `LegacyFallback`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit
```
