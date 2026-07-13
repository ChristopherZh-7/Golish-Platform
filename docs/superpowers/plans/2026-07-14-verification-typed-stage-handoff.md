# Verification Server-Authored Typed StageHandoff 实现计划

- **日期**：2026-07-14
- **状态**：Implemented；本地确定性测试已执行，未执行真实 live run
- **设计来源**：`docs/design/2026-07-13-verification-typed-stage-handoff.md`
- **migration**：`20260712000014_verification_stage_handoffs.sql`

## 1. 已执行任务

### Task 1：建立 typed authority 与 fail-closed schema ✅

- 新增 immutable `verification_stage_handoffs`，以 exact operation / scope / Wave / WaveUnit / organization / StageExecution / StageRunUnit / logical primary WorkerRun 复合身份绑定服务端 close。
- `authority_kind` 固定为 `verification_wave_close`；不创建 synthetic deliverable submission、tool call 或 worker lease。
- 用 deferred constraint 把 WaveUnit `ready`、primary Worker `passed`、Verification Unit `passed`、pass watermark 与 typed handoff 绑定成同一事务的 closure。
- 关闭 payload 根对象、typed-claim wrapper 与内部 claim 的 JSON shape；拒绝 direct/nested preseed、caller-authored Gate 时间、hash/identity 漂移与写后修改。

### Task 2：实现 DB-truth terminal close 与 exact replay ✅

- `verification_truth::close_verification_unit` 按固定锁序重载 Wave、WaveUnit、Verification Unit、logical primary Worker、CandidateAttempt、Finding/FactDelta 与 evidence truth。
- close 只接受全部 work item 已决策、每个 approved Candidate 有合法 terminal Attempt、verified/refuted/blocked 各自具备闭合 evidence authority 的状态。
- 服务端生成 deterministic handoff id、bounded payload、payload hash、truth hash、evidence ids 与 coverage watermark；同一事务封口 WaveUnit、Worker、Unit 与 handoff。
- response-loss 只接受 exact identity、payload/hash、truth 与 watermark replay；缺 handoff 或任一权威漂移均 fail closed。

### Task 3：统一下游 handoff read model ✅

- `stage_handoffs::list_latest_final_sealed_for_sources` 把 generic final seal 与 Verification typed authority 投影为统一 read model。
- Verification 投影保持 `deliverable_submission_id=None`，并再次要求 exact Unit/Worker 已 passed、scope snapshot 已 sealed。
- app DB bridge 暴露 typed close 与 inherited handoff；runtime 继续按 stage spec 的 inherited source 与 closed evidence vocabulary 消费引用，不回退模型 prose。

### Task 4：加固 evidence 与 canonical lineage 绑定 ✅

- handoff evidence 必须归属 exact operation / organization，target-bound Attempt 必须引用同 target evidence，且 evidence `created_at` 必须落在 source Attempt 生命周期内。
- verified claim 必须绑定 terminal Attempt 的 exact Finding lineage；FactDelta proposal 必须绑定 exact source Attempt 与 evidence membership。
- checked-empty 路径仍需 closed review、零 pending work item 与 evidenced no-Candidate truth，不能把“未检查”伪装为“已检查为空”。

## 2. 实现落点

| 层 | 文件 | 责任 |
|---|---|---|
| Schema | `backend/crates/golish-db/migrations/20260712000014_verification_stage_handoffs.sql` | typed table、closure constraints、DB validation、immutability |
| DB close | `backend/crates/golish-db/src/repo/verification_truth.rs` | terminal truth projection、atomic close、exact replay |
| DB read | `backend/crates/golish-db/src/repo/stage_handoffs.rs` | generic/typed unified final-seal projection |
| Runtime trait | `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs` | typed close 与 unified handoff contract |
| App adapter | `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs` | transaction boundary 与 DB/runtime view mapping |
| Consumer | `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs` | Verification close orchestration 与 inherited evidence routing |

## 3. 验证矩阵

| 不变量 | 覆盖证据 | 结果 |
|---|---|---|
| PASS 缺 typed handoff、replay 缺 handoff 均拒绝 | `verification_pass_without_typed_handoff_is_rejected_at_commit`、`verification_close_replay_rejects_missing_typed_handoff` | PASS |
| raw hash/evidence drift、空 manifest、直接伪造 terminal receipt 均拒绝 | `verification_typed_handoff_rejects_raw_hash_and_evidence_drift`、`verification_raw_handoff_rejects_unfrozen_empty_manifest`、`verification_rejects_raw_direct_terminal_attempt_without_receipt_bundle` | PASS |
| checked-empty 保留 evidenced no-Candidate truth | `verification_zero_approved_handoff_retains_evidenced_checked_empty_truth` | PASS |
| exact Finding/FactDelta/evidence lineage、closed JSON schema、preseed 与时间/target drift fail closed | `terminalizer_replay_returns_same_finding_and_lineage` hostile assertions | PASS |
| atomic close、统一 inherited projection、immutability 与 exact response-loss replay | `terminalizer_replay_returns_same_finding_and_lineage` terminal close assertions | PASS |
| 全量 Candidate V2 migration integration suite | `cd backend && cargo nextest run -p golish-db --test attack_execution_v2_migrations --no-tests=fail --status-level fail` | PASS，56/56，run `5fd8fc8f-f32d-4c83-8680-f95b504f8d45` |

## 4. 验证边界

以上证据仅证明本地 schema/repository/runtime contract 与 hostile fixture。未对任何真实目标执行 Verification、scanner、exploit、LLM 或外部服务请求，也不把本计划标记为 live acceptance；feature 的 `passing` 仍须独立、明确授权范围内的端到端运行证据。
