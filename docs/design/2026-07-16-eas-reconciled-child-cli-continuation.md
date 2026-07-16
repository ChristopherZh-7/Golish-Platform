# EAS reconciled child 与 CLI continuation 闭环设计

- **日期**：2026-07-16
- **状态**：已实现并由原实体 CLI successor Turn 验证；additive migration、exact Controller reopen、最终 EAS Gate PASS
  与数据库/transcript 审计均保留可重放证据
- **现场**：workspace `/Users/christopherzheng/golish-platform/Test1`，session
  `pentest-chat-1784179823492-1`，operation `a8029de1-9f37-4450-b7e9-f08f7ba4c371`
- **范围**：Company Controller 对 safe-reconciled child 的重新 drain；GUI/CLI 共用 exact
  operation-Turn claim；真实 CLI 续跑现有 operation
- **非目标**：放宽 Gate、重放旧工具参数、接受未知外部副作用、创建 replacement Agent、修改 IPC/frontend；除本次用户
  已明确授权的 successor-Turn additive migration 外，不扩展其它 schema 范围

## 1. 已确认根因

最后一次 continue 已成功把中断的 `eas_fingerprint_services` tool 原子封存为 failed，并把 exact
WorkItem/WorkerRun/message chain 重排为 queued。Controller 仍是 `waiting_background`，因此
`claim_stage_team_leader` 正确返回 `None`；但 runtime 随即直接报
`Company Controller is waiting but no runnable child WorkItem remains`，没有调用现有的
`drain_company_controller_children`。DB 中存在 runnable child，生产调度器却没有消费它。

现有 DB integration 随后由测试代码手工调用 `claim_stage_work_item`，所以只证明了持久化恢复合同，未覆盖真实
runtime 的 leader-none 分支。

CLI 还有第二个闭环缺口：GUI continuation 已使用 `source + open_turn_id` 的 durable Turn CAS，而
`golish --stage-run-resume` 仍使用独立的 waiting-to-running SQL，并拒绝非 `stage-run-*` session。它因此既不能接管当前
GUI operation，也不会关闭旧 Turn、追加 successor Turn。

## 2. Company Controller 恢复合同

Controller claim 返回 None 后，runtime 必须读取同一 exact barrier 并按以下顺序处理：

1. `recovery_required_workers > 0`：保持 typed operator recovery，禁止自动执行；
2. `live_workers > 0` 或 `retry_pending_work_items > 0`：调用
   `drain_company_controller_children`；至少一个 child 走完一次 durable execution 后，回到 Controller loop 再 claim
   原 Controller；
3. drain 返回 0 时重新读取 barrier；若出现 operator recovery，返回 typed blocker；若确实没有 live/retry child，才返回
   deterministic no-runnable error；
4. child claim/execute 保持原 WorkItem、WorkerRun、message chain；safe-reconcile 后只递增 `attempt_epoch`，先读取
   worklist，再按 exact gap 决定是否调用新工具。

`waiting_background` 表示 Controller 等依赖，不表示整个 Team 没有 runnable child。两者不得再合并成一个错误。

## 3. GUI/CLI 共用 Turn claim

CLI 继续保留 operation advisory lock、transcript workspace 校验、provider/model 不变、project scope、profile、stage、org、
runtime source、chain/tool fence 的 exact 校验。chat key 前缀不再承担授权职责；本地 CLI 可以通过 exact operation/session UUID
选择 `stage-run-*` 或 `pentest-chat-*` Task session，但仍必须命中当前 workspace 的 transcript 与 frozen project scope。

正常 running/waiting continuation 使用 `golish-agent-app` 的同一服务：

```text
select exact complete runtime source + exact open Turn
  -> prepare bridge/repository/project scope/orchestrator
  -> claim(source, open_turn_id, new_turn_id, trigger_input)
  -> old Turn = interrupted
  -> successor Turn = running
  -> task = running
  -> TaskOrchestrator.resume(existing operation)
```

两个 caller 竞争同一 open Turn 时最多一个成功。没有 live Worker lease、source/identity/row witness 任一漂移时均 fail closed。
CLI 不创建新 task/operation/session，不调用 `run_stage`，也不把 running task 先降成 waiting。

## 4. 真实验收

先以 focused test 锁定 leader-none → child drain 和 CLI Turn claim。随后使用当前已授权 operation 的 exact UUID、workspace、
provider/model 运行真实 `golish --stage-run-resume`。验收必须同时证明：

- operation/task/session/WorkItem/WorkerRun/message chain 均不变；
- old Turn 被 interrupted，新增连续 ordinal Turn；
- safe-reconciled child 被实际 claim，`attempt_epoch` 增加，child transcript 出现新 Turn；
- recovery directive 先要求 authoritative worklist，不重放旧 tool args；
- Controller 回到同一 chain，EAS 最终由 deterministic Gate 决定 PASS/BLOCK；
- 不再出现 reconcile 后的 `COMPANY_CONTROLLER_FAILED`，也不需要打开 GUI。

若真实 Gate 暴露新的持久化或调度 blocker，继续按同一 RED→GREEN 流程修复；不得用 terminal exception 或自然语言绕过。

