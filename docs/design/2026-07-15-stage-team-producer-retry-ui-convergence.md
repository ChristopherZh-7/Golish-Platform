# Stage Team producer retry 与 UI 收口

> 状态：Implemented with focused validation（2026-07-15）；fresh live rerun / full precommit pending

## 问题

真实运行 `pentest-chat-1784102030587-1` 的 `target_intel` Team 中，六个 producer
进程都结束，但 WHOIS 返回了带前后说明的 fenced JSON，ASN 返回
`checked_empty + checked_empty_units=[]`。两者都被直接固化成
`STAGE_TEAM_WORKER_OUTPUT_INVALID`，随后整个 Unit 被置为 `gate_blocked`；同一请求再次
调用 `stage_run` 只能得到 `stage_team_unit_not_runnable`。

同一详情页还同时渲染 legacy org event card 与 DB-backed Team read model，并默认展开
plan hash、epoch、lease、schema、chain 等内部字段，导致同一运行状态被重复且冲突地表达。

## 决策

### Producer 输出

1. 协议/格式校验失败不是业务 `blocked` 输出，不能在第一次尝试时写入 immutable
   `StageWorkerOutput`。
2. 校验失败复用现有 `retry_stage_worker` compound API：当前 Worker attempt 失败，稳定
   WorkItem 在冻结预算内重新入队；只有 attempt 耗尽时由 repository 写确定性 blocked output。
3. 仅允许从回复中提取唯一一个 fenced JSON object；多个 fence、多个对象或边界不明确继续
   fail closed。自动附加且 chain id 匹配的 session marker 仍可剥离。
4. `checked_empty` 必须同时带非空 `checked_empty_units` 与真实 evidence；prompt 与 validator
   保持一致，不由 runtime 猜测或伪造空检查单元。
5. `no_registrable_domain` 是已登记的 dependency-not-ready blocker：在 attempt 预算内先重试，
   让并发的 domain-discovery sibling 有机会落 canonical domain。其他未知 blocker 仍按真实业务
   blocked 处理，不扩大自动重试范围。
6. 已落的 immutable output 不更新、不删除。旧 BLOCK run 只能通过 fresh Stage execution/Team
   generation 继续，历史失败保持可审计。

### 前端

1. `StageRunOrgRows` 变成 legacy/Team 路由器：存在一致的 exact
   `operation_id + stage_execution_id` 时只渲染 `StageTeamRunView`；否则保留 legacy org cards。
2. Team 默认视图只显示组织、producer 有效/运行/阻塞计数、每个轴的业务状态、Gate/恢复提醒。
3. 展示状态优先级为 immutable output disposition > WorkItem lifecycle > Worker lifecycle。
   `Worker passed + output blocked` 必须显示为“输出无效/阻塞”，不能显示绿色完成。
4. hash、epoch、lease、schema、chain、request 等放入显式“调度详情”折叠区。
5. Team 模式保留一个轻量“查看 Agent 运行流”入口，不再保留完整 legacy card。

## 验收

- 带外层说明但只有一个 fenced JSON object 的 producer 回复可被确定性解析。
- invalid `checked_empty` 不落完成输出，走有界 retry；预算耗尽才形成 terminal blocked output。
- `no_registrable_domain` 走 dependency retry；未知业务 blocker 不被静默改写。
- exact Team pointer 与 legacy card 永不同时显示。
- Team 默认视图不出现 hash/epoch/lease/schema/chain，展开调度详情后仍可审计。
- blocked output 覆盖 Worker `passed` 的视觉状态。
