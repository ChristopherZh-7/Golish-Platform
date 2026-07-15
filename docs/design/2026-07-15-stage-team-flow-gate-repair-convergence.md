# Stage Team 运行流、Gate truth 与 Repair 收敛设计

## 1. 现场问题

`target_intel` Team UI 当前把组织级 `::team::<org>` progress pointer 同时当作
Agent 运行流 identity。Producer 实际运行在
`::team::<org>::worker:<worker_run_id>` 下，所以采集阶段点击“查看 Agent 运行流”找不到
Agent；Barrier 闭合后 Aggregator 才使用无后缀 identity，页面于是看起来像“外面已经查完，
里面才开始运行”。

同一现场还暴露两个 authority 错位：

1. UI 把 Producer 自报的 `found/checked_empty` 计为“有效/已发现/已查空”，即使 Unit 尚未
   通过 Gate；
2. Aggregator 的首次 `submit_stage_deliverable needs_fix` 已经产生 durable submission，
   但通用 SubAgent loop 仍把 BLOCK 喂回同一个 Aggregator，连续三次后才以 generic failure
   返回，外层 `stage_run` 因此无法进入既有 repair-generation 分支。

## 2. 不变量

- Team progress pointer 只负责把事件绑定到 exact `stage_run` tool；它不是 Agent identity。
- 每个 Producer 和 Aggregator WorkerRun 都有唯一 UI parent request id。
- Producer output 是“Worker 已返回”的 durable manifest；只有 authoritative coverage/Gate
  才能证明 `found/checked_empty`。
- 一个 Aggregator 只提交一次、Gate 评估一次。BLOCK 必须终结旧 Aggregator并打开有界 repair
  generation；不得在原 WorkerRun 内循环修补。
- 不新增 DB schema/migration，不修改 generated IPC types。

## 3. 运行流 identity

- Producer：`<team_pointer>::worker:<worker_run_id>`（保留现状）。
- Aggregator：新执行改为 `<team_pointer>::aggregator:<worker_run_id>`。
- `stage_run_org_progress.agent_request_id` 继续携带无后缀 `<team_pointer>`，用于 UI read-model
  路由，不再被当成唯一 Agent。
- 前端从 session 的 `activeSubAgents` 解析 WorkerRun 后缀，并把每个 WorkItem/Worker 的“运行流”
  按钮接到 exact parent request id。旧 session 的无后缀 Aggregator 仍以 Team pointer 兜底。

## 4. UI 语义

- 顶部计数改为“采集 N/M 已返回”，不再叫“有效”。
- Unit 未 final-seal 时，Producer `found/checked_empty` 只显示“已返回，待 Gate”；Unit 通过后
  才显示“已发现/已查空”。
- Aggregator 单独成行，显示“等待校验 / 校验中 / Repair 中 / 已通过 / 已阻塞”。
- Stage 总状态单独显示，只有 `Unit passed + final_handoff_id` 才是“阶段已通过”。
- Producer 与 Aggregator 的运行流入口都留在 Team 卡内，不恢复 legacy 重复卡。

## 5. Producer authority fence

`target_intel` Producer 解析出 `found/checked_empty` 后、写入 immutable output 前，runtime 读取
同一 operation/org/stage/session 的 authoritative coverage snapshot，并按 WorkItem axis 映射到
exact `GOLISH-INTEL-*` technique：

- `found`：该 technique 至少一格 authoritative `found`，且所有适用格均已到 terminal state；
- `checked_empty`：至少一格 authoritative `checked_empty`、没有 `found`，且所有适用格均已
  terminal；
- `blocked` 保留为显式业务 blocker，不伪装成功。

不匹配属于可重试 Producer contract violation，复用 frozen attempt budget；snapshot 读取失败则
fail closed，不写错误的 immutable output。

## 6. Aggregator repair handoff

给内部 `BoundWorkerChainContext` 增加 host-owned “stage submission is terminal” 标志，仅
Stage Team Aggregator 设为 true。通用 SubAgent executor 看到该 Worker 的
`submit_stage_deliverable` 返回 `accepted` 或带 durable submission id 的 `needs_fix` 时，在完整
checkpoint 后立即结束 loop并把 submission id 交回 scheduler。

Scheduler 随后加载 exact submission、运行 authoritative Gate：PASS final-seal；BLOCK 调现有
`open_stage_team_repair`，旧 Aggregator `gate_blocked`，新 repair Producer + fresh Aggregator 在同一
`stage_run` 内继续。没有 durable submission id 的拒绝仍是明确 Aggregator failure，不能伪造 repair。

## 7. 兼容与验证

- `stageRunRequestIdFromAgentRequestId` 同时识别 `::org::` 与 `::team::`，避免 Team retry progress
  写进旧 tool card。
- 旧 session 的错误 immutable output不原地修改；新执行通过 authority fence防止再次产生。
- 前端用 Vitest 覆盖 producer/aggregator drill-in、Stage/Gate 文案和 Team request parser。
- Rust 先写纯函数/dispatcher RED 测试，再验证 Producer snapshot fence与 Aggregator首个
  `needs_fix` terminal handoff。
