# Candidate V2 Operation 级 Fuel Reservation 实现计划

**目标：** 把 Candidate/Attempt fuel 从 consolidation 的事后观察改为 operation-scoped、DB-enforced hard cap，同时保证 cap 到达后 Verification 仍能终态收敛。

**架构：** 以 Candidate/Attempt/approved-with-zero-Attempt 行作为派生 fuel ledger；repository 在固定父锁下预检，additive `00015` deferred trigger 独立复算。first claim 转换 reservation，retry 仅消费空闲 slot，最后一个失败 Attempt 原子 blocked 并生成 residual。

---

## Task 1：先写 DB RED

**文件：** `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`

覆盖：

1. cap-1 时提交两条 Candidate，整批失败且无部分 work-item/Candidate 写；
2. 两个 org 顺序及并发 final seal 的合计数不越界；
3. raw Candidate/Attempt INSERT 与 approval reservation 旁路在 commit 被 deferred trigger 拒绝；
4. exact replay 不重复占槽。

每次 Cargo 测试前运行 `just space-guard`，记录 RED run id。

## Task 2：实现 Candidate 与 review reservation

**文件：**

- `backend/crates/golish-db/migrations/20260712000015_attack_fuel_reservation.sql`
- `backend/crates/golish-db/src/repo/attack_candidates.rs`
- `backend/crates/golish-db/src/repo/attack_candidate_approvals.rs`

final seal 锁 operation/Wave 后预检全 operation Candidate count。Review 在 mutation 前计算提交后 approved-with-zero-Attempt reservation；失败无 approval、Candidate disposition 或 barrier 部分写。migration deferred trigger 在 commit 重算 Candidate hard cap 与 effective Attempt fuel。

## Task 3：实现 claim/release 终态收敛

**文件：**

- `backend/crates/golish-db/src/repo/candidate_attempts.rs`
- 必要时 `backend/crates/golish-db/src/repo/finding_lineage.rs`

claim 优先无 Attempt Candidate；first claim 只转换 reservation，retry 需空闲 slot。release 若无 retry slot，在同事务写 blocked Attempt/Candidate、固定 hash/reason、lane release、reportable residual 与 exact evidence membership；否则保持 retryable_failed。补顺序、并发、response-loss 与 residual replay tests。

## Task 4：集成与文档

同步 `golish-db` / repo / Candidate harness 模块卡、INDEX、设计引用、`agent-progress.md` 与 feature evidence。先跑 focused DB tests，再跑整包 `attack_execution_v2_migrations`、scoped Clippy，最终由主任务执行 full `just precommit`。
