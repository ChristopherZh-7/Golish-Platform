# FactDelta Canonical Time 与 Evidence Interval 补充设计

- **日期**：2026-07-13
- **状态**：Accepted for implementation
- **适用范围**：terminal CandidateAttempt 的 FactDelta proposal 与 Wave consolidation
- **补充关系**：修正 generic StageHandoff freshness 不能直接复用于 FactDelta 的语义缺口

## 1. 决策

FactDelta kind 是闭集：`created | updated | refuted | new_surface`。未知 prose kind 在 typed decode、repository proposal、DB 新写入与 consolidation 四层均拒绝。

canonical subject 继续由 closed catalog 按 exact operation/org/project identity 解析，version/hash 必须与锁住的权威行一致；时间规则按 kind 区分：

- `created` / `new_surface`：canonical row 的 observed/updated time 必须位于 source Attempt 的 `created_at..terminal_at`；
- `updated`：subject 可更早存在，但当前 canonical projection 的 updated time 必须位于同一 Attempt interval；
- `refuted`：允许引用 Attempt 前存在的 exact canonical row，但不能引用 terminal 后才出现/更新的 row。

所有 kind 的 `fact_delta` evidence 都必须属于 exact source Attempt，且 `audit_log.created_at` 位于同一 Attempt interval。把旧 evidence 在 terminal 后补 link 不能制造新 FactDelta。

## 2. 写入与 consolidation

proposal repository 在写 `attack_fact_deltas` 前验证 kind、semantic hash、source Attempt terminal identity、exact evidence membership 与 evidence interval。additive migration 对新 delta kind 与 evidence link做同等 fail-closed guard；migration-time scan 对既有未知 kind 或 Attempt interval 外 evidence 直接失败，不能把不合法历史静默冻结成可消费事实。

consolidation 不再调用 generic `resolve_for_handoff(freshness_floor)`；它调用 `resolve_for_fact_delta`，按 kind/Attempt interval重载 canonical row。decision reason 稳定区分 unsupported kind、evidence interval mismatch、canonical time mismatch 与 hash mismatch。只有全部通过的 proposal 才能进入 accepted set/next Wave。

accepted/rejected decision 不是 caller 可自报的标签。DB deferred validator 对两类 decision 都从 FactDelta evidence membership/timestamp 重新计算 `evidence_set_hash` 与 `decision_hash`；accepted 还必须从 source Attempt、冻结 target tuple与 closed canonical source row 重新计算 semantic `dedupe_hash`。delta/decision/evidence 任一写入和 Wave consolidation graph 都调用同一校验，因此裸 SQL 即使绕开普通 immutable trigger，也不能凭伪造 hash 打开 follow-on Wave。

## 3. 验证矩阵

- old canonical fact + fresh `refuted` evidence：accept；
- old canonical fact + `created|updated|new_surface`：reject；
- canonical row 在 Attempt 内更新 + `updated`：accept；
- pre-Attempt evidence 后补 link：proposal/DB guard reject；
- sibling org、future row、hash drift、unknown kind：reject；
- forged accepted decision/evidence-set/dedupe hash：deferred commit reject；
- response-loss replay保留首次 semantic provenance/evidence，不追加第二套 evidence。
