# golish-agent-runtime

> **一句话职责**：**高层 agent runtime**（Layer 4b）——流式 tool-call loop、sub-agent 派发、exact-scope ContextPack 注入、上下文压缩、eval harness 与 test_utils。

- **类型**：crate（Layer 4b · agent 高层）
- **路径**：`backend/crates/golish-agent-runtime/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 agentic loop 主体（流式 tool-call 循环、turn 处理、stream_processor）时
- 改上下文压缩 / 摘要触发（`maybe_compact` / `apply_compaction`）、transcript/artifacts/summaries 目录解析时
- 改 `stage_run` / sub-agent 派发 / harness repair checkpoint 传递时
- 改 evals 评测 harness（`eval_support`）或共享 mock（`test_utils`）时

## 职责

承载约 6.5 KLOC 的流式 loop 主体（从 `golish-agentic-loop` 在 A2 改名）。从 `golish-agent-kit`（L4a）拆出，是为了把 rig-core 重泛型实例化的 loop 与底层基础设施分开编译，恢复增量编辑。下游（bridge / app / evals）直接从这里 import loop 入口。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `run_agentic_loop` / `_generic` / `_unified` | 流式 tool-call loop 入口 |
| `AgenticLoopConfig` / `AgenticLoopContext` | loop 配置与上下文 |
| `retrieve_scoped_context_data` | active harness 的 operation/execution/unit/org/stage identity 组装与 ContextPack provider 调用；输出 data-only block |
| `LoopLlmRefs` / `LoopEventRefs` / `LoopCaptureContext` / `LoopAccessControl` | loop 依赖注入引用束 |
| `maybe_compact` / `apply_compaction` / `CompactionResult` | 上下文压缩 |
| `get_transcript_dir(_for)` / `get_artifacts_dir(_for)` / `get_summaries_dir(_for)` | 产物目录解析 |
| `McpToolExecutor` / `OutputClassifier` / `PostShellHook` / `TerminalErrorEmitted` | 工具执行/分类/钩子 |
| `StageRunReentryGuard` | 顶层 Task 请求内的 stage-run retry-exhaustion 断路器；新用户请求重置 |
| V2 worker runtime | frozen-scope seed、exact claim/prebound chain、10s/30s heartbeat、tool/chain fencing；PASS 只允许 final-seal seam 发布 |
| sub-agent persistence identity wiring | 事件 `session_id` 与 `DbTracker::session_uuid()` 分离；后者用于 message-chain create/resume |
| `eval_support` / `test_utils`（feature `test-utils`） | 评测 harness / 共享 mock |

## 依赖

- **内部**：`golish-agent-kit`（核心下层）、`golish-memory-app`、`golish-memory-domain`、`golish-core`、`golish-context`、`golish-events`、`golish-llm-providers`、`golish-settings`、`golish-tools`、`golish-sub-agents`、`golish-prompts`、`golish-indexer`、`golish-json-repair`
- **外部**：`rig-core`、`rig-anthropic-vertex`、`rig-openai-responses`、`tokenx-rs`

## 被谁依赖 / 改动影响面

`golish-agent-bridge`、`golish-agent-app`、`golish`。改 loop 入口签名会波及 bridge 与 app 层。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `agentic_loop/` | 流式 loop 主体（turn/stream_processor） | [→](golish-agent-runtime/agentic_loop.md) |
| `execution_mode/` | 执行模式策略框架（policy/registry） | [→](golish-agent-runtime/execution_mode.md) |
| `eval_support/` | evals 评测 harness | [→](golish-agent-runtime/eval_support.md) |
| `test_utils/` | 共享 mock（feature test-utils） | [→](golish-agent-runtime/test_utils.md) |

## 关键文件

- `agentic_loop/tool_execution/direct/stage_run_call.rs`：Task harness `stage_run`；stage fork 进入后仍复用同一 stage/team/candidate plan，仅把 Candidate 的 initial authority识别为 exact `ForkedVulnHandoff`；V2 Target Intel / EAS / Enumeration / Vuln 都走 durable Company Controller + sibling Team queue，child 在冻结 cap 内任一完成即滚动补位，且至多一个持久 pending claim 与已有 child completion 同时 poll，避免 refill 等待 completion 所持 operation row lock时自锁。每个 WorkerRun 保持独立 SubAgent UI parent identity、producer 有界 attempt retry、唯一 final submitter 与 Gate repair。Vuln Gate PASS后还会把可信 Enumeration-manifest structural N/A以当前operation/org/Unit/session/project的新鲜aggregate attestation重新锚定并物化，再做完整outcome-set final seal；Candidate Verification 走 TerminalIntent recovery与同链submit-only continuation。
- `agentic_loop/tool_execution/direct/stage_team_scheduler.rs`：Stage Team plan/WorkItem/output/request 的确定性构造、唯一 fenced-object bounded parser、business output authority validator、stable hash 与 repair fuel；Company Controller 的历史 lifetime totals 只做 restart replay，不参与 admission/claim/retry/coordination loop。
- `agentic_loop/tool_execution/direct/sub_agent_call.rs`：`sub_agent_*` 派发、Stage Team dispatch host router、sub-agent repair checkpoint 恢复、tool observer；零项持久化成功时返回带 request/card identity 的 terminal `dispatch_failed`，不能让前端把失败误画成 queued。
- `test_utils.rs` / `test_utils_tests.rs`：feature gate 下的 mock 与自测。

## 注意事项 / 坑

- 与 `golish-agent-kit` 是**有意分家**（A2，编译预算）：底层在 kit，loop 在此。
- `test-utils` feature 才会编 `test_utils`（并拉 `tempfile`、传递给 `golish-agent-kit/test-utils`）；普通 release 不付出成本。
- crate 级 `#![allow(too_many_arguments)]`：loop 主体按设计透传宽 context。
- active harness 的 scoped ContextPack 缺 identity/provider 或 retrieval 失败时只能省略/报错；不得把 legacy global memories/wiki 重新注入 customer operation。非-harness 普通 sub-agent 的 legacy briefing 不因此改变。
- main-agent tool execution 会把 `event_tx` 通过 `golish_core::with_agent_tool_output_sender` 绑定到当前 `AgentToolContext`，并把 `AgenticLoopContext.harness_operation_id` 作为可信 stage-attempt id 注入；这样 bridge tools 既能发 `tool_output_chunk`，也无需接受模型伪造的 operation id。新增绕过 `single_tool_call` 的执行路径时，要同步包上 tool context + output sender。
- `sub_agent_call.rs` 构造的每个 `SubAgentExecutorContext`（override model、override fallback、normal model）都必须透传同一 `harness_operation_id`；Reflector 和 bridge 的直接 sub-agent 兼容路径也要显式传递或置空。nested delegate 继续继承父 worker 的 operation id，不能生成新的 stage-attempt identity。
- `SubAgentExecutorContext.session_id` 只用于 Langfuse/transcript 事件路由，可能是 `stage-run-*` 等非 UUID 文本键；message-chain persistence 必须另传 `DbTracker::session_uuid()` 到 `persistence_session_id`。不得再解析或改写事件键来猜 DB session；否则 capacity continuation 会丢失精确 worker chain。
- sub-agent chain 错误必须以稳定 typed kind 进入 `sub_agent_call` 结果：`restore_exact` 仅在已有同一 chain id 时允许有界同链 retry，`create_fresh` 可在工具尚未执行时重试，`restore_latest` / `finalize` 直接 fail closed。尤其 finalize 失败表示 worker 已可能产生外部副作用，不能落入普通 gate retry 再派 fresh worker。
- sub-agent 返回 `success=false` 时，runtime 的 tool result 与 `sub_agent_dispatches` 生命周期都必须记失败；不能因 Rust 外层是 `Ok(SubAgentResult)` 就把 provider stream error、timeout 或显式失败标为 completed。
- legacy `stage_run` 和 `sub_agent_call` 共享同一个 per-org `agent_path` checkpoint；`submit_stage_deliverable needs_fix` 里的 `SubmitRepairMode.coverage_gap_actions` 必须被 `stage_run` 接住并继续持久化。V2 Stage Team 不能复用这条共享 checkpoint：每个 sibling WorkItem 必须绑定自己的 WorkerRun/message chain/lease，Gate BLOCK 用 immutable gap + 新 repair generation恢复。
- V2-writing operation 的 stage worker lifecycle 只调用 `RuntimeMemoryRepository` compound APIs；禁止把 generic message-chain update、worker checkpoint、Unit transition 拼成顺序写。普通兼容 Worker 的 GateBlocked/Exhausted 可由 worker+Unit compound finish 落地；Team producer只结束自身 WorkItem/Output，不能关闭 Unit，只有 Aggregator final seal可 PASS Unit，Gate BLOCK必须终结当前 Aggregator并新开有界 repair generation。
- Team producer 的 provider 执行失败、输出协议不合格、无 canonical fact/evidence authority 的 `found/checked_empty`，以及已登记的 dependency-not-ready blocker，不得第一次就固化成 business `blocked` output；它们必须走 `retry_stage_worker` compound API，在 frozen attempt budget 内重新排队，预算耗尽才由 repository 生成确定性 terminal blocker。合法未知 business blocker 仍是 immutable output，不能被自动重试规则吞掉。
- 大型 Enumeration worklist 可能超过 Enumerator 单段 40 iterations；worker 无 deliverable 返回时，`stage_run` 会读取同源 DB coverage snapshot，只有 pending/error/partial 数量继续下降时才续同一精确 worker chain，最多两次。ready 但未 submit 只允许一次 submit-only continuation；停滞、取消或预算耗尽进入既有 request-scoped breaker，不能无限重开。
- `stage_run_call::build_org_objective` 会把本次 Task 的 `SubAgentContext.original_request` 作为**有界、JSON 引用、低优先级**的 operator-constraint 摘录传给 specialist worker（最多 4096 Unicode 字符，超长保留首尾并显式标记截断）。该文本只可收紧现有执行方式（如 read-only、批次、已知不可达 exact origin、禁止某 producer），绝不能变成新的授权源：stage / authoritative org subtree / DB scope / exact-origin denominator / StageSpec tool boundary / evidence+gate contract 仍由 Rust 侧固定，冲突文字必须忽略。
- Vuln Company Controller 是例外的 formulaic host path：runtime 从 operation-scoped coverage 生成 exact origin × capability shard，并以 canonical Target subject持久化；Nuclei shard 由 host 直调 guarded wrapper，pending 可按 capability 合并，partial/error 只生成 single-technique recovery。primary/narrowed预算固定为300/600秒；旧代码已经停在attempt 3的精确`scanner_runtime/scan_budget_exhausted` cell可生成唯一attempt-4 `budget_recovery`，其他retry-disabled/runtime failure与attempt 4都不重开。重复 stable request 必须区分 claimable、in-flight 与 recovery-required，不能回退到“ALL remaining” LLM派工。
- Vuln structural N/A不能直接引用早于Vuln Unit的Enumeration evidence。runtime只接受exact final-sealed Enumeration handoff的operation/org/scope/stage/schema/authority lineage，随后追加一条facts=None的新鲜attestation；其raw有界保存handoff hash/gate/source evidence与排序后的canonical N/A cell。append返回0、identity漂移、重复/伪造cell或conditional outcome upsert失败都阻止final seal，现有producer终态仍优先且不被降级。
- Company Controller 的 child drain 对四个普通阶段共用同一个 rolling refill driver：`max_workers_active - 1` 仍是每公司 child 上限，任一 child 完成后立刻重试 durable claim，不等待慢 sibling；global provider semaphore与DB active-worker fence仍可进一步限流。claim保持串行且最多一个pending future；driver用child优先的并发poll推进claim与`FuturesUnordered` completion，不能在poll sibling前同步await refill。repository返回`None`只暂停到下一child进展，避免空转。重启续跑时，barrier只要仍有 required producer未终态，就必须进入drain并领取尚无WorkerRun的queued WorkItem；不能因`live_workers=0`误判为无可运行child。execution error只记录第一项并继续排空 queued/retry工作；claim/storage error或取消停止新 claim，但已经启动的child以及已成功claim的pending结果都必须执行到既有 lifecycle/evidence landing边界后才返回错误。

## Company finalizer restart recovery（2026-07-20）

- Gate PASS 后的 materialization/final-seal error 会先调用 typed DB parking seam，释放 exact final submitter租约并排回同一 Worker/message chain；当前 request随后以 `company_controller_finalization_failed` halt。
- aggregator claim收到 DB的 `stage_team_final_submitter_runtime_replaced` 时生成 `COMPANY_CONTROLLER_RUNTIME_RECOVERED`，说明旧坏 execution已追加式替换、operation facts/evidence保留，并要求下一独立“继续”；不再泛化为业务 Gate BLOCK。
- tool dispatch识别 final submission missing、finalization failed与runtime recovered三种 closeout halt，阻止同一 tool batch继续 coverage/submit。
- legacy no-purge replacement进入 Vuln coverage前先调用 exact adoption seam，再重新读取DB worklist；adoption不能直接声明PASS。replacement Unit 与operation共用source freshness epoch，保证coverage、Gate和final-seal resolver看到同一180-cell窗口。
- parked finalizer checkpoint允许provider chain是JSON array或object；runtime只解开server-owned `{_runtime,chain}` wrapper后恢复原链，不能把array解析失败覆盖成新的generic closeout错误。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime
```
