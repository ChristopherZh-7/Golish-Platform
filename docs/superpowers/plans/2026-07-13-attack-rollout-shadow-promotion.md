# Candidate V2 Shadow-Gated Rollout Promotion 实现计划

**目标：** 消除 migration 直跳 V2Only 的 shadow-gate 旁路，以 commit 后独立 reconciler 完成可重放的相邻 promotion。

## Task 1：cutover RED ✅

修改 migration integration test，要求 fresh migration 默认只到
`dual_write_read_legacy/rank=1/row_version=1`；新 operation 冻结同一 contract。旧测试会观察到
`v2_only/rank=3`，必须先 RED。

## Task 2：reconciler / cohort RED ✅

覆盖：

- rank1 无 admission / open-unsealed WaveUnit / missing final Candidate sample 均 typed not-ready；
- Candidate final seal 可在 review/verifier 未终态时晋级；exact terminal follow-on zero-input
  WaveUnit 从 Candidate denominator 排除，malformed zero-input shape fail closed；
- exact retained match 只推进 rank1→rank2；
- rank2 必须有本 contract 的新 exact sample才推进 rank2→rank3；
- 两个 concurrent reconciler 最多推进一级；
- forged stored V2 hash 重建后仍 incomplete；
- final-seal commit 成功而 promotion storage failure 时，业务 truth 保留并可稍后重试；
- admission/receipt 的 nested mutation、shadow nested delete 与 caller-forged close timestamp 均拒绝；
- 已 admission 的 frozen old-contract operation 在默认 rank 提升后仍可继续后续 Wave，首次 stale
  admission 仍拒绝。

## Task 3：GREEN ✅

- `00005` 只做 rank0→rank1 guarded UPDATE；
- additive `00016` 新增 operation-scoped admission sequence、promotion cutoff/receipt、DB canonical
  Candidate rebuild 与 raw UPDATE gate；
- `attack_execution_rollout.rs` 新增 pool-owned post-commit reconciler，内部固定
  rollout→cohort/Wave/Unit→ordered shadow 锁序；not-ready 返回 typed unchanged；
- app `finalize_unit_pass` 在业务 transaction commit 后 best-effort reconcile；
- app `create_runtime_operation` 在冻结新 contract 前 best-effort reconcile，关闭 crash gap；
- promotion trace 只发 contract/rank/aggregate。

## Task 4：验证（scoped ✅；2026-07-14 final precommit ✅）

```bash
just space-guard
cd backend && cargo test -p golish-db --test attack_execution_v2_migrations rollout -- --nocapture
just space-guard
cd backend && cargo test -p golish-agent-app attack_rollout -- --nocapture
just space-guard
cd backend && cargo clippy -p golish-db -p golish-agent-app --all-targets -- -D warnings
```

独立 integration test 已覆盖 fresh rank1、合同矩阵、首次 admission、open/unsealed block、
Candidate final-seal liveness、exact/malformed follow-on zero-input、old-contract continuation、
stale first admission、admission/shadow nested-trigger mutation、server-owned close timestamp、SQL
canonical rebuild、raw UPDATE、DB-generated receipt、direct receipt forgery 与 typed repository
reconcile。2026-07-14 已纳入 feature verification，最终 `just precommit` exit 0；父 feature 仍须等授权 live acceptance 后才能标 `passing`。
