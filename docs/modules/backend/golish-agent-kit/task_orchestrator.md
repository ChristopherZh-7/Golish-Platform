# golish-agent-kit / task_orchestrator

> **一句话职责**：harness 驱动的自动化任务执行——一个 Task 由 metalcraft Executor 在 profile 投影的 Operation DAG 上推进：每阶段自规划+派发 specialist、提交 StageDeliverable、过确定性 evidence gate 才前进（大阶段边界 HITL），末尾 reporter 收尾。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/task_orchestrator/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Task 模式编排（stage 自规划/派发/gate/前进、reporter 收尾）时
- 改 `AgentExecutor` trait、`TaskOrchestrator::run` 入口、per-stage agentic loop 时
- 改 operation/task 断线恢复、runtime correction/refiner checkpoint、`operation_state.state_blob` schema 时

## 职责

在 `AgentBridge` 之上编排整个 Task 生命周期 + DB 持久化，每次 agent 调用回落到 bridge。`subtask_phases` 跑 Executor 驱动的 operation loop + `execute_single_subtask`（per-stage loop + gate）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TaskOrchestrator`（`run`） | 编排主体 + 入口 + 事件发射 |
| `AgentExecutor`（trait） | 每次 agent 调用的抽象（bridge 实现在 `golish-agent-bridge::bridge_executor`） |
| `agent_run_checkpoint` | P2a 细粒度 agent-run checkpoint DTO + `state_blob.agent_run` merge helpers |
| `stage_refiner` | Stage-aware deterministic repair owner：submit/gate 失败后生成 `RepairDirective`，并转换为 sub-agent `SubmitRepairMode` |
| `runtime_supervisor` | PentAGI-style in-run strategy supervisor：重复/停滞工具触发后解析 LLM JSON，按 stage/tool policy 裁剪成 `StrategyDirective` |
| `types`（planning DTO / token usage / 执行上下文） | 编排类型 |
| `prompts` | 各阶段 prompt 模板 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `orchestrator.rs` | `TaskOrchestrator` + `run` |
| `agent_run_checkpoint.rs` | resumable agent-run 状态：pending correction、background job ids、last tool ref、`submit_repair_mode`、`repair_directive` |
| `continuity.rs` | DB-backed continuity preflight：只读 in-scope org + `org_stage_completions`，构建可确认复用的 `ContinuityAdoptionPlan` |
| `stage_refiner.rs` | `RepairDirective` DTO + deterministic StageRefiner：EAS coverage gaps / TargetIntel provider-only repair / background/evidence refs |
| `runtime_supervisor.rs` | `StrategyDirective` DTO + RuntimeSupervisor prompt/parse/sanitize：运行中防跑偏，不决定 gate |
| `subtask_phases/` | Executor loop + `execute_single_subtask`（per-stage gate） |
| `types.rs` / `prompts/` / `helpers.rs` | DTO/trait · prompt 模板 · 共享小函数 |

## 依赖

- crate 内 `harness`（gate/DAG）、`AgentBridge`（经 trait）；`golish-events`

## 注意事项 / 坑

- **不变量 I7/I8**：每阶段必须过 evidence gate 才前进；gate 是确定性规则，不能拿「agent 自信说完成」当通过。
- `bridge_executor`（`AgentExecutor` 实现）在 `golish-agent-bridge`（依赖 AgentBridge）；本模块只持 trait。
- graph-flow 的 `operation_state.current_stage` 表示**当前正在执行的 stage**：进入新 stage 时同步并刷新 `stage_started_at`；断线后回到同一 stage 时不能重复刷新，否则 freshness-window gate 会看不到断线前已落库的 evidence。
- `operation_state.state_blob` 是 graph checkpoint、stage_run worker resume 等多消费者共享 JSON；更新 `graph_flow` 时要 merge 保留其他 key，不能整段覆盖。
- `operation_state.state_blob.agent_run` 是 P2 细粒度恢复槽：只存稳定、可恢复的轻量状态（如 pending gate correction、submit repair directive、background job ids、last tool result ref），大段输出仍留 transcript/background job/evidence ledger。写入或清理时必须保留 `graph_flow` / `stage_run_workers` 等 sibling key。`repair_directive` 是 StageRefiner 的结构化真相源，`submit_repair_mode` 是给 executor/tool guard 的兼容投影。
- Cross-session adoption 不等于 resume：resume 继续同一个 `graph_flow` checkpoint；adoption 是新 operation 经用户确认后复用旧 DB truth。`TaskOrchestrator::set_continuity_adoption` 会同时设置 entry stage 和 remaining-stage allowlist，否则 metalcraft Executor 仍会从 profile DAG entry（通常 scoping）启动。
- continuity preflight 不能在没有 `engagement_root` 的情况下 adopt `scoping`：legacy `in_scope_org_ids(None)` 可能包含 sibling/test org，跳过 scoping 会让后续 pass-token/coverage/gate 落回全库口径并污染当前任务。
- Red-team scoping 的 anti-shortcut gate 只强制真实 `unit_review`，不再强制 `manage_organizations(create)`：已有 root/org tree 经用户确认后就是可复用记录，不能为了满足 gate 再 `create_batch` 扩树。只有 root 缺失或用户在 unit review 里显式新增/确认的单位才创建。
- `stage_refiner` 是 gate/submit 失败后的唯一修复建议来源：gate 仍决定 PASS/BLOCK，StageRefiner 只查询/汇总 gate+DB-backed context，生成下一步 `RepairDirective`，不能伪造 StageDeliverable。EAS coverage-gap repair 必须 batch-first，把同 technique 的 sibling gaps 合成少量 `pentest_run`（httpx stdin/input_lines；naabu/masscan/nmap/whatweb/gowitness 用 `{{input_file}}` + `input_lines`）再提交，避免被 gap_examples 带成一资产一调用。SERVICE-FINGERPRINT repair 只能对确认开放端口的 host:port 分组跑 `nmap -sV` / whatweb；无开放端口、不可解析或批次过宽的资产要用 blocked/not_applicable/checked_empty 终态说明收口，不能 broad service sweep 后循环等待。
- EAS 的 `asset_wave_barrier` 会让 Task-mode stage close gate 用 `operation_state.stage_started_at` 冻结 in-scope asset 轴；运行中新发现的 target 不阻塞当前 wave，待当前 wave PASS 后再作为 next-wave 输入。
- Task-mode stage close gate 构造 `GateContext` 时必须把 Enumeration 的 `web_capable_assets` 一并传入：`fetch_enumeration_web_capable_assets_for_gate` 只在 `StageSpec.enum_ip_web_coverage=true` 时从 repo 读取 EAS/httpx 证明为 Web 服务的 IP/CIDR。缺省/查询失败保持旧裸 IP 不适用口径，不能让 stage-close 和 per-org `org_gate` / `ai_get_stage_asset_coverage` 的 IP-web 分母漂移。
- `prompts::stage_charter` 会把 `StageSpec` 的 gate contract 渲染给执行 agent；`external_attack_surface` 有专门文案说明 domain/ip/url/cidr 的 coverage 差异，以及 SERVICE-FINGERPRINT 的 `tested_units/total_units = 已指纹开放端口/发现开放端口`。
- 带 `StageSpec.specialist` 的阶段（如 `target_intel` → `recon`）必须让 primary 通过 `stage_run` 扇出；`synthesize_stage_subtask` / prompt 不应再要求 primary 直接调用 `recon_*`，否则会绕过 per-org worker gate。
- `TaskOrchestrator::set_force_stage_run_on_resume_once` 只用于裸 continuation resume：若当前 active stage 有 `StageSpec.specialist` 且已绑定 `engagement_root`，下一次 primary loop 会通过 `ExecutionContext.harness_forced_tool="stage_run"` 强制先 dispatch `stage_run`；非 specialist/rootless/带 steering 的 resume 保持普通 agentic loop。该 hint 是 one-shot，进入 loop 后必须清掉，避免后续聊天继承。
- Graph-flow 分支路由的 `made_progress` 必须按 stage 语义判断：漏洞阶段可以用 `findings_count`，但 `findings_allowed=false` 的信息收集/覆盖矩阵阶段（`target_intel` / `external_attack_surface` / `enumeration`）不能要求 findings；它们通过 evidence refs、coverage handoff、stage_run pass token 等信号表示“有进展”。否则 EAS 会因 `findings=[]` 被误判为无进展并走 `reporting` 短路，跳过 `enumeration`。
- RuntimeSupervisor 是运行中策略纠偏机制：只在 `ExecutionMonitor` 触发后运行，模型输出必须先解析成 `StrategyDirective` 并经过 stage/tool policy 裁剪，不能覆盖 gate，也不能覆盖 StageRefiner 的 post-gate `RepairDirective`。repair/stage_run 的确定性补洞仍以 StageRefiner 为准。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator
```
