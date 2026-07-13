# Candidate V2 Shadow-Gated Rollout Promotion 补充设计

- **日期**：2026-07-13
- **状态**：Implemented；scoped verification 与 2026-07-14 final `just precommit` 均已通过，live acceptance 仍待授权 workspace/scope
- **适用范围**：attack execution deployment singleton、Candidate final-seal shadow attestation、新 operation contract freeze
- **修正关系**：取代 `20260712000005_attack_execution_v2_cutover.sql` 直接连跳到 `v2_only` 的实现方式

## 1. 问题与决策

`00005` 的 migration 顺序早于 retained shadow schema `00013`。migration 若直接执行
`legacy -> dual_write_read_legacy -> dual_write_read_v2_fallback -> v2_only`，就不可能读取
任何持久化 sample，等价于绕过 promotion gate。

因此 `00005` 只做唯一无需历史 sample 的相邻转换：

```text
legacy -> dual_write_read_legacy
```

它只启用 whole-record dual-write/read-legacy 采样，不把新 operation 提前切到 V2 read 或
V2Only。后续两级 promotion 不能扫描“碰巧存在”的 shadow rows，而必须以 `00016` 的
Candidate admission cohort 为分母：自行重建 canonical V2 record，并确认每个 materialized
非零输入 WaveUnit 的 sealed positive manifest、唯一 final-passed Candidate Unit 与 shadow
全部一一闭合后才允许相邻 CAS。

## 2. Candidate admission 与 cutoff

首次插入 generation-zero `attack_wave_units` 时，DB trigger 在 rollout singleton 上先取
`FOR SHARE`，再写一条 operation-scoped admission：`operation/scope/initial Wave/first WaveUnit/
contract/rank/row-version/admission_seq`。`admission_seq` 是 DB 自增单调序列，不接受 caller
提供。WaveRun 与 WaveUnit 的触发器都先取 rollout lock，因此 promotion 持有 `FOR UPDATE`
后不会再有旧 contract operation 穿过 cutoff。

- operation 从未进入 Candidate Wave：没有 admission，不进入本轮分母；
- 已 admission operation：可在同一 frozen contract 下完成后续 Wave；
- rollout 已提升后才尝试首次 admission：拒绝为 stale contract，必须由上层重启/替换为
  当前 contract operation；
- 每次只推进一个 rank；rank 2 需要 rank 2 自己的新 cohort/sample，不能复用 rank 1 receipt。

promotion 在 rollout `FOR UPDATE` 下冻结当前 contract/rank 的 `MAX(admission_seq)`。DB 从每条
admission 扩到该 operation/scope 下全部 materialized WaveUnit。promotion 证明的是 Candidate
final-seal shadow domain，而不是稍后的 human review / verifier / consolidation 生命周期，因此不以
Wave 或 WaveUnit 的 terminal 状态作为分母门槛：每个非零输入 WaveUnit 必须已有 nonblank
`manifest_hash`、positive `manifest_count`、`manifest_frozen_at`，并且同 generation/org 只能有一个
final-passed Candidate Unit，再到 exact closed matching shadow。open 但已完成 Candidate final seal
的 WaveUnit 可以成为 promotion sample；open/unsealed、Candidate Unit 缺失或重复、shadow 缺失/
未关闭/mismatch 任一都返回 not-ready。

唯一不进入 Candidate 分母的是 exact follow-on zero-input WaveUnit：必须有
`entry_consolidation_id`，WaveUnit 自身 status/review/verification/consolidation 全部 terminal，manifest
三元组全为 `NULL`，且不存在 Candidate StageRunUnit。任一字段不满足就不是“空输入”的证明，继续
按普通 WaveUnit fail closed。成功后 trigger 内生成 immutable promotion receipt，receipt 自身的
nested INSERT trigger 再从 DB 重算 cutoff/counts，外部不能伪造。

admission 与 promotion receipt 都是 DB-owned retained authority。admission 只允许 admission trigger
嵌套 INSERT，任何 direct/nested UPDATE 或 DELETE 都拒绝。shadow sample 在 owning Unit/operation
仍存在时同样拒绝 direct/nested DELETE；只有父记录已经删除的真实 FK cascade 可以清理。pending
shadow close 的 `compared_at/updated_at` 由 DB 写入当前时间，caller 不能伪造 chronology；closed
row 继续整条 immutable。

## 3. 事务与锁序

Candidate final seal、legacy mirror、comparison/source/hash attestation 在原业务事务内原子
commit。promotion **不得**加入该事务：finalizer 已持 shadow row，再锁 rollout 并扫描全部
sample，会与另一 finalizer 形成 `shadow A -> rollout -> shadow B` / `shadow B -> rollout`
死锁。

业务 commit 后另起 promotion transaction，固定顺序：

```text
rollout singleton FOR UPDATE
  -> admitted cohort/cutoff
  -> materialized WaveUnit/sealed Candidate manifest/final-passed Candidate Unit
  -> shadow rows ORDER BY operation/unit
```

not-ready 是正常 `unchanged` 结果，不通过 SQL error 假装忽略；malformed storage 才返回
error。promotion 失败不得反向否定已提交的 Candidate final seal。创建 operation 前也先用独立
best-effort transaction reconcile，随后才在新 transaction 冻结 runtime/attack contract。

## 4. DB 自验证与合同兼容

`00016` 替换 rollout transition trigger：raw adjacent `UPDATE` 仍必须通过同一 admission/Wave/
Candidate/shadow gate。SQL 递归 canonicalize JSONB，并从 `attack_candidate_work_items`、
`attack_candidates` 与 evidence membership 重建 semantic hashes/whole record；不信 caller counts、
stored comparison 或 selected hash。Rust reconciler 再独立 rehydrate 相同 Candidate rows，形成双层
校验。

operation contract 矩阵同时收紧：dual attack writer 拒绝 `runtime_memory_contract=legacy_v1`；
attack `v2_only` 仍只允许 runtime `v2_only`。现有 operation 的两份 frozen contract 不原地改写。

## 5. Crash / response-loss 收敛

- final seal commit 后进程崩溃：sample 已持久，未丢 promotion 条件；
- finalize response-loss replay：不会重复 sample，post-commit reconciler 可再次执行 adjacent CAS；
- 创建新 operation 前再执行一次 best-effort reconciler，确保已有 ready sample 不因上次崩溃永久滞留；
- concurrent reconciler 依靠 rollout row-version 与相邻 rank 串行；第二个调用重读新 rank，不能跳级；
- mismatch/incomplete retained sample 保持当前 rank，不能拼 legacy/V2 字段或删除坏样本来放行。

## 6. 可观测结果

reconciler 返回 `promoted | unchanged_not_ready | already_v2_only` typed outcome；app trace
记录旧/新 contract 与 retained aggregate，不记录 Candidate 原始 payload。没有 operator push、
外部请求或真实 verifier 执行。
