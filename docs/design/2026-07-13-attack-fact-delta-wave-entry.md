# Attack FactDelta Wave Entry 与全局收敛补充设计

- **日期**：2026-07-13
- **状态**：Accepted for implementation
- **适用范围**：`runtime-memory-candidate-pipeline-v2-2026-07-12` 的 Task 10–12
- **补充关系**：细化 `2026-07-12-runtime-memory-candidate-pipeline-v2.md` §6；不改写已经冻结的 `20260712000004_attack_execution_v2.sql`

## 1. 问题

首波 Candidate Wave 由每个组织的 final-passed `vuln_triage` handoff 进入，因此 `attack_wave_units` 当前把 `entry_stage_execution_id`、`entry_stage_run_unit_id`、`entry_deliverable_submission_id` 与 `entry_stage_kind='vuln_triage'` 全部设为必填。

后续 Wave 的唯一合法来源却是当前 Wave 收敛事务接受的 FactDelta 集。若继续复用旧形状，只能伪造一个不存在的 `vuln_triage` handoff，或丢失 FactDelta → WaveUnit 的 typed provenance。两种做法都违反 evidence-first 与确定性 Gate 不变量。

## 2. 最终决策

### 2.1 00004 冻结，00012 只做 additive forward migration

新增 `20260712000012_attack_fact_delta_wave_entry.sql`。它不修改历史 migration 文件，也不推进 deployment rollout singleton；`00002` 与 `00005` 仍由各自 shadow/cutover 门禁决定。

`attack_wave_units` 变为一个由空值形状判别的 sealed union：

| entry 形态 | 旧 handoff 四元组 | `entry_consolidation_id` |
|---|---:|---:|
| 首波 `vuln_triage_handoff` | 全部非空，kind 必须为 `vuln_triage` | NULL |
| 后续 `fact_delta_consolidation` | 全部 NULL | 非空 |

旧行原样满足第一分支。新触发器只对第一分支验证 final-passed StageRunUnit + immutable handoff；第二分支验证 consolidation 与目标 Wave、operation、frozen scope 完全一致。禁止两套 entry 同时存在，也禁止两套 entry 同时为空。

### 2.2 accepted 不是一个可随意写入的状态

`attack_fact_deltas.status` 继续作为快速读模型，但 `accepted` 的权威凭据是新的不可变 `attack_fact_delta_decisions` 行。每个 proposal 至多一个决定，决定冻结：

- exact source Attempt/Candidate/Wave/Unit/org spine；
- `accepted | rejected` disposition 与稳定 reason code；
- canonical ref kind/id/version、proposal hash 与重新解析出的 hash；
- exact `fact_delta` evidence 集 hash；
- canonical decision hash 与决定时间。

生产代码只能在短事务内先锁 proposal，确认 terminal source Attempt、exact Attempt evidence role、frozen scope/org、closed canonical catalog 的 version/hash 后，插入不可变决定并 CAS 更新 materialized status。没有 accepted decision 的 `status='accepted'` 行不能进入下一 Wave。

V1 只接受 UUID 可寻址的 closed canonical kinds：`target`、`target_asset`、`api_endpoint`、`directory_entry`、`js_analysis_result`、`fingerprint`、`attack_candidate_work_item`、`finding`；它们的 projection version 固定为 `1`。组合键 kind 与未知 kind fail closed，直到另一个 additive schema 为它们提供 typed key。

### 2.3 一个不可变 consolidation 同时充当 FactDelta set header 与全局游标决定

新增 `attack_wave_consolidations`，每个 source Wave 恰好一行，`decision_kind` 为：

- `closed_no_delta`：没有可消费 accepted delta；
- `opened_next_wave`：存在 accepted delta 且 fuel 允许，绑定 exact target Wave；
- `exhausted`：存在 accepted delta，但至少一个持久化 cap 阻止下一 Wave。

该行冻结 source/target Wave、source/target generation、policy hash、FactDelta set hash/count、全 operation 的 wave/candidate/chain/attempt counters、稳定 reason code、decision hash 与 terminal 时间。`source_wave_run_id` 和非空 `target_wave_run_id` 都唯一，因此它是 operation-level 单游标；模型、deliverable 和 process-local `chain_wave_seen` 都不能推进它。

新增不可变 `attack_wave_consolidation_members`，按稳定 ordinal 冻结每个 accepted FactDelta 的：

- source FactDelta/WaveUnit/org；
- `opened_next_wave` 时的 target WaveUnit 与 exact work item；
- `exhausted` 时的 exact residual-risk row；
- member hash。

父行的 `fact_delta_count` 与 member 数量由 deferred constraint trigger 在事务提交前核对。member 的复合外键同时验证 operation/scope/org，不能靠应用层字符串拼接 provenance。

