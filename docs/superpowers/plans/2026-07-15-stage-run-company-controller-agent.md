# Stage Run Company Controller Agent 实现计划

**目标：** 把 `target_intel` V2 Stage Team从固定六 Producer + 事后 Aggregator改成每公司一个真实、
持续监控并可恢复、多轮调用 0..N durable SubAgent、自己提交 Gate 的 Company Controller。

**约束：** Controller主循环第一阶段不改 DB schema/migration、不手改 generated IPC、不执行外部目标或真实扫描；
沿用现有 WorkItem/WorkerRun/lease/checkpoint/Gate。采用聚焦 TDD；用户已授权删除 fixed
Producer/Aggregator执行兼容，旧 session只提示重跑。

## Task 1：锁定 Controller-only seed 并删除旧模式

**文件：**

- `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- `resources/harness/stages/target_intel/spec.json`

**步骤：**

1. 先加 RED：controller mode只 seed `leader:primary`，不 seed coverage-axis Producers或第二 Aggregator。
2. StageTeam policy只保留 Company Controller和 server-owned child limits。
3. 删除 legacy fixed scheduler seed、资源字段与运行测试。

## Task 2：实现 durable Controller claim / park / resume compound transactions

**文件：**

- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`

**步骤：**

1. RED锁定 Controller首次 claim、park后 `waiting_dependency/waiting_background`、同 WorkerRun/chain resume。
2. 增 exact Controller fence与依赖/output barrier；children未闭合时 resume fail closed。
3. lease/checkpoint/attempt epoch在一个事务内推进；response-loss replay不创建新 Controller。

## Task 3：实现 Controller-owned dynamic child request

**文件：**

- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- `backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- 相关聚焦测试

**步骤：**

1. 只允许 exact Controller请求；Producer继续禁止嵌套/动态派发。
2. server派生 child output schema/budget/scope；校验 role/kind/objective/subject/dedupe与 limits。
3. `stage_team_dispatch_workers`批量 durable写入后触发 waiting barrier；外层 scheduler持续监控 child，
   不在 Controller provider调用内递归执行 child。
4. 持久化 parent tool-call identity，使 UI按真实 identity嵌套，而非工具名前缀。

## Task 4：改 stage_run queue drain 为 Controller循环

**文件：**

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- `backend/crates/golish-sub-agents/src/defaults/prompts/`

**步骤：**

1. 把 frozen organizations作为 durable Unit队列；移除 per-org serial假设，增加有界公司级并发和
   operation级总 live-agent budget。
2. 每个已领取 Unit先执行 Controller；它进入 `waiting_for_subagents`后 scheduler继续 drain并监控该公司
   children，Controller不是停止或晚启动。
3. children terminal后继续同一 Controller时间线，并注入 immutable output manifest/current DB truth。
4. Controller可重复多轮；准备提交时关闭 epoch并绑定自己为 final submitter。
5. 一家公司 waiting/Gate BLOCK不能阻塞其他 Unit；用稳定顺序 + aging避免主/子公司饥饿。
6. 保留 deterministic Gate；child自然语言不改变 coverage/final status。
7. Controller和children都通过现有 exact-scope ContextPack入口注入长期上下文；加测试证明检索失败
   不回退 global/sibling memory，且 resumed Controller复用原 message chain。

## Task 4A：给 Controller 接 Codex 同款 update_plan

**文件：**

- `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 相关聚焦测试

**步骤：**

1. RED锁定：只有 exact Company Controller看到同名 `update_plan`；ordinary child和无 binding调用均拒绝。
2. 复用 Golish现有 Codex-compatible schema与校验：1..12 steps、pending/in_progress/completed、最多一个
   in_progress；返回 explanation、normalized plan和 summary。
3. `update_plan`不触发 scheduler barrier；Controller可在同一 chain继续工具调用，并在 dispatch/child
   output/Gate gap后重写计划。plan tool-call/result随 bound chain checkpoint持久化，不写全局 Stage plan。
4. Controller prompt要求复杂 Unit首轮建计划且状态随执行更新；children prompt/allowlist不得获得计划权。
5. SubAgent详情对 Controller的 `update_plan`渲染 Codex式计划卡；计划状态不参与 Unit/Gate truth。

## Task 5：实现 Controller-first UI 与真实 Agent树

**文件：**

- `frontend/components/Engagement/StageTeamRunView.tsx`
- `frontend/components/Engagement/StageRunOrgRows.tsx`
- `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 相关 Vitest

**步骤：**

1. RED锁定主公司 + 子公司显示为真实并发队列；已启动项显示 Controller摘要，queued项显示队列态，
   不铺固定六卡。
2. Controller时间线内按 persisted parent identity嵌套 children，支持 drill-in/返回。
3. 只提供新 Controller运行流；旧 fixed Team明确提示重跑且不得提供 Aggregator入口。
4. waiting/no-transcript/Gate BLOCK状态均使用权威 read model文案。

## Task 6：Gate repair exact-chain continuity（等待 migration 明确授权）

1. 已用 compound transaction持久化 gap/checkpoint，并恢复同一 leader WorkItem、WorkerRun和 message
   chain；不创建 fresh Aggregator。
2. 当前 trigger/epoch parent fence与 gap来源唯一约束会阻止该 Controller在 repair轮继续追加 child。
3. 获得用户明确 schema授权后新增向前 migration：受限 same-epoch Controller reopen、移除 gap来源
   WorkerRun唯一约束并保留普通索引，以 gap条数限制 repair fuel。
4. 加 RED/GREEN覆盖：首次 BLOCK后同 Controller派新 child、关闭新 manifest再提交；fuel耗尽和 response-loss
   replay仍确定性终止。

## Task 7：聚焦验证和状态记录

1. Cargo前运行 `just space-guard`。
2. 聚焦运行 seed、DB compound lifecycle、runtime controller loop、sub-agent tool、frontend identity/UI测试。
3. 运行 scoped rustfmt、Biome、TypeScript、`jq empty feature_list.json` 与 scoped diff check。
4. 更新模块卡、INDEX、feature和 progress；未跑 fresh app/live/precommit时继续 `in_progress`。
