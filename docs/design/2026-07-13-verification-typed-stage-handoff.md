# Verification Server-Authored Typed StageHandoff 补充设计

- **日期**：2026-07-13
- **状态**：Accepted for implementation
- **适用范围**：Candidate V2 Verification terminal close、下游 Access Validation / Reporting inheritance
- **补充关系**：补齐 `2026-07-12-runtime-memory-candidate-pipeline-v2.md` 的 StageHandoff 不变量；不伪造模型 deliverable、tool call 或 worker lease

## 1. 问题

Candidate V2 Verification 由确定性 scheduler 逐条领取 CandidateAttempt，并由 DB terminalizer 写 `verified | refuted | blocked` 真相。全部 Attempt 终态后，逻辑 organization WorkerRun 与 StageRunUnit 需要 PASS，WaveUnit 才能进入 consolidation barrier。

generic `stage_handoffs` 只能引用 `stage_deliverable_submissions`。为 Verification 伪造 submission、tool call 或一次不存在的 primary worker lease，会把服务端聚合 close 冒充模型交付；若只把 StageRunUnit 改为 `passed` 而不发布 handoff，又违反“final PASS 必须有可继承、可追溯交接”的运行期记忆不变量，使 Reporting / Access Validation 看不到 Verification 终态来源。

## 2. 最终决策

### 2.1 dedicated typed handoff 是服务端 final seal

新增 additive migration `20260712000014_verification_stage_handoffs.sql`。一条 `verification_stage_handoffs` 绑定：

- exact operation / frozen scope / organization；
- source Wave / WaveUnit / generation；
- Verification StageRun / StageRunUnit；
- 唯一 logical primary `candidate_verifier` WorkerRun；
- server-built bounded payload、payload hash、truth hash、evidence ids 与 coverage watermark；
- close 后 WaveUnit row version 与 Gate timestamp。

它的 `authority_kind` 固定为 `verification_wave_close`，没有 fake deliverable submission。行写入后不可变；live organization 删除不能改写冻结 identity。

### 2.2 一个事务封口四个权威对象

`close_verification_unit` 按固定锁序重载 exact Wave → WaveUnit → Verification StageRunUnit → primary WorkerRun → Candidate/Attempt/evidence/Finding/FactDelta truth。首次 close 只允许：

- Wave 和 WaveUnit 正处于 verification；
- review 已关闭、consolidation 仍 pending；
- 所有 work item 已决策；
- 每个曾批准 Candidate 恰有合法 terminal Attempt；
- verified 有 proof + exact Finding lineage，refuted 有 refutation，blocked 有 reason/evidence；
- StageRunUnit 与 primary WorkerRun queued，且没有 lease、heartbeat、active tool 或 terminal timestamp。

同一事务随后：

1. 把 WaveUnit 置 `verification_closed=true, consolidation_status='ready'`；
2. 把 logical primary WorkerRun 置 `passed`；
3. 生成 deterministic typed handoff id、payload 与 hash；
4. 插入 immutable typed handoff；
5. 把 StageRunUnit 置 `passed`，`pass_watermark` 绑定 handoff id/hash；
6. 由 deferred constraint 在 commit 前核对四者完全一致。

任一步失败全部 rollback。数据库 constraint 同时禁止绕过 repository，单独把 Unit、Worker 或 WaveUnit 改成 terminal-ready。

### 2.3 payload 只发布终态引用，不复制攻击内容

payload 由服务端从 DB truth 构造，稳定包含：

- Attempt id、Candidate id、plan hash 与 terminal disposition；
- Finding canonical ref（仅 verified）；
- FactDelta id/ref（若存在）；
- evidence id 集、coverage count 与 verification truth hash。

禁止放入 execution plan、raw action args/result、proof body、credential、lease token、checkpoint 或模型总结。下游拿到引用后仍须回查 canonical DB/evidence，handoff 本身不是 Finding 内容权威。

### 2.4 统一投影给下游继承

runtime repository 把 generic deliverable handoff 与 Verification typed handoff 投影为同一个 `RuntimeStageHandoffView`：

- generic 行：`authority_kind='deliverable_final_seal'`，`deliverable_submission_id=Some(...)`；
- Verification 行：`authority_kind='verification_wave_close'`，`deliverable_submission_id=None`。

读取器只返回 source StageRunUnit 已 PASS、typed authority 仍 exact 的 final-sealed 行。Access Validation / Reporting 按 stage spec 的 `inherits_evidence_from` 与 closed evidence vocabulary 过滤 Finding/FactDelta 引用；不因为没有 generic submission 而丢弃 Verification handoff。

### 2.5 response-loss 只允许 exact replay

close 请求重放时，repository 必须同时看到：

- same WaveUnit 已 ready；
- same primary WorkerRun 与 StageRunUnit 已 passed；
- same typed handoff identity、payload hash 与 pass watermark；
- exact frozen scope/org/generation spine。

全部一致才返回 `replayed=true`。缺 handoff、hash 漂移或任一状态不一致都 fail closed，不能“补写”一个不同的 terminal story。

## 3. 失败语义

| 场景 | 结果 |
|---|---|
| Attempt 尚未全终态 | close BLOCK，无局部 terminal 写 |
| verified 缺 proof/Finding lineage | close BLOCK |
| sibling org / stale Wave / wrong generation | identity mismatch，rollback |
| primary Worker 有 lease/tool | close BLOCK |
| Unit/Worker/WaveUnit 绕过 repo 直接 PASS | deferred constraint 在 commit 拒绝 |
| PASS 后缺 typed handoff | commit/replay BLOCK |
| exact response-loss replay | 返回同 handoff id/hash |
| 下游 handoff loader 失败 | 不回退模型 prose，继承上下文缺失并 fail closed 于需要该依赖的 Gate |

## 4. 非目标

- 不创建 synthetic StageDeliverableSubmission、tool call 或 CandidateAttempt。
- 不让 handoff 代替 canonical Finding、FactDelta 或 evidence ledger。
- 不在 transaction 内执行 LLM、HTTP、MQ 或长耗操作。
- 不改变 legacy Verification 的 generic handoff 路径。