### 2.4 follow-on Wave 对 frozen scope 的每个组织都有 Unit

打开下一 Wave 时，事务按 `operation_org_scope_units.ordinal` 为 frozen scope 的每个组织创建一个 deterministic WaveUnit：

- 本组织有 accepted delta：创建一条 delta-backed seed/work item，冻结非空 manifest，Unit 保持 `open`；
- 本组织没有 delta：仍创建 Unit，但立即置为 `terminal`，`review_closed=true`、`verification_closed=true`、`consolidation_status='terminal'`，且不伪造空 manifest。

这样全局 barrier 不会丢 sibling org，也不会把“没有本波输入”冒充“尚未检查”。下一轮调度只运行非终态 Unit。

### 2.5 唯一写路径是一个短事务

`consolidate_attack_wave` 的锁序固定为：operation → source Wave → source WaveUnits（scope ordinal）→ proposals/decisions → target Wave/Units → manifests/members/residuals → consolidation parent。

事务必须依次完成：

1. 确认 operation 的 runtime/attack contract 均允许 V2，scope snapshot sealed。
2. 确认 source Wave 是当前未收敛 Wave，且 frozen scope 每个 org 恰好一个 Unit。
3. 确认所有 Unit 已 `review_closed`、`verification_closed`、`consolidation_status='ready'`，没有非终态 approved Candidate/Attempt。
4. 对本 source Wave 的每个 proposal 写 immutable accepted/rejected decision；拒绝 sibling、stale、unknown、hash drift 或 evidence mismatch。
5. 只选择带 accepted decision、尚未 consumed、属于本 source Wave 的 FactDelta，并按 scope ordinal + FactDelta id 稳定排序。
6. 从 source Wave 的持久化 policy 列计算 fuel；不得信 caller 传入的 cap。
7. 原子执行 `closed_no_delta`、`opened_next_wave` 或 `exhausted` 分支。
8. terminalize source Units/Wave，写唯一 consolidation；提交前所有 FK、count 与 immutable trigger 必须通过。

精确 replay 读取既有 consolidation 和 members，不再重复消费 delta、创建 Wave、work item 或 residual。任何 identity/hash 漂移都稳定报错。

### 2.6 fuel 与 residual disclosure

沿用零基 generation，但 `max_waves` 表示 Wave **总数**，因此 generation 0 已经消耗第一个 Wave：当 `source_generation + 1 >= max_waves` 时不得再打开 follow-on Wave。`max_chain_depth` 仍约束下一代深度，只有 `next_generation = source_generation + 1 > max_chain_depth` 才停止。全 operation 的 V2 Candidate/Attempt 计数达到 `max_candidates_total` / `max_attempts_total` 时同样停止。

`exhausted` 不消费 FactDelta。每个尚未消费的 accepted member 创建一条 deterministic `attack_residual_risks`，复制 frozen target identity、policy hash、四类 counters，并复用 exact FactDelta evidence，reason code 取第一个稳定 cap 优先级：`max_waves`、`max_candidates_total`、`max_chain_depth`、`max_attempts_total`。Reporting 可据此披露未执行方向，而不能把它显示成 verified Finding。

## 3. 可观察性与敏感字段

`scripts/run_tree.py --full --db` 增加稳定 marker，输出 rollout contract、Wave/cap/hash、per-org barrier/count、Attempt/Worker/lane ownership、FactDelta decision/consumption/consolidation provenance 与 residual disclosure。

诊断禁止输出 raw lease token、worker checkpoint JSON、execution plan、budget、result JSON、canonical args/action outcome、evidence body 或 exploit payload；只输出 id、状态、hash、count、版本、布尔一致性和时间。

## 4. 失败语义

| 场景 | 结果 |
|---|---|
| sibling org / foreign scope canonical ref | proposal 写 immutable rejected decision；不得开 Wave |
| canonical row 不存在、version 非 1、hash 漂移 | rejected；不得消费 |
| accepted status 缺 immutable decision | BLOCK / transaction rollback |
| 任一 frozen org 未 terminal-ready | BLOCK；游标不推进 |
| target Wave/unit/member/work-item 任一写失败 | 整个事务 rollback |
| response loss 后 exact replay | 返回原 consolidation DAG，`replayed=true` |
| cap 命中 | source Wave exhausted + residual rows；delta 保持 accepted/unconsumed |

## 5. 非目标

- 本补丁不执行真实扫描、exploit、外部 API、embedding 或 Graphiti。
- 不把 memory/KG/prose 当作 canonical FactDelta truth。
- 不在 00012 推进 rollout default，也不改变既有 operation 的 frozen contract。
- 不为组合键 canonical kind 猜造 UUID。
