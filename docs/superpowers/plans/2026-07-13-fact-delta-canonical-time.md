# FactDelta Canonical Time 与 Evidence Interval 实现计划

> **执行要求：** 使用 TDD；每个生产边界必须先有可观察 RED，再转 GREEN。

**目标：** 让 FactDelta 的 canonical 时间语义与 source Attempt evidence 时间在 repository、migration 和 consolidation 三层一致 fail closed。

**设计依据：** `docs/design/2026-07-13-fact-delta-canonical-time.md`。

## Task 1：closed kind 与 evidence 时间（RED）

修改 `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`：

- raw SQL 写入未知 `delta_kind` 必须失败；
- `audit_log.created_at` 早于 source Attempt 的 evidence，即使后来被 link，也不能成为 FactDelta evidence；
- repository exact replay 仍保留首次 semantic provenance/evidence。

验证：

```bash
just space-guard
cd backend && cargo test -p golish-db --test attack_execution_v2_migrations fact_delta -- --nocapture
```

## Task 2：kind-specific canonical time（RED）

修改 canonical acceptance fixture：旧 canonical API endpoint 的 fresh `refuted` 应接受；旧事实的 `created|updated|new_surface`、sibling org、hash drift 应拒绝。

验证：

```bash
just space-guard
cd backend && cargo test -p golish-db --test attack_execution_v2_migrations sibling_or_stale_canonical_ref_delta_is_rejected -- --nocapture
```

## Task 3：三层 GREEN

- `20260712000012_attack_fact_delta_wave_entry.sql`：为新写入增加 closed-kind CHECK；为 FactDelta evidence link 增加 Attempt interval trigger。
- 同一 migration 增加 canonical JSON/hash、closed source rehydrate 与 deferred decision material validator；delta/decision/evidence 以及 consolidation graph 都必须独立重算 semantic dedupe、evidence set 和 decision hash，migration-time 扫描既有 kind/time 漂移。
- `attack_fact_deltas.rs`：proposal 校验 exact evidence membership 与 `created_at..terminal_at`。
- `canonical_fact_refs.rs`：新增 `resolve_for_fact_delta`；`refuted` 可引用旧 exact row，其他 kind 要求当前 projection 位于 Attempt interval，全部拒绝 post-terminal row。
- `attack_wave_consolidations.rs`：使用 closed kind 和专用 resolver，并对历史 malformed evidence 做 defense-in-depth reject。

## Task 4：回归与文档

```bash
just space-guard
cd backend && cargo test -p golish-db --test attack_execution_v2_migrations -- --nocapture
just space-guard
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
```

同步 `docs/modules/backend/golish-db{,/repo}.md`、`docs/modules/INDEX.md`、`agent-progress.md` 与 `feature_list.json`。只有 fresh `just precommit` 全绿后才可宣称完成或提交。
