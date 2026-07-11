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
| `stage_refiner` | Stage-aware deterministic repair owner：submit/gate 失败后生成 capability-first `RepairDirective`，并转换为 sub-agent `SubmitRepairMode` |
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
- `stage_refiner` 是 gate/submit 失败后的唯一修复建议来源：gate 仍决定 PASS/BLOCK，StageRefiner 只查询/汇总 gate+DB-backed context，生成下一步 `RepairDirective`，不能伪造 StageDeliverable。EAS coverage-gap repair 必须 batch-first，把同 technique 的 sibling gaps 合成少量 backend wrapper 调用；repair lock 的 allowed tools 必须是 `stage_worklist_status` / `stage_worklist_next` / `list_recent_evidence` / `query_target_data` / `check_stage_asset_coverage` / `eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services` / `eas_fingerprint_web_stack` / job wait/check/kill / `submit_stage_deliverable`，不要暴露 raw `pentest_run` 或 raw `whatweb`。concrete IP/CIDR 的 LIVENESS gap 先收敛到 `eas_discover_ports`，端口扫描结果同时写 PORT 与 LIVENESS；SERVICE-FINGERPRINT repair 只给 `eas_fingerprint_services`，先读已确认 open ports，对 host:port 分组跑 nmap recipe；WEB-FINGERPRINT repair 只给 `eas_fingerprint_web_stack`，并且只对 confirmed HTTP(S) origin 跑，不能作为 DNS/MySQL/SSH 等通用 service fallback。submit 同时报 evidence_ref 和 coverage gap 时优先修 coverage gap。Enumeration coverage-gap repair 的 allowed tools 必须保留 `stage_worklist_status` / `stage_worklist_next`、`list_enumeration_web_roots`、`list_recent_evidence` 和 direct enum wrappers（含 `enum_crawl_same_origin_urls`），让恢复/重试从 DB worklist/带 `target_id` web roots 重新取当前 gap，而不是困在 submit 时那批 action 或回到 URL 猜测。无开放端口、不可解析或批次过宽的资产要用 blocked/not_applicable/checked_empty 终态说明收口，不能 broad service sweep 后循环等待。
- `RepairDirective.actions` 是 tool guard/checkpoint 的全量结构化真相，不能为了省 prompt 而截断；`model_instruction()` 是独立有界投影：写全量 `total + stable hash`、只展示原序前 20 条、明确用 `stage_worklist_next` 分页，并受 32 KiB UTF-8 硬上限约束。`to_submit_repair_mode()` 可携带这段已投影 directive，但下层不得再次展开同一 action list。
- StageRefiner 的 coverage-gap repair 先消费 `CoverageGapAction.suggested_capabilities`，缺失时再用 `stage_capability` registry 按 stage+technique 回填；`suggested_tools` 只作为兼容/具体实现 hint，并会把 EAS 的 raw `httpx/naabu/masscan/nmap/whatweb` hint 映射到 `eas_*` wrapper。SERVICE-FINGERPRINT 应收敛到 `eas.fingerprint_services`，WEB-FINGERPRINT 应收敛到 `eas.fingerprint_web_stack`，不能因为自然语言 repair reason 又放大成 broad service sweep。
- EAS 的 `asset_wave_barrier` 会让 Task-mode stage close gate 用当前 durable wave asset list（无 wave 时才用 `operation_state.stage_started_at`）冻结 in-scope asset 轴；运行中新发现的 target 不阻塞当前 wave，待当前 wave PASS 后再按 parent wave `started_at` 之后的新入库 target 作为 supplemental wave 输入。
- Task-mode EAS close 不能读取 chat-session 全局 audit facts：只读 producer-org + current-target-owner + stage-fresh 的 guarded facts，并与同 org/run fresh `technique_outcomes`、fresh business Found 做交集；缺 operation `stage_started_at` 时不允许 presence fallback。该合同与 per-org `org_gate`、submit preview、StageAssetCoverage 相同。
- Task-mode stage close gate 构造 `GateContext` 时必须按 stage 注入 `web_capable_assets`：Enumeration 只在 `StageSpec.enum_ip_web_coverage=true` 时从 repo 读取 EAS/httpx 证明为 Web 服务的 IP/CIDR；EAS 则读取 `eas_web_capable_assets`，让 WEB-FINGERPRINT 只覆盖已确认 HTTP surface 的资产。缺省/查询失败保持旧裸 IP 不适用口径，不能让 stage-close 和 per-org `org_gate` / `ai_get_stage_asset_coverage` 的 IP-web/WEB-FP 分母漂移。
- Enumeration close gate 必须从成功的 coverage snapshot 保留 exact-origin 轴：snapshot 明确 `assets: []` 时向 `GateContextBuilder::authoritative_in_scope_assets` 传 `Some([])`，与读模型一样 vacuous PASS；snapshot 缺失/查询失败则保持 `None` 并 fail-closed 回到自报轴。Enumeration 的 expected techniques 始终是 JS/DIR/PARAM/JSAPI 四轴，且只有存在 `stage_started_at` freshness cutoff 才可读取 `technique_outcomes`。
- `prompts::stage_charter` 会把 `StageSpec` 的 gate contract 渲染给执行 agent；`external_attack_surface` 有专门文案说明 domain/url 只做 LIVENESS/WEB-FINGERPRINT、IP/CIDR 先做 PORT 再做 SERVICE，以及 SERVICE-FINGERPRINT 的 `tested_units/total_units = 已指纹开放端口/发现开放端口`；同 IP 多域名/Host/SNI 时，nmap 的 SERVICE 以 IP:port 为单位，WhatWeb 的 WEB-FINGERPRINT 以 `scheme://host:port` origin 为单位。
- `prompts::stage_charter` 的 submit 示例不要把内部 canonical empty arrays 伪装成模型必填字段：`submit_stage_deliverable` 由后端补齐省略的 `evidence_refs` / `findings` / `coverage` / `skipped_checks` / `required_checks_done`；各阶段都不要求模型手写 evidence ids，若模型写了 id 则必须是真实 ledger id。普通 scope exclusion 不应被描述成 skipped check。
- 带 `StageSpec.specialist` 的阶段（如 `target_intel` → `recon`）必须让 primary 通过 `stage_run` 扇出；`synthesize_stage_subtask` / prompt 不应再要求 primary 直接调用 `recon_*`，否则会绕过 per-org worker gate。
- `TaskOrchestrator::set_force_stage_run_on_resume_once` 只用于裸 continuation resume：若当前 active stage 有 `StageSpec.specialist` 且已绑定 `engagement_root`，下一次 primary loop 会通过 `ExecutionContext.harness_forced_tool="stage_run"` 强制先 dispatch `stage_run`；非 specialist/rootless/带 steering 的 resume 保持普通 agentic loop。该 hint 是 one-shot，进入 loop 后必须清掉，避免后续聊天继承。
- `TaskOrchestrator::resume(task_id, user_message, ...)` 把本次非空 continuation/steering 文本作为 request-local `ExecutionContext.task_input`，继而由 bridge 传到 `SubAgentContext.original_request` 和 stage-run worker 的 operator-constraint 摘录；不覆写 `tasks.input` 中的 durable 初始目标。空或全空白 continuation 明确回退 durable 初始目标。worker chain/checkpoint 和 request-scoped reentry guard 不参与该选择。
- Graph-flow 分支路由的 `made_progress` 必须按 stage 语义判断：漏洞阶段可以用 `findings_count`，但 `findings_allowed=false` 的信息收集/覆盖矩阵阶段（`target_intel` / `external_attack_surface` / `enumeration`）不能要求 findings；它们通过 DB/ledger truth、coverage handoff、stage_run pass token 等信号表示“有进展”，不要依赖模型提交的 evidence ids。否则 EAS 会因 `findings=[]` 被误判为无进展并走 `reporting` 短路，跳过 `enumeration`。
- RuntimeSupervisor 是运行中策略纠偏机制：只在 `ExecutionMonitor` 触发后运行，模型输出必须先解析成 `StrategyDirective` 并经过 stage/tool policy 裁剪，不能覆盖 gate，也不能覆盖 StageRefiner 的 post-gate `RepairDirective`。repair/stage_run 的确定性补洞仍以 StageRefiner 为准。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator
```
