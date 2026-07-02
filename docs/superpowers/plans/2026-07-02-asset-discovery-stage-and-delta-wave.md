# 資產發現階段歸屬 + 新資產 delta 波次 實現計畫（問題二）

> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任務實現。每任務單獨 commit。

**目標：** (A) 讓每個資產/埠帶「在哪個階段被發現」；(B) EAS 掃出的新資產不灌當前卡分母（已做）、而是走一個**全局 delta 擴展波次**再跑一次，而不是「同卡加分母 retry」。
**架構：** A 是低風險獨立項（targets 加 `discovered_stage` denormalized 列 + 埠標記 + 前端顯示）。B **不重新設計**——`docs/design/2026-06-28-eas-global-delta-expansion.md` 已是權威設計（supersede 了 wave-barrier），且 `golish-db/src/repo/stage_asset_waves.rs` 的波次 CRUD 已實作；B 的工作是**把「Future Delta Pass」那段接線到 stage_run 調度**。
**技術棧：** Rust（golish-db / golish-agent-kit / golish stage_run）+ React/TS。

> 先讀權威設計：[`docs/design/2026-06-28-eas-global-delta-expansion.md`](../../design/2026-06-28-eas-global-delta-expansion.md)（問題二 B 的行為規格全在這）。

---

## 現狀（實讀證據）

- **無「發現階段」欄位**：`targets` 只有 `status`（階段進度）、`created_at`、`source`（`automated/ai/manual`，migration `20260409000003`，不是 stage）。埠在 `targets.ports`(JSONB) / `network_endpoints` / `target_assets.port`，無發現階段。
- **分母凍結已做**：`asset_wave_cutoff_for_gate`（`execute.rs:1421`）+ `stage_asset_waves` 表 + repo（`current_or_create_initial`/`create_next`/`complete`，`stage_asset_waves.rs:248/290/369`）都在。
- **未做的是「全局 delta pass 調度」**：`2026-06-28-eas-global-delta-expansion.md` §Future Delta Pass 明列——目前「per-org 通過後不立即開下一波」，但「全局 delta 擴展 pass」還沒接。`stage_run/` 目錄不引用 `stage_asset_waves`（調度接線缺）。

---

## Part A — 發現階段歸屬（低風險，先做）

### Task A1：migration 加 `discovered_stage`（⚠️ schema，執行前確認）

**檔案**：`backend/crates/golish-db/migrations/20260703000002_targets_discovered_stage.sql`（新建）

```sql
-- 资产发现阶段归属（问题二 A）。I10：nullable、无 default。NULL = 未知/历史行。
-- 值 = StageKind.as_str()（scoping/target_intel/external_attack_surface/enumeration/...）。
ALTER TABLE targets ADD COLUMN IF NOT EXISTS discovered_stage TEXT;

-- 回填：已有资产按 status 反推「至少在哪个阶段之前已存在」的保守下界；
-- 无法精确知道首次发现阶段，故只对明显的初始种子给 scoping/target_intel，其余留 NULL。
-- （保守：宁可 NULL 也不假标；前端把 NULL 显示为「未知」。）
UPDATE targets SET discovered_stage = 'target_intel'
WHERE discovered_stage IS NULL AND status::text IN ('passive','active','enumerated','vuln_scan','verified')
  AND source = 'ai';

CREATE INDEX IF NOT EXISTS idx_targets_discovered_stage
  ON targets(discovered_stage) WHERE discovered_stage IS NOT NULL;
```

**驗證**：`cd backend && sqlx migrate run` → `\d targets` 見新列。
**Commit**：`feat(db): add targets.discovered_stage (migration)`

### Task A2：model + row cols + domain/ts-rs

**檔案**：`golish-db/src/models/pentest.rs`（`Target`）、`golish-db/src/repo/targets.rs`（`TARGET_ROW_COLS` 三處）、`golish-app-core/src/domain/targets.rs`（`Target` + `#[ts(optional)]`）——加 `pub discovered_stage: Option<String>`（做法同死資產計畫 Task 1.2/1.3）。

**驗證**：`cargo check -p golish-db && cargo check -p golish-app-core`。
**Commit**：`feat(db): thread discovered_stage through Target model`

### Task A3：資產首次落庫時蓋發現階段

**檔案**：資產建立寫點。首選在 recon landing / `manage_targets` create 處，拿當前 `operation_state.current_stage`（`execute.rs` 已有 `sync_operation_stage_on_entry`，stage 從 harness context 可得），INSERT 時帶 `discovered_stage`。若寫點拿不到 stage 上下文，退而在 `direct/mod.rs` 落資產的 harness hook 處補（那裡有 `harness_org_id` / stage）。**實作前先讀資產 INSERT 的真實呼叫鏈**（`golish-recon-app` persistence / `manage_targets`），確認 stage 上下文可達，勿假設。

