# Candidate V2 Whole-Record Shadow Attestation 补充设计

- **日期**：2026-07-13
- **状态**：Accepted for implementation
- **适用范围**：`runtime-memory-candidate-pipeline-v2-2026-07-12` 的 Candidate Task 12 rollout
- **补充关系**：细化 `2026-07-12-candidate-verification-pipeline-v2-corrected.md` §0.1 与 Task 12；不改写已冻结 migration，也不改变既有 operation 的 contract

## 1. 问题

Candidate V2 的 rollout contract 已定义四个相邻阶段，但原有实现只有纯函数 selector 和 fixture migration，缺少生产运行时可重建的比较证据。若 comparison 只存在进程内，deployment default 无法证明自己是根据真实 final-seal 样本推进；若 V2 缺 child 时逐字段回退，又会把两个不完整来源拼成看似完整的 Gate truth。

`operation_state.state_blob` 也不能承担 legacy mirror。运行期记忆进入 `v2_only` 后不得继续写 legacy checkpoint document，否则 V2Only 仍隐式依赖旧状态。

## 2. 最终决策

### 2.1 final seal 同事务写独立 legacy semantic mirror

新增 additive migration `20260712000013_attack_execution_shadow_reads.sql` 与专用 repository。对 `dual_write_read_legacy`、`dual_write_read_v2_fallback` operation，`attack_candidate` final Gate PASS 的同一短事务完成：

1. 写 V2 Candidate / no-Candidate 权威行；
2. repository 从 typed server command 与 exact persisted manifest 构造完整 legacy semantic record，caller 不能提交任意 legacy JSON；
3. 以 NULL attestation 输入插入 `attack_execution_shadow_reads`，DB `BEFORE INSERT` owner 重建 canonical V2 whole record，并原子派生 comparison、selected source/hash 与时间戳；
4. final StageRunUnit、StageHandoff、两个语义来源与已关闭 attestation 一起 commit，不产生可观察的 pending row。

mirror 只保存 work-item key、decision kind、完整 decision payload 的 canonical hash 和冻结时 review counts，不保存 execution plan、evidence body 或模型 prose。它不写 `operation_state.state_blob`。

双写事务不得因 V2/legacy semantic mismatch 而 rollback。mismatch 正是 shadow rollout 必须持久化的观测结果；提前拒绝会让 promotion aggregate 永远看不到错误样本。

### 2.2 selector 只能选择整条记录

`golish-agent-kit` 的纯 selector 接收两个封闭值：

- `legacy: Option<CompleteAttackRead>`；
- `v2: Complete | Missing | Incomplete`。

它返回一个不可拆分的 `AttackReadSelection`。四种 contract 的语义固定为：

| contract | 选择 | comparison | V2 verifier |
|---|---|---|---:|
| `legacy` | 完整 legacy | 无 | 否 |
| `dual_write_read_legacy` | 完整 legacy | `match / mismatch / v2_missing` | 否 |
| `dual_write_read_v2_fallback` | 完整 V2；缺失时整条 legacy | `match / mismatch / v2_missing` | 否 |
| `v2_only` | 只允许完整 V2；缺失即 BLOCK | 无 | 是 |

禁止从 legacy 取 decisions、从 V2 取 review count，或反向拼字段。`v2_only` 即使还存在历史 legacy 数据也不得 fallback。

### 2.3 comparison 是一次性、可审计的 DB attestation

生产 adapter 在 final-seal transaction 中只提交 frozen identity 与 repository 构造的完整 legacy record/hash；DB `BEFORE INSERT` owner 随即重载 operation、exact final-passed Candidate Unit 与 canonical V2 relation，验证 legacy 对 exact manifest 的闭合覆盖，再原子派生：

- `comparison`；
- `selected_source`；
- `selected_record_hash`；
- `compared_at`。

