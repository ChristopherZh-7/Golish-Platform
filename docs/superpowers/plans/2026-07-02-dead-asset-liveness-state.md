# 死資產標記（target liveness_state）實現計畫（問題一）

> Superseded by [`2026-07-11-intel-eas-asset-identity-closure.md`](../../design/2026-07-11-intel-eas-asset-identity-closure.md) for `real_ip`/liveness semantics. Do not implement this draft's `set_real_ip_by_id -> alive` path.

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任務實現。每任務單獨 commit。設計依據：[`docs/design/2026-07-02-dead-asset-liveness-state.md`](../../design/2026-07-02-dead-asset-liveness-state.md)。

**目標：** 給 `targets` 一個一等持久欄位 `liveness_state`（`alive`/`dead`/`unreachable`，NULL=未探），EAS 探活後蓋值；下游 enumeration/vuln_triage 把確認死亡的資產剔出覆蓋率分母。
**架構：** I10 expand-first 分四期。判據 DRY 復用 `coverage_truth::build_liveness_values_sql` 的存活式；寫點單一（EAS 落庫 `update_recon_extended_by_id` / `set_real_ip_by_id`）；讀路徑 gray-switch。
**技術棧：** Rust（sqlx migration / golish-db repo+model / golish-app-core domain+ts-rs / golish-agent-kit gate）+ React/TS。

> ⚠️ **高風險確認**：Task 1.1 的 migration 屬 AGENTS.md §2.7「改 DB schema/migration」，**執行前必須在對話裡取得用戶確認**。其餘任務不需。

---

## Phase 1 — 加欄位 + 回填（inert，零行為變化）

### Task 1.1：migration（⚠️ 執行前確認）

**檔案**：`backend/crates/golish-db/migrations/20260703000001_targets_liveness_state.sql`（新建）

```sql
-- 死資產標記：targets.liveness_state（设计 docs/design/2026-07-02-dead-asset-liveness-state.md）。
-- I10 expand-first：nullable、无 default。NULL = 未探（unknown）。
ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS liveness_state  TEXT,
  ADD COLUMN IF NOT EXISTS liveness_reason TEXT;

ALTER TABLE targets DROP CONSTRAINT IF EXISTS targets_liveness_state_check;
ALTER TABLE targets ADD CONSTRAINT targets_liveness_state_check
  CHECK (liveness_state IS NULL OR liveness_state IN ('alive', 'dead', 'unreachable'));

-- 一次性回填：只对已探过(liveness_checked_at 非空)的行推导；未探保持 NULL(I8)。
-- alive 判据与 coverage_truth::build_liveness_values_sql 一致。
UPDATE targets
SET liveness_state = CASE
    WHEN http_status IS NOT NULL OR real_ip <> ''
      OR EXISTS (SELECT 1 FROM jsonb_array_elements(ports) p
                 WHERE COALESCE(p->>'state','open') = 'open')
    THEN 'alive' ELSE 'dead' END
WHERE liveness_checked_at IS NOT NULL AND liveness_state IS NULL;

CREATE INDEX IF NOT EXISTS idx_targets_liveness_state
  ON targets(liveness_state) WHERE liveness_state IS NOT NULL;
```

**驗證**：
```bash
cd backend && sqlx migrate run
# 预期：迁移无错；psql \d targets 显示两列 + CHECK
```
**Commit**：`feat(db): add targets.liveness_state column + backfill (migration)`

### Task 1.2：sqlx model + row cols

**檔案**：`backend/crates/golish-db/src/models/pentest.rs`（`Target` `:12`）——在 `organization_id` 後、`created_at` 前加：
```rust
    #[serde(default)]
    pub liveness_state: Option<String>,
    #[serde(default)]
    pub liveness_reason: Option<String>,
```

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`——`TARGET_ROW_COLS`（`:98`）及測試常量（`:718`、`:834`）三處，在 `content_type,` 後、`created_at` 前補 `liveness_state, liveness_reason,`。

**驗證**：
```bash
cd backend && cargo check -p golish-db
# 预期：FromRow 列数与 SELECT 对齐，编译过
```
**Commit**：`feat(db): thread liveness_state through Target row model`

### Task 1.3：app-core domain Target（ts-rs 源）+ 純函數

**檔案**：`backend/crates/golish-app-core/src/domain/targets.rs`（`Target` `:17`）加：
```rust
    #[serde(default)]
    #[ts(optional)]
    pub liveness_state: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub liveness_reason: Option<String>,
```
> 該 crate 的 `Target` 若也走 sqlx `FromRow` / 手工 map，同步 SELECT 欄位（讀該檔既有 from-row 實作，對齊）。

同檔加純函數 + 單測：
```rust
/// EAS 探活后由 (http_status, real_ip, open_ports, probe_errored) 推导存活态。
/// alive 判据与 coverage_truth::build_liveness_values_sql 一致。
pub fn compute_liveness_state(
    http_status: Option<i32>, real_ip: &str, open_ports: usize, probe_errored: bool,
) -> (&'static str, Option<&'static str>) {
    if http_status.is_some() || !real_ip.trim().is_empty() || open_ports > 0 {
        ("alive", None)
    } else if probe_errored {
        ("unreachable", Some("probe_error"))
    } else {
        ("dead", Some("no_service"))
    }
}
```

**驗證**：
```bash
cd backend && cargo test -p golish-app-core compute_liveness_state
```
**Commit**：`feat(app-core): Target.liveness_state + compute_liveness_state`

---

## Phase 2 — EAS 寫點蓋值

### Task 2.1：`update_recon_extended_by_id` 蓋 liveness_state

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`（`:533`）——簽名加 `probe_errored: bool`（新 bind `$10`，`id` 順延到 `$11`）；在 `updated_at = NOW()` 前加（見設計 §4.1 完整 SQL）：
```sql
            liveness_state = CASE
                WHEN ($1 != '' AND {real_ip_guard}) OR $4 IS NOT NULL
                  OR ($8::jsonb <> '[]'::jsonb AND EXISTS (
                       SELECT 1 FROM jsonb_array_elements($8::jsonb) p
                       WHERE COALESCE(p->>'state','open')='open')) THEN 'alive'
                WHEN $10 THEN 'unreachable'
                WHEN $8::jsonb <> '[]'::jsonb OR $4 IS NOT NULL THEN 'dead'
                ELSE liveness_state
            END,
            liveness_reason = CASE
                WHEN $10 THEN 'probe_error'
                WHEN ($1='' OR NOT {real_ip_guard}) AND $4 IS NULL
                  AND $8::jsonb <> '[]'::jsonb THEN 'no_service'
                ELSE liveness_reason
            END,
```
呼叫方（`golish-recon-app` / `golish-pentest-app` 落 httpx/nmap 輸出處）用 `evidence_facts::eas_outcome_for_run(..) == "error"` 推 `probe_errored`（復用既有判定，`evidence_facts.rs:433`）。搜全部呼叫點補傳 `probe_errored`（預設 `false`）。