- 埠層級：`update_recon_extended_by_id` 合併 ports 時，對**新增**的埠 element 打 `"discovered_stage": "<stage>"`（在 JSONB merge 的 `np` 分支加欄位）。

**驗證**：`cargo test -p golish-db targets`（SQL 含 discovered_stage 寫入）。
**Commit**：`feat: stamp discovered_stage on asset/port first landing`

### Task A4：前端顯示

**檔案**：`frontend/lib/generated/Target.ts`（ts-rs 重生成）+ `TargetGroupedView.tsx` / `TargetDetail.tsx` 顯示「發現於 <stage>」小標籤；埠列表顯示每埠 `discovered_stage`。

**驗證**：`just check-fe && just test-fe`。
**Commit**：`feat(frontend): show asset/port discovery stage`

---

## Part B — 新資產全局 delta 波次（接線既有設計，較大工程）

> 依 `2026-06-28-eas-global-delta-expansion.md`：**不做 per-org 立即下一波**，做**全局 delta pass**（所有 org 種子波都過關後，跑一次跨 org 的新發現增量）。

### Task B1：把 wave repo 接進 db trait（讓 stage_run/orchestration 可用）

**檔案**：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`（trait）+ `golish-agent-app/src/ai/db_bridge/orchestration.rs`（impl）——若尚未轉發，加 `stage_asset_wave_current_or_create_initial` / `_create_next` / `_complete` / `list_new_in_stage`（讀 `created_at > 波起始` 且未歸屬任何波的 in-scope 資產）。部分已在（grep 顯示 orchestration.rs 已引用 stage_asset_wave），**先讀現況補缺口**。

### Task B2：delta 候選分類器（純函數，可單測）

**檔案**：`backend/crates/golish-agent-kit/src/harness/`（新 `delta_expansion.rs`）——純函數把一條新發現分類為 `web_endpoint | new_host | service_fact_only | duplicate | blocked | out_of_scope`（設計 §Future Delta Pass 2）。非 web 埠（SSH/MySQL/RDP/Redis/VPN）→ `service_fact_only`（存 ports/fingerprints，**不**進資產分母）；web-like（http/https/8443…）→ `web_endpoint`（升級成 URL target）；全新 host/IP/domain → `new_host`。DB-free、可單測。

**驗證**：`cargo test -p golish-agent-kit delta_expansion`（每類一個用例）。
**Commit**：`feat(harness): pure delta-expansion candidate classifier`

### Task B3：全局 delta pass 調度

**檔案**：`backend/crates/golish/src/stage_run/scheduler.rs`（或 `mod.rs`）——所有 org 的 EAS 種子波都 `complete` 後，跑一次全局 delta：讀各 org `list_new_in_stage` → 過 B2 分類器 → 只把 `web_endpoint`/`new_host` 用 `stage_asset_wave_create_next` 建一個跨 org delta 波 → fan-out 對 delta 波只探「缺的最小維度」→ 全部關閉才發 EAS close pass token。

**收斂/邊界（硬約束，設計 §Invariants）**：
- 波深上限（防遞迴爆炸）；delta 波為空即終止。
- 新發現**絕不**silently 擴當前種子波分母。
- URL endpoint 資產不再要 PORT/SERVICE 格（host 級已覆蓋），只要 liveness/web-fp。
- HITL：模糊 scope / 風險擴展要人工確認（`ask_human`）。

**驗證**：`cargo test -p golish stage_run`（delta 波建立/收斂/空即停的單測，用 in-memory / mock repo）；`just test-rust`。
**Commit**：`feat(stage_run): global EAS delta-expansion pass`

---

## 落地順序與協調

- **Part A 現在可做**（獨立、低風險，除 A1 migration 需確認）。
- **Part B 較大**，且 B3 碰 `stage_run` 調度 + 與死資產 P3 / gate 計畫共用 `execute.rs` 分母注入語義——建議 **B 待 gate 方向定後**再落，避免分母語義改兩遍。Part B 也應與死資產 P3 協同：dead 資產不該進 delta 波（分類器歸 `blocked`/`out_of_scope` 或 delta 候選查詢直接排除 `liveness_state='dead'`）。

## 收口與自檢

```bash
just precommit
```
自檢：發現階段回填保守（拿不準留 NULL）；delta pass 有波深上限與空即停；非 web 埠不進資產分母；URL endpoint 不重複要 PORT/SERVICE；dead 資產不進 delta 波。Part B 完全對齊 `2026-06-28-eas-global-delta-expansion.md`，未另立設計。
