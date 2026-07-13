# golish-core

> **一句话职责**：全仓库最底层的基础类型与 trait（L1 Foundation）——`Tool` trait、会话、事件、计划、HITL、prompt 贡献者等跨所有 crate 的根类型都在这。

- **类型**：crate（Layer 1 基础层）
- **路径**：`backend/crates/golish-core/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 用到 `Tool` trait、会话类型、事件类型、`TaskPlan`/计划、HITL 审批、prompt 贡献机制、`utils` 时
- 几乎任何后端改动的根类型都从这里来——改这里前务必评估影响面

## 职责

提供跨所有 golish crate 的基础类型与 trait。位于依赖树最底层，唯一内部依赖是同层的 `golish-platform`，其余只依赖外部库。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Tool` | 所有 agent 工具实现的 trait（被 golish-tools 等实现） |
| `AgentMode` / `SessionManager` / `SessionManagerFactory` | 会话与模式 |
| `GolishRuntime` / `RuntimeEvent` / `ApprovalResult` | 运行时抽象 |
| `TaskPlan` / `PlanStep` / `StepStatus` | 计划系统类型 |
| `AttackExecutionContract` | operation-frozen Candidate 执行 rollout 枚举；稳定值为 `legacy` / `dual_write_read_legacy` / `dual_write_read_v2_fallback` / `v2_only` |
| `CandidateAttemptContextRef` / `check_candidate_tool_boundary` | opaque verifier identity 与 dependency-floor closed-tool/foreground guard；不携带 plan/action/budget/scope |
| HITL：`ApprovalDecision` / `ApprovalPattern` / `RiskLevel` / `ToolApprovalConfig` | 人类在环审批 |
| `PromptContributor` / `PromptContext` / `PromptSection` | prompt 组装贡献机制 |
| `EventEmitter` / `NullEmitter`、`DbReadyGate`、`SkillProvider` | 事件/就绪门/技能 |
| `with_agent_session` / `with_agent_tool_context` / `with_agent_tool_output_sender` / `AgentToolContext` | agent loop 的 task-local session/tool/output attribution；tool context 携带可信 operation/org 绑定 |
| `emit_current_agent_tool_output_chunk` | bridge/direct tools 的 best-effort live output side-channel，发 `AiEvent::ToolOutputChunk` 给当前可见 tool card |
| `web_fetch`、`vault`、`utils`、`time::now_ms` | 通用能力 |

## 依赖

- **内部**：`golish-platform`

## 被谁依赖 / 改动影响面

**几乎所有上层 crate（28+）**：db / pty / settings 间接、tools / events / session / prompts / sub-agents / agent-kit / agent-runtime / agent-bridge / 各 *-app / golish 等。**这是影响面最大的 crate**：改任何 `pub` 类型前先 grep 全量引用。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `events/` | 事件类型集合（`AiEvent` wire 契约） | [→](golish-core/events.md) |
| `session/` | 会话归档/快照/列表 | [→](golish-core/session.md) |
| `tool_name/` | 工具名与分类枚举 | [→](golish-core/tool_name.md) |

## 关键文件（单文件模块）

`agent_mode.rs`、`agent_session.rs`、`attack_execution.rs`、`hitl.rs`、`plan.rs`、`prompt.rs`、`runtime.rs`、`session_manager.rs`、`tool.rs`、`tool_args.rs`、`textual_tool_call.rs`、`ready_gate.rs`、`skill_provider.rs`、`pentest_context.rs`、`event_emitter.rs`、`vault.rs`、`web_fetch.rs`、`paths.rs`、`os.rs`、`jsonl.rs`、`time.rs`、`utils.rs`、`message.rs`、`api_request_stats.rs`、`session_kind.rs`。

## 注意事项 / 坑

- 跨 IPC 的类型若在此定义，必须 `#[derive(ts_rs::TS)]` 同步前端（不变量 I5）。
- `AttackExecutionContract` 在这里仅定义稳定纯类型与 rollout 语义；deployment default、operation row 冻结、DB constraint/immutable trigger 属于 Candidate V2 后续 schema/repo task，不能用环境变量在 operation 中途覆盖。
- `agent_session.rs` 的 task-local attribution 是 best-effort：只对 inline awaited work 生效，启动后台 job 时要立即 capture，不能等到 spawned task 里再读。`AgentToolContext.operation_id` 来自 runtime 的 active harness operation/stage attempt，不能从模型参数猜；`organization_id` 承载当前 harness org。后台 completion 用这些可信绑定把结构化扫描结果、证据和 coverage outcome 写回正确 run/org。
- direct/bridge 工具如果要让前端实时看到“工具现在在看什么”，用 `emit_current_agent_tool_output_chunk` 发 chunk；主 loop / sub-agent executor 会注入 `with_agent_tool_output_sender`。如果工具自己 `tokio::spawn` 读子进程 stderr/stdout，必须先在 inline scope capture `current_agent_tool_context()` 和 `current_agent_tool_output_sender()`，spawn 里不能再读 task-local。
- 改动牵一发动全身：优先在子模块内部小改，避免改公共 `pub` 签名。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-core
```
