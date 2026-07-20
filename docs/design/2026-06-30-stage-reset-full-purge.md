# 階段重置：完整清理 + 可往回跳（dev stage reset full purge）

> Superseded by [`2026-07-20-chatpanel-stage-reset-consistency.md`](2026-07-20-chatpanel-stage-reset-consistency.md). The successor narrows in-place full reset to four Company stages, uses the sealed frozen scope rather than a live organization subtree, retains evidence and ownership-ambiguous immutable facts, and commits runtime/facts/state/graph/cursor atomically.

- 狀態：草案（待用戶確認刪除矩陣後進入實作）
- 日期：2026-06-30
- 觸發：用戶反映 chatpanel 右下角「重置階段」按鈕無法把階段恢復到初始狀態（已發現的 JS / 資產等資料不會被刪），導致重測階段時不是乾淨起跑。

## 1. 問題與根因（證據鏈）

現行按鈕（`frontend/components/AIChatPanel/AIChatPanel.tsx` 右下角 `RotateCcw`）呼叫
`resetHarnessStageCheckpoint({ mode: "restart_stage" })` → 後端
`harness_dev_reset_stage_checkpoint`（`backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`）。

該 command 檔頭註解明確：只調整 `operation_state` 的 resumability checkpoint，
**without deleting evidence, assets, or target facts**。因此四類狀態完全沒被清：

| # | 殘留 | 影響 | 證據 |
| --- | --- | --- | --- |
| 1 | `org_stage_completions`（每 org 每 stage 通過台帳） | resume oracle 視為「已過」→ 重跑被 skip | `repo/org_stage_completions.rs` 只有 `upsert`/`get`，無 delete |
| 2 | `stage_asset_waves` / `_items`（階段資產波次快照） | 舊波次殘留、gate 分母錯亂 | `repo/stage_asset_waves.rs` 無 delete |
| 3 | 發現的事實（assets / dns / endpoints / js / sitemap / evidence / coverage） | 重跑看到舊資料，不是零狀態 | `db_bridge/recon.rs` 寫入皆以 `organization_id` 範圍 |
| 4 | `.golish/captures｜analysis/<host>` 落地檔 | 抓包 / 分析產物殘留 | `organizations/artifact_cleanup.rs` 同款清理模板 |

## 2. 目標 / 非目標

- 目標：選定階段 X（如 `external_attack_surface`）→ 一鍵把 X 及其**所有 DAG 後代**恢復到「X 剛開始」的乾淨狀態，並把 operation cursor 移到 X，可直接重測 X，無需從頭跑。
- 目標：按鈕升級為下拉，只能選 **≤ 當前階段**（往回 / 更早，已跑過）的階段；更晚 / 未到達的階段 disabled（不能往後跳）。
- 非目標：不刪「日誌」——`audit_log`、`~/.golish/*.log`、transcripts、`run.log` 一律保留（用戶明確要求）。
- 非目標：不動 X **之前**（ancestors）階段的資料（它們是 X 的輸入）。
- 非目標：不改 DB schema（表已存在，只新增 delete 函式）。

## 3. 往回跳語意

DAG（assessment）：`scoping → target_intel → external_attack_surface → {enumeration, reporting}`，`enumeration → reporting`。

- 「往前跳」=回到更早 / 已跑過的階段（允許）。
- 「往後跳」=跳到更晚 / 還沒到的階段（禁止，UI disabled）。
- 判定：可選集合 = `dag.ancestors_inclusive(current_stage)`（即當前階段及其全部上游）。

## 4. 刪除矩陣（階段 → 資料域）— **待用戶確認**

「重置 from X」= 對 X ∪ `dag.descendants_inclusive(X)` 的每個階段，在**當前 engagement org 子樹**內清下表對應資料；同時清台帳/波次/coverage/cursor。

| 階段 | 清除的事實表（repo） | targets.status 回滾到 |
| --- | --- | --- |
| `target_intel` | `target_assets`、`dns_records`、`passive_scans`、`fingerprints`(passive)、`source_query_log`、organizations intel 欄位(intel_collected_at/whois) | `new` |
| `external_attack_surface` | targets 的 ports/real_ip/ip_whois/eas_collected_at、active `fingerprints`、`screenshots`、`expansion_queue`、`source_stage='external_attack_surface'` 衍生 asset | `passive` |
| `enumeration` | `api_endpoints`、`js_analysis`、`sitemap_store`、`directory_entries`、`coverage_truth`(dir/param/jsapi)、`endpoint_tests` | `active` |
| `vuln_triage`/`verification`/`access_validation` | `vuln_scan`、`vuln_intel`、`findings`、`sensitive_scan`、`technique_outcomes`、`evidence_classifications` | `enumerated` |
| `reporting` | 報告類產物（findings 最終化 / notes 報告段） | 不回滾 |

跨階段一律清：`org_stage_completions`、`stage_asset_waves`/`_items`、`evidence_ledger`(對應 X+ 階段的 evidence)、`.golish/captures｜analysis/<host>`。

> 待確認點：`evidence_ledger`（屬「證據」非「日誌」）預設清 X+，以便 gate 乾淨重評；若用戶要保留，改為不清。

## 5. IPC 變更

- `harness_dev_reset_stage_checkpoint` 新增模式 `restart_from_stage_purge`（或在 args 加 `purge: bool`）。
- 回傳 `HarnessDevStageCheckpointResetResult` 擴充各表刪除計數（透明可審計）。
- 前端 `harness-dev.ts` 對應加 mode + 回傳型別（ts-rs 同步）。

## 6. 安全 / 回滾

- 仍受 `ensure_dev_checkpoint_reset_allowed()` 門禁（debug build 或 `GOLISH_ENABLE_DEV_STAGE_RESET`）。
- 刪除限定 engagement org 子樹，不波及整庫 / 其他 engagement。
- 破壞性、不可逆：UI 二次確認 + 後端記 `audit_log` 一筆（保留日誌）。
- 所有刪除在單一 DB transaction 內（I9：交易內不呼叫外部 HTTP；檔案刪除放交易外）。

## 7. 測試計畫

- repo 層：各 delete 函式的 SQL 形狀單測（org 子樹、stage 過濾）。
- command 層：reset_state_blob 既有測試擴充 + purge 路徑的 stage 集合 / 計數測試。
- 前端：下拉只列 ≤ 當前階段、disabled 規則、確認流程測試。
- `just precommit` 全綠。