### Task 2.2：`set_real_ip_by_id` 蓋 alive

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`（`build_set_real_ip_by_id_sql` `:623`）：
```sql
   SET real_ip = $1, liveness_checked_at = NOW(),
       liveness_state = 'alive', liveness_reason = NULL, updated_at = NOW()
```

**驗證**：
```bash
cd backend && cargo test -p golish-db targets && just test-rust
# 预期：SQL 字符串断言含 "liveness_state ="；现有 targets 测试不回归
```
**Commit**：`feat(db): stamp liveness_state at EAS landing write sites`

---

## Phase 3 — 下游讀路徑（gray-switch）

### Task 3.1：`coverage_truth::dead_asset_values`

**檔案**：`backend/crates/golish-db/src/repo/coverage_truth.rs`——加查詢（鏡像既有 `web_capable_ip_assets` `:587` 的形態，canonical key 歸一）：
```rust
/// scope='in' 且 liveness_state='dead' 的资产 canonical key 集（供 gate 剔分母）。
/// 只剔 dead，不剔 unreachable（可能临时网络问题，保守保留）。
pub async fn dead_asset_values(pool: &PgPool, org_id: Option<Uuid>) -> Result<HashSet<String>> { … }
```

### Task 3.2：`stage_spec` flag + gate 分母扣除

**檔案**：`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`——`StageSpec` 加 `#[serde(default)] pub skip_dead_assets: bool`。
**檔案**：`resources/harness/stages/enumeration/spec.json`、`resources/harness/stages/vuln_triage/spec.json`——各加 `"skip_dead_assets": true`（**EAS 不加**：它是判死階段）。
**檔案**：`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（`:1389` 注入 in-scope 資產後）：
```rust
if spec.skip_dead_assets {
    if let Ok(dead) = self.repo.dead_asset_values(self.harness_org_id).await {
        let before = assets.len();
        assets.retain(|a| !dead.contains(&canonical_asset_key(a).key));
        tracing::info!(target: "harness::hook", removed = before - assets.len(),
            "excluded confirmed-dead assets from coverage denominator");
    }
}
```
> 需在 `db_traits/repo.rs` trait + `db_bridge` impl 加 `dead_asset_values` 轉發（鏡像既有 `web_capable_ip_assets` 的 trait 佈線）。

### Task 3.3：seed JSON 帶 liveness + enumeration 分母過濾

**檔案**：`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`——
- `attack_surface_seeds_impl`（`:615`）/`in_scope_targets_impl`（`:588`）JSON 加 `"liveness_state": t.liveness_state`。
- `in_scope_typed_assets_impl`（`:702`）：`.filter(|t| t.liveness_state.as_deref() != Some("dead"))`。

**驗證**：
```bash
cd backend && cargo test -p golish-agent-kit gate && just test-rust
# 预期：新增 gate 测试——dead 域名不入 enumeration 分母，alive 仍在
```
**Commit**：`feat(harness): exclude confirmed-dead assets from downstream coverage`

---

## Phase 4 — 前端徽章

### Task 4.1：重生成 ts-rs
```bash
cd backend && cargo test -p golish-app-core   # 触发 ts-rs export
# 预期：frontend/lib/generated/Target.ts 出现 liveness_state / liveness_reason
```
> `Target.ts` 由 ts-rs 生成，禁止手改（I5）。

### Task 4.2：徽章
**檔案**：`frontend/components/TargetPanel/TargetGroupedView.tsx` / `TargetDetail.tsx`——加 `LivenessBadge`（見設計 §6 Task 4.2），放到 target 名稱旁。alive=綠/dead=紅/unreachable=黃/NULL=「未探」灰。

**驗證**：
```bash
just check-fe && just test-fe
```
**Commit**：`feat(frontend): target liveness_state badge`

---

## 落地順序與協調

- **P1、P2 現在可做**（inert，不改行為）；但 **P3 改 `execute.rs:1389` 分母注入，與 gate 計畫（`2026-07-02-gate-capability-ledger.md`）+ 問題二 wave 計畫共用同一觸點**——三者其一先落，其餘 rebase。建議 P3 待 gate 方向定後與之同輪落。
- P4 依賴 P1 的 ts-rs 欄位。

## 收口與自檢

```bash
just precommit
```
自檢：未探≠死（回填/寫點都不標未探）；只剔 dead 不剔 unreachable；EAS 不開 skip_dead_assets；`Target.ts` 不手改。規格三問（標記/下游跳過/前端可見）分別由 P2 / P3 / P4 覆蓋。