## 5. 真实 CLI 结果

当前 binary 已用 exact GUI task/session 通过共享 Turn claim 接管同一 operation：旧 Turn 5 被标记为
`interrupted`，Turn 6 为 `running`。safe-reconciled child 继续使用 WorkItem
`24d552d5-6986-5a48-b17a-6119924e23fc`、WorkerRun
`8fbe6711-f121-4183-8be3-a7663f37792d` 和 message chain
`121d81ed-e78b-42b5-a2c7-ce8990f708f0`，仅把 attempt epoch 推进到 2；child 完成后，Controller
也继续使用原 WorkerRun `cde3c089-510c-431c-a083-f0dd7e3eab54` 和原 chain
`c52509e3-b469-4ffc-90b3-e075ec89c9d6`，推进到 epoch 3。

第一次 Gate BLOCK 的四个 service fingerprint cell 已由同一 Controller repair，写入 evidence
`28715/28718/28721/28724`。第二次 Gate 的 authoritative projection 为 70/72，仅余三个 exact Web
origin（两个 denominator cell）：

- `http://123.6.40.244:8088`；
- `http://150.138.234.105:8000`；
- `https://150.138.234.105:8443`。

这证明原始的 leader-none child-drain、same-worker budget 与 CLI/GUI Turn claim 缺口均已跨过；当前失败不再是
`COMPANY_CONTROLLER_FAILED`，而是 repair 后的历史 seed/reopen 合同。

## 6. 续跑后暴露的两个后续根因

第一处是 seed replay 把 immutable WorkItem 的历史 `dispatch_epoch=0` 与已经推进到 1 的 mutable Team Plan
做相等比较，产生 `stage_team_work_item_replay_mismatch`。正确合同是 WorkItem 保留创建时 epoch；历史静态 item
允许 `item.dispatch_epoch <= plan.dispatch_epoch`，而动态 request/gate-repair item 必须分别回查其 sealed authority。
该修复已有 RED/GREEN DB integration，并未改写历史行。

第二处是普通 Company Controller Gate BLOCK 没有像 operator-recovery 一样关闭当前 model request，外层 Agent
会继续尝试 `submit_stage_deliverable`。普通 gap 现在也返回 closed
`runtime_control=halt_current_request/company_controller_blocked`，同 batch 后续工具被 synthetic block，ToolResult
落盘后结束当前 request；下一条独立 continuation 才获得新 Turn。

## 7. schema 硬边界与已落地 migration

当前历史 row 已是 `plan.dispatch_epoch=1`、requests closed、leader WorkItem `superseded`、Controller/Unit
`gate_blocked`。现有数据库合同同时阻止 successor Turn reopen：

1. `enforce_stage_team_plan_contract` 只有绑定新 `stage_team_repair_generations` 的 repair advance 才允许 reopen，
   否则抛 `STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN`；
2. `enforce_stage_work_item_contract` 不允许 terminal `superseded` 回到 runnable；
3. `stage_team_unit_gaps.source_aggregator_worker_run_id` 为 `UNIQUE`，同一稳定 Controller 只能成为一条 durable
   gap source；这也正是 frozen `max_controller_gate_repairs=1` 的 schema 原因。

因此 repo/runtime 不能安全绕过。用户明确同意迁移后，已落地以下最小向前兼容方案：

- 新增 operation-Turn-bound 的 Controller resume authority，冻结 plan/unit/leader/worker、from/to epoch、checkpoint
  与 open/successor Turn；
- 仅在该 authority 精确存在时允许 plan 递增 epoch、重新开放 request，并允许同一个 stable leader 从
  `superseded` 回到 runnable；
- 去掉 gap source WorkerRun 的全局唯一性，改成能保留每轮 immutable gap 的组合唯一/索引；
- 同一事务恢复原 plan/unit/leader/WorkerRun/chain，不创建 replacement Agent，不清除历史 evidence，不降低 Gate；
- 继续受 frozen repair fuel 和 Turn CAS 约束，并为 response-loss exact replay 添加 integration test。

实现位于 `20260716000002_stage_team_controller_turn_resume.sql` 与
`runtime_memory_tx::resume_company_controllers_for_successor_turn_in_transaction`。authority 采用 `building -> applied` 提交合同，冻结
prior/successor Turn、plan/unit/leader/WorkerRun/chain、submission/gap/checkpoint/hash/Gate witness；deferred trigger 拒绝未消费的
半成品 authority。迁移前确实没有 gap 的历史 fuel-exhausted Controller 只允许使用 migration-time immutable checkpoint-hash
witness，迁移后不能制造同类例外。

真实 operation 在 Turn 7 复用 stage execution `346b4899-9a9d-4268-afdc-d611fd5aa091`、Controller WorkerRun
`cde3c089-510c-431c-a083-f0dd7e3eab54` 与 chain `c52509e3-b469-4ffc-90b3-e075ec89c9d6`，只处理剩余三个 exact
Web origins，最终 authoritative EAS Gate 为 `PASS`。CLI 退出码为 0，未创建 replacement Task/Operation/Controller。
