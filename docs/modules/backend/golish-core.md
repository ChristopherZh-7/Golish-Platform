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
| `InvestigationContractVersion` / `InvestigationRolloutMode` / `InvestigationModePolicy` | Candidate/Hypothesis Registry 的 operation-frozen contract、五态 rollout 与唯一纯 policy matrix；未知 wire 值严格拒绝，不 fallback |
| `InvestigationErrorCode` | Investigation read/write/authority 边界使用的 8 个稳定错误码闭集 |
| `hypothesis_semantic_key::*` | Plan B/C 共用的 canonical JSON、claim polarity、semantic-key SHA-256、Candidate 非终态闭集以及 initial/split/merge/derive/revision UUIDv5 公式 |
| `verification_contract::*` | 唯一 host-compiled `VerificationContractV1`；四种 combinator、predicate/control/pair/order exact sets 与 persisted replay validator |
| `hypothesis_verification::*` | revision claim-component/objective/proof-path seal、typed VerificationContract binding、outer truth reducer，以及 Plan C 无环 adjudication/transition authority DTO；完整只读 getters 供 repo 精确持久化 |
| `investigation_projection::*` | Projection/Timeline closed catalogs、Plan B verification-plan exact-one route、Plan C same-revision terminal exact-five manifest，以及支持 bounded opaque string entity id 的 typed source/entity records；共享 `projection_entity_hash_v1` / `projection_change_hash_v1` / `projection_event_id_v1` 让 projector 与 Timeline reader重算完整 body/source/change identity，四个 public enum 是 ts-rs 唯一 Rust 来源 |
| `investigation_comparison::*` | `comparison_record.v1` whole-record canonical compiler与 `compare_whole_records_v1`；authority basis只接受 `plan_b_checked` 或 `grandfathered_legacy`，跨 basis、缺任一侧或不完整记录一律 `incomplete`，绝不逐字段混读 |
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

`agent_mode.rs`、`agent_session.rs`、`attack_execution.rs`、`investigation_contract.rs`、`hypothesis_semantic_key.rs`、`verification_contract.rs`、`hypothesis_verification.rs`、`investigation_projection.rs`、`investigation_comparison.rs`、`hitl.rs`、`plan.rs`、`prompt.rs`、`runtime.rs`、`session_manager.rs`、`tool.rs`、`tool_args.rs`、`textual_tool_call.rs`、`ready_gate.rs`、`skill_provider.rs`、`pentest_context.rs`、`event_emitter.rs`、`vault.rs`、`web_fetch.rs`、`paths.rs`、`os.rs`、`jsonl.rs`、`time.rs`、`utils.rs`、`message.rs`、`api_request_stats.rs`、`session_kind.rs`。

## 注意事项 / 坑

- 跨 IPC 的类型若在此定义，必须 `#[derive(ts_rs::TS)]` 同步前端（不变量 I5）。
- `AttackExecutionContract` 在这里仅定义稳定纯类型与 rollout 语义；deployment default、operation row 冻结、DB constraint/immutable trigger 属于 Candidate V2 后续 schema/repo task，不能用环境变量在 operation 中途覆盖。
- `InvestigationRolloutMode::policy()` 是 Plan B/C/D 共用的唯一 final policy。这里不读取 deployment/operation row；上层必须先冻结合法 contract/mode pair，再消费 policy，不能按组件另造布尔解释。
- Hypothesis sealed DTO 的 member/count/set/final hash 只能由 core compiler 或 persisted replay validator 生成；plan objective直接持有并校验sealed `VerificationContractV1`，不能用caller自报contract id/hash替换。`CandidateMutationEpistemicState`不含`verified/refuted/invalid`。Plan C 必须直接复用这里的 VerificationContract、proof-path reducer与 transition hash 顺序。
- Projection source/entity payload不是自由JSON/type tag：每个 entity kind有独立typed record wrapper，schema固定为V1，canonical redacted body在constructor和deserialize replay时重算SHA-256；Plan C terminal manifest还要求五个member绑定同一revision且source hash互异。
- Projection enum 在 Task 4 只运行内存 `TS::decl()` golden；实际写入 `frontend/lib/generated/` 只由 Task 11 的授权生成步骤完成，禁止手改。
- `comparison_record.v1` 对 Plan C 字段使用显式 `not_available_plan_c`，并冻结 wire/residual membership；这只是未来接口形状，不安装 Plan C capability assessment、adjudication或terminal authority。
- `agent_session.rs` 的 task-local attribution 是 best-effort：只对 inline awaited work 生效，启动后台 job 时要立即 capture，不能等到 spawned task 里再读。`AgentToolContext.operation_id` 来自 runtime 的 active harness operation/stage attempt，不能从模型参数猜；`organization_id` 承载当前 harness org。后台 completion 用这些可信绑定把结构化扫描结果、证据和 coverage outcome 写回正确 run/org。
- `AgentToolCancellation` 是 sticky task-local取消通道：wrapper/runner在 inline scope捕获 clone，Stop 后所有观察者都能看到同一状态；等待实现必须在注册 `Notify` 前后都重查 flag，避免 cancel 与 waiter注册竞态。它只传递取消，不替代具体工具的 kill/await/landing责任。
- direct/bridge 工具如果要让前端实时看到“工具现在在看什么”，用 `emit_current_agent_tool_output_chunk` 发 chunk；主 loop / sub-agent executor 会注入 `with_agent_tool_output_sender`。如果工具自己 `tokio::spawn` 读子进程 stderr/stdout，必须先在 inline scope capture `current_agent_tool_context()` 和 `current_agent_tool_output_sender()`，spawn 里不能再读 task-local。
- 改动牵一发动全身：优先在子模块内部小改，避免改公共 `pub` 签名。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-core
```
