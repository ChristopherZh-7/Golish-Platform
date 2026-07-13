# Candidate V2 Operation 级 Fuel Reservation 补充设计

- **日期**：2026-07-13
- **状态**：Accepted for implementation
- **适用范围**：Candidate fresh final seal、human review、CandidateAttempt claim/retry、fuel residual
- **补充关系**：补齐 `2026-07-12-runtime-memory-candidate-pipeline-v2.md` 与 `2026-07-13-attack-fact-delta-wave-entry.md` 的 hard-cap 写入语义

## 1. 问题

现有 `max_candidates_total` / `max_attempts_total` 只在 Wave consolidation 后统计。单个 org 的 manifest 有界不等于整个 operation 有界；多 org final seal 或 Attempt retry 可以在 consolidation 看到之前超过冻结 policy。只在 claim 时返回错误还会留下 approved Candidate 永久等待，Verification 无法收敛。

## 2. 持久化 fuel 模型

不维护可漂移的自增计数器。数据库权威行本身就是 fuel ledger：

- 每条 V2 `attack_candidates` 消耗一个 Candidate slot；rejected/blocked 也保留历史，因此不返还。
- 每条 `candidate_attempts` 消耗一个 Attempt slot。
- `disposition='approved'` 且尚无任何 Attempt 的 Candidate 是一个持久化 first-Attempt reservation。
- `effective_attempt_fuel = Attempt 行数 + approved-with-zero-Attempt 数`。

operation 的 frozen `operation_state` 行与 exact Wave 行按固定顺序加锁。Candidate final seal、review 与 claim 因而在多 org 并发下共享同一个串行化点。additive `20260712000015_attack_fuel_reservation.sql` 再用 deferred constraint trigger 重算相同集合，阻止旁路 repository 的 raw INSERT/UPDATE 在 commit 时超限。

## 3. 写入语义

### 3.1 Candidate final seal

fresh 写入前重载 operation 全量 Candidate count，要求 `current + fresh_batch <= max_candidates_total`。整批越界则整笔 final-seal transaction rollback；exact response-loss replay 不新增行、不再次占槽。

### 3.2 Review reservation

批准前按整批 operator decision 计算提交后的 `effective_attempt_fuel`。若 first-Attempt reservation 会超过 `max_attempts_total`，整批 review 拒绝且不产生部分 approval。rejected Candidate 不占 Attempt reservation；approval expiry 在尚未创建 Attempt 时把 Candidate 重开为 proposed，自然释放派生 reservation。

### 3.3 Attempt claim 与 retry

first claim 把一条 approved-with-zero-Attempt reservation 原子转换为真实 Attempt，effective fuel 不变。retry 会新增 Attempt，只在 `effective_attempt_fuel < max_attempts_total` 时允许。claim 优先选择尚无任何 Attempt 的 approved Candidate，避免 retry 抢走已经为 sibling Candidate 保留的 first-attempt slot。

worker release 时若当前 effective fuel 已到 cap，则本 Attempt 不再写 `retryable_failed`：同一事务把它和 Candidate 终态化为 `blocked`，reason code 固定为 `max_attempts_total`，释放 lane，并创建 evidence-backed residual risk。该路径不伪造 proof/refutation，也不创建额外 Attempt；Candidate support evidence 只作为 residual disclosure provenance。还有 retry slot 时保持既有 `retryable_failed` 历史语义。

## 4. 失败与重放

| 场景 | 结果 |
|---|---|
| 多 org Candidate batch 合计越 cap | 后提交事务整体 rollback，无部分 Candidate/work-item decision |
| review 批次无法为每个 approved Candidate 保留首 Attempt | review 整批拒绝，barrier 不假装关闭 |
| first claim | reservation → Attempt，effective fuel 不变 |
| retry 有剩余 slot | 新 Attempt ordinal |
| retry 已无 slot | 当前 Attempt/Candidate blocked + residual，Verification 可确定性收敛 |
| exact final-seal/claim response-loss replay | 返回既有行，不重复消费 |
| raw SQL 绕过 repository | deferred hard-cap constraint 在 commit 拒绝 |

## 5. 非目标

- 不动态修改 operation-frozen policy。
- 不让模型自报 fuel counters 或 residual。
- 不执行真实 scan/exploit/外部请求。
- 不删除历史 Candidate/Attempt 来“返还”fuel。