因此 attestation 没有 INSERT 后再 UPDATE 的 pending/CAS 阶段。`record_unit_selection_with_connection` 仅把 kit selector 的结果与 DB-owned closed seal 做 exact 验证，不拥有也不改写 comparison/source/hash。attestation 四个字段由数据库原子生成，写入后不可修改；response-loss 只接受 exact source replay。

样本以 `stage_run_unit_id` 为主键，并通过复合 FK 绑定 exact operation、stage execution、organization 与 `stage_kind='attack_candidate'`。organization 是冻结审计身份，不依赖可删除的 live organization FK。

数据库约束与 INSERT owner 一起封住旁路：caller 必须把四个 attestation 字段留空，owner 在同一 INSERT 内关闭它们；`dual_write_read_legacy` 只能选 legacy，fallback 的 `v2_missing` 只能选 legacy_fallback，凡选择 legacy/legacy_fallback 都必须让 selected hash 等于 immutable `legacy_record_hash`。完整 V2 hash 跨 WaveUnit、work item、Candidate 与 evidence 规范化 join，由 INSERT owner 直接重建；raw SQL 仍是 DB-privileged 边界，不是应用写 API。应用写 seam 只负责插入 server-derived legacy source，selector seam 只做 closed-seal verification；promotion aggregate 还会逐样本重新 hydrate canonical V2 record，任何 shape 合法但 durable projection 不一致的样本都被计为 incomplete 并阻止晋级。

### 2.4 rollout promotion 只读持久化 aggregate

`legacy → dual_write_read_legacy` 只是开启采样，不可能要求之前已有 dual 样本。之后每个相邻推进必须在锁住 singleton 的同一事务内逐条重建当前 contract 的持久化样本，而不是只 `COUNT` 自报标签，并满足：

- `sample_count > 0`；
- `mismatch_count = 0`；
- `incomplete_count = 0`；
- expected row version 与相邻 rank CAS 同时成立。

调用者不能提交自报 attestation counts。任一未比较、`v2_missing` 或 mismatch 样本都稳定阻止 promotion，且不改变 singleton row version。

`20260712000005_attack_execution_v2_cutover.sql` 仍是 release-time fixture gate 全绿后创建的部署 migration；生产 repository gate 用于运行中分阶段 promotion。两者都不更新既有 `operation_state.attack_execution_contract`，因此旧 operation 永远保留创建时冻结的 contract。

### 2.5 retained sample 的删除边界

shadow sample 是 deployment 决策证据，不是可按结果筛选的临时 telemetry。legacy source、closed comparison/source/hash 与时间戳均不可改，应用也不能直接删除单条样本。promotion 聚合当前 contract 的全部 retained sample，因此历史 mismatch 会持续阻止晋级，不能通过挑选或逐条清理把失败洗掉。

生命周期只跟随 owner runtime truth：删除 owning `StageRunUnit` 或 operation 时允许复合 FK 的 nested cascade 一并回收 sample。`organization_id` 仅保存 frozen audit identity，不绑定可变 live organization FK；删除/重建组织树不会单独擦除 rollout evidence。

## 3. 失败语义

| 场景 | 结果 |
|---|---|
| dual final seal 缺 legacy mirror | transaction BLOCK / rollback |
| V2 child 缺失 | `v2_missing`；legacy 模式整条 legacy，V2Only BLOCK |
| V2 与 legacy 不等 | 持久化 `mismatch`；按 frozen contract 选择整条来源；禁止 promotion |
| caller 伪造 comparison/source/hash | repository 重算后拒绝 |
| response loss 重放同一 attestation | 返回成功，不重复写 |
| response loss 参数漂移 | 拒绝 replay |
| promotion 无样本或有异常样本 | singleton 不变 |

## 4. 非目标

- 不执行真实 verifier、扫描、exploit 或外部请求。
- 不把 shadow mirror 升格为 Candidate/Finding Gate authority。
- 不允许运行中修改 operation-frozen contract。
- 不在 `v2_only` operation 写 legacy `state_blob`。
