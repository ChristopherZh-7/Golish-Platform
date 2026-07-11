# Enumeration worker capacity continuation

## 问题

Enumeration 的 worklist 每页最多 50 个 exact Web Origin。Test1 当前有 372 个
origin，即 8 页。现场 worker 做完第一页后，明知 `ready_to_submit=false`、仍有 322
个 root，却因单段无法完成全部工作而提前提交。DB gate 正确返回了 1316 个
coverage gaps，但 runtime 把这个“coverage-only 且仍在前进”的结果当成普通 gate
失败，开始消耗 3 次通用 gate retry。

只处理“没有 deliverable”的容量分支仍不够：worker 可以产生一个 slim deliverable，
但该 deliverable 只因当前 worklist 尚未收口而 BLOCK。并且不能假设 worker 会吃满
40 iterations；弱模型可能每完成一页就主动结束，因此 8 页最坏需要 7 次同链工作续段。

## 决策

`stage_run` 对 DB-backed Enumeration worker 在每个 segment 前后读取与 UI/worklist
同源的 `stage_asset_coverage` 全量 snapshot，并在服务端统计
`unfinished = pending + error + partial`：

1. 只有 denominator 非空、worker 成功返回、请求未取消且存在 durable exact chain 时
   才考虑续跑。
2. 两类结果可以进入 capacity 判定：没有 accepted deliverable；或 deliverable 的唯一
   blocker 是 coverage complete，且 `coverage_gap_actions` 的规范化
   `(exact-origin, technique)` 集合与 authoritative snapshot 的完整未终态 cell 集合完全
   相等。只有数量相同但 key 不同、或 compact snapshot 已截断时都 fail closed；混合
   schema/evidence/contract blocker 继续走普通 gate repair。
3. 工作续段必须满足 segment 后 `unfinished` 严格小于 segment 前；没有基线或没有下降
   立即 BLOCK，不用通用 gate retry 重跑同一页。
4. 工作续段预算为 `min(ceil(root_count / 50) - 1, 8)`。372 roots 得到 7 次续段；
   更大 denominator 被每请求 hard cap 8 截断，后续用户 continuation 可继续 exact chain。
5. worklist 已 ready 但 worker 尚未正确 submit 时，另有一次独立 submit-only
   continuation；它不占工作分页预算，也不能重复。
6. capacity continuation 复用同一 exact chain，且不增加 per-org gate attempt；真正的
   非 coverage-only gate BLOCK 才消费原有 deterministic gate retry。
7. denominator 缺失、读取失败、chain 缺失、无进展、预算耗尽或用户取消时进入现有
   exhausted/reentry circuit breaker。

## 不变量

- 续跑必须使用 `operation_state.state_blob.stage_run_workers[stage][org]` 保存的精确
  `sub_agent_session_id`，不能使用 `latest`。
- DB snapshot 仅决定“是否继续做工作”，不授予 PASS；PASS 仍只来自 per-org gate。
- 不扩大 stage、org、target、exact-origin 或工具授权边界。
- continuation 同时受页数和固定 hard cap 约束，并沿用顶层 Stop cancellation flag 与
  `StageRunReentryGuard`。

## 验证

- 372 roots / 50-per-page 回归测试得到 8 页、7 次工作续段，超大 denominator 被 cap 8。
- coverage-only deliverable BLOCK、无 deliverable、strict progress、stall、chain missing、
  mixed blocker、同数量不同 cell key、compact truncation、budget exhausted、独立
  ready-submit-only 分支均有单元测试。
- raw DB coverage snapshot 与 compact worklist snapshot 的 cell 统计口径一致。
