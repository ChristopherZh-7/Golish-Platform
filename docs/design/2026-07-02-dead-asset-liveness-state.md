# 死資產標記（target liveness_state）設計方案

> **目標**：給每個 in-scope target 一個一等、持久化的「存活狀態」欄位 `liveness_state`，讓 EAS 探活後能把死資產標記下來，下游階段（enumeration / vuln_triage）不再對已確認死亡的資產灌覆蓋率分母、不再浪費工具，前端也能顯示「活 / 死 / 不可達 / 未探」。
>
> **架構**：`targets` 表加一個 nullable `liveness_state` 欄位（I10 expand-first）；EAS 落庫寫點（`update_recon_extended_by_id`）在寫 `http_status`/`real_ip`/`ports` 的同時，用與 `coverage_truth::build_liveness_values_sql` 完全一致的存活判據把 `liveness_state` 蓋上；下游 seed 查詢與覆蓋率 gate 讀這個欄位把「確認死亡」的資產排除出分母、標成 `not_applicable`；ts-rs `Target` 導出該欄位供前端渲染徽章。
>
> **技術棧**：Rust（`golish-db` sqlx migration + repo、`golish-app-core` domain + ts-rs、`golish-agent-kit` harness gate、`golish-agent-app` recon bridge）+ React/TS 前端（TargetPanel）。
>
> **狀態**：草案，待用戶確認後才執行。**其中 schema migration 屬 AGENTS.md §2.7 高風險操作，動手前需用戶明確確認。**

---

## 0. 為什麼要做（現狀勘驗，帶證據）

當前 `targets` 表**沒有一等的存活狀態**，只能從幾個可變列間接推斷，且下游不按存活過濾：

| 現狀事實 | 證據 |
|---|---|
| `targets` 無 alive/dead 欄位，只有 `http_status`(nullable) / `real_ip` / `ports` / `liveness_checked_at` | `backend/crates/golish-db/src/models/pentest.rs:12`；migration `20260413000001_target_recon_extend.sql:6`、`20260623000001_targets_eas_collected_at.sql:30` |
| `target_status` enum 是**階段進度**（new/passive/active/enumerated/vuln_scan/verified），不是存活態 | `backend/crates/golish-app-core/src/domain/targets.rs:139`；migration `20260614000001_target_status_stage_aligned.sql` |
| 「探過但死」與「從沒探」在 target 行上都是 `http_status=NULL`，只有 `liveness_checked_at` 能區分 | 寫點 `backend/crates/golish-db/src/repo/targets.rs:586`（`liveness_checked_at = CASE WHEN … THEN NOW()`） |
| 存活的判據其實已存在，只是沒落成欄位：`http_status IS NOT NULL OR real_ip <> '' OR 有 fresh ports` | `backend/crates/golish-db/src/repo/coverage_truth.rs:252`（`build_liveness_values_sql`） |
| 探測失敗（DNS fail / timeout）vs 探測為空（活著沒服務）在 ledger 已區分 `error`/`empty`，但沒回寫 target | `backend/crates/golish-agent-kit/src/harness/evidence_facts.rs:433`（`eas_outcome_for_run`）；`technique_outcomes.outcome` migration `20260623000002_technique_outcomes.sql:18` |
| 下游 seed **不過濾死資產**：回傳所有 `scope='in'` | `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs:702`（`in_scope_typed_assets_impl`）、`:615`（`attack_surface_seeds_impl`） |
| 唯一存活相關過濾只針對 IP/CIDR，**域名/URL 不過濾** | `coverage_truth.rs:587`（`web_capable_ip_assets`，`http_status IS NOT NULL AND target_type IN (ip…)`） |
| 前端無存活徽章 | `frontend/components/TargetPanel/surface/tabs/NetworkEndpointsTab.tsx:64`（只渲染 `endpoint.service`） |

**直接後果**（用戶問題一）：一個探完是死的域名，`http_status` 留空、`liveness_checked_at` 被蓋戳，但沒有任何「已確認死亡」的持久標記；enumeration 的分母 `in_scope_typed_assets_impl` 照樣把它算進去，逼著模型對死域名去跑 JS/DIR/PARAM/JSAPI 或補 `checked_empty`，浪費輪次。

---

## 1. 設計決策（TL;DR）

1. **加一個一等欄位**：`targets.liveness_state TEXT`（nullable，無 default），值域 `alive | dead | unreachable`；`NULL` = 未探（unknown）。附帶可選 `liveness_reason TEXT` 記失敗原因（`dns_fail` / `timeout` / `conn_refused` / `no_service`）。
2. **判據 DRY 復用** `coverage_truth` 現有存活式：`alive` ⇔ `http_status IS NOT NULL OR real_ip <> '' OR 有開放埠`。避免第二套判定邏輯漂移。
3. **寫點單一**：只在 EAS 落庫的 `update_recon_extended_by_id`（`targets.rs:533`）與 `set_real_ip_by_id`（`:633`）蓋 `liveness_state`，與現有 `liveness_checked_at` 蓋戳同址同時。
4. **I8 語義對齊**：`dead`（探了、活著但無服務 / 探了無回應但可達）用 `checked_empty`；`unreachable`（DNS 解析失敗 / 連不上）用 `error`。兩者都是**終態**，區別只是 reason。
5. **下游讀取**：seed 查詢與覆蓋率 gate 把 `liveness_state='dead'`（及可選 `unreachable`）的資產**排除出分母**，下游該資產的技術格自動判 `not_applicable`（asset dead），打破「死域名逼分母」的迴圈。
6. **前端**：ts-rs `Target` 導出 `liveness_state` → 重生成 `Target.ts` → 在 target 列 / 詳情渲染徽章。
7. **I10 分期**：P1 加欄位+回填（inert）→ P2 寫點蓋值 → P3 讀路徑（gray-switch）→ P4 前端。每期可獨立上線、可回滾。

**不採用的替代方案**：純推導（不加欄位，用 SQL/view 即時算存活）。優點是零 schema 風險；缺點是「死」不可斷言、每處查詢都要重算、前端難顯示、也無處記失敗原因。用戶明確要一個「標記」，且下游過濾需要穩定值，故選加欄位。推導式判據仍作為 P1 回填與 P2 寫點的計算來源（兩者共用）。

---

## 2. 檔案清單（每檔職責）

| 檔案 | 動作 | 職責 |
|---|---|---|
| `backend/crates/golish-db/migrations/20260703000001_targets_liveness_state.sql` | 新建 | 加 `liveness_state` / `liveness_reason` 兩個 nullable 列 + 從既有列回填一次 |
| `backend/crates/golish-app-core/src/domain/targets.rs` | 改 | `Target` struct（ts-rs 導出源）加 `liveness_state: Option<String>`、`liveness_reason: Option<String>`；加純函數 `compute_liveness_state()` + 單測 |
| `backend/crates/golish-db/src/models/pentest.rs` | 改 | `Target`（sqlx `FromRow`）加對應兩欄 + `TARGET_ROW_COLS` |
| `backend/crates/golish-db/src/repo/targets.rs` | 改 | `update_recon_extended_by_id` / `set_real_ip_by_id` 蓋 `liveness_state`；`TARGET_ROW_COLS` 兩處字串同步 |
| `backend/crates/golish-db/src/repo/coverage_truth.rs` | 改 | 加 `dead_asset_values()` 查詢（`liveness_state='dead'`）；gate 分母排除 |
| `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 改 | 注入 in-scope 資產時扣掉 dead（gray-switch） |
| `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs` | 改 | seed JSON 帶 `liveness_state`；`in_scope_typed_assets_impl` 過濾 dead |
| `frontend/lib/generated/Target.ts` | 重生成 | ts-rs 產出，勿手改 |
| `frontend/components/TargetPanel/TargetGroupedView.tsx` / `TargetDetail.tsx` | 改 | 渲染存活徽章 |
| 各檔對應 `*_tests.rs` / `*.test.ts` | 改/新增 | 見 §9 測試計畫 |

---

## 3. Phase 1 — 加欄位 + 回填（純新增，inert）

### Task 1.1：寫 migration

**檔案**：`backend/crates/golish-db/migrations/20260703000001_targets_liveness_state.sql`（新建）

```sql
-- 死資產標記：targets.liveness_state（设计 docs/design/2026-07-02-dead-asset-liveness-state.md）。
-- I10 expand-first：nullable、无 default。NULL = 未探（unknown）。
-- 值域 alive|dead|unreachable，用 CHECK 约束守卫（允许 NULL）。
-- liveness_reason 记失败细分：dns_fail|timeout|conn_refused|no_service（可空）。
-- 可安全 replay（IF NOT EXISTS）/ 回滚（DROP COLUMN，无代码引用时）。

ALTER TABLE targets
  ADD COLUMN IF NOT EXISTS liveness_state  TEXT,
  ADD COLUMN IF NOT EXISTS liveness_reason TEXT;

ALTER TABLE targets DROP CONSTRAINT IF EXISTS targets_liveness_state_check;
ALTER TABLE targets ADD CONSTRAINT targets_liveness_state_check
  CHECK (liveness_state IS NULL OR liveness_state IN ('alive', 'dead', 'unreachable'));

-- 一次性回填：只对「已探过」(liveness_checked_at 非空) 的行推导初值；
-- 未探过的行保持 NULL（未知，绝不假装死，符合 I8）。
-- alive 判据与 coverage_truth::build_liveness_values_sql 完全一致：
--   http_status 非空 OR real_ip 非空 OR ports 里有 state=open 的端口。
UPDATE targets
SET liveness_state = CASE
    WHEN http_status IS NOT NULL
      OR real_ip <> ''
      OR EXISTS (
          SELECT 1 FROM jsonb_array_elements(ports) p
          WHERE COALESCE(p->>'state', 'open') = 'open'
      )
    THEN 'alive'
    ELSE 'dead'
  END
WHERE liveness_checked_at IS NOT NULL
  AND liveness_state IS NULL;

CREATE INDEX IF NOT EXISTS idx_targets_liveness_state
  ON targets(liveness_state) WHERE liveness_state IS NOT NULL;
```

> 回填只給 `alive`/`dead`，不給 `unreachable`——歷史行分不清「探了無回應」與「解析失敗」，保守歸 `dead`（可達性未知，但確定無服務）。`unreachable` 只在 P2 之後由新寫點按 `error` outcome 精確標。

**驗證**：
```bash
cd backend && sqlx migrate run   # 或 just check 触发 migration
# 预期：迁移无错；\d targets 显示两个新列 + CHECK 约束
```

### Task 1.2：sqlx model 加列

**檔案**：`backend/crates/golish-db/src/models/pentest.rs`（`Target` struct，`:12`）

```rust
    pub organization_id: Option<Uuid>,
    #[serde(default)]
    pub liveness_state: Option<String>,
    #[serde(default)]
    pub liveness_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
```

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`（`TARGET_ROW_COLS`，`:98` 與 `:718`、`:834` 測試常量）——三處字串都要在末尾 `updated_at` 前補 `liveness_state, liveness_reason`：

```rust
const TARGET_ROW_COLS: &str = "id, name, target_type::text, value, tags, notes, scope::text, status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports, real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type, liveness_state, liveness_reason, created_at, updated_at";
```

**驗證**：
```bash
cd backend && cargo check -p golish-db
# 预期：编译过（FromRow 列数与 SELECT 对齐）
```

### Task 1.3：app-core domain Target（ts-rs 源）加列 + 純函數

**檔案**：`backend/crates/golish-app-core/src/domain/targets.rs`（`Target` struct `:17`）

```rust
    pub http_status: Option<i32>,
    // ... 既有欄位 ...
    #[serde(default)]
    #[ts(optional)]
    pub liveness_state: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub liveness_reason: Option<String>,
```

同檔加純函數（P2 寫點 + P1 回填共用同一判據，DRY）：

```rust
/// EAS 探活结束后由 (http_status, real_ip, ports, probe_errored) 推导 target 存活态。
/// alive 判据与 coverage_truth::build_liveness_values_sql 一致；probe_errored=true
/// (DNS 解析失败 / 连不上，对应 ledger outcome=error) → unreachable；探了可达但
/// 无服务 → dead（I8：跑了→空≠没跑）。返回 (state, reason)。
pub fn compute_liveness_state(
    http_status: Option<i32>,
    real_ip: &str,
    open_ports: usize,
    probe_errored: bool,
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
# 预期：新单测全过（见 §9 Task 9.1）
```

---

## 4. Phase 2 — EAS 寫點蓋值

### Task 2.1：`update_recon_extended_by_id` 蓋 `liveness_state`

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`（`:533`）

在函數簽名加一個 `probe_errored: bool`（由呼叫方從工具 outcome 判定），並在 SQL 末尾（`updated_at = NOW()` 前）加：

```sql
            liveness_state = CASE
                WHEN $1 != '' AND {real_ip_guard} THEN 'alive'
                WHEN $4 IS NOT NULL THEN 'alive'
                WHEN $8::jsonb <> '[]'::jsonb AND EXISTS (
                    SELECT 1 FROM jsonb_array_elements($8::jsonb) p
                    WHERE COALESCE(p->>'state','open') = 'open'
                ) THEN 'alive'
                -- 探了但既无 real_ip/http_status/开放端口：区分失败与无服务
                WHEN $10 THEN 'unreachable'
                WHEN ($1 != '' {real_ip_guard_or}) OR $4 IS NOT NULL OR $8::jsonb <> '[]'::jsonb
                    THEN 'dead'
                ELSE liveness_state   -- 本次没带任何存活信号 → 不动（避免误标）
            END,
            liveness_reason = CASE
                WHEN $10 THEN 'probe_error'
                WHEN (…上面判 dead 的同条件…) THEN 'no_service'
                ELSE liveness_reason
            END,
```

> `$10` = 新增 bind 的 `probe_errored`。關鍵約束：**只有本次呼叫確實帶了存活信號或明確錯誤時才寫 `liveness_state`**；一個既沒 real_ip、沒 http_status、沒 ports、也沒 error 的「空呼叫」保持原值不動（與現有 `liveness_checked_at` 的 CASE 守衛同構，見 `:586`）。

呼叫方（httpx/nmap 輸出落庫處，`golish-recon-app` / `golish-pentest-app` 的 recon 寫入）用 `eas_outcome_for_run(...) == "error"` 推 `probe_errored`（`evidence_facts.rs:433` 已有此判定，直接復用）。

### Task 2.2：`set_real_ip_by_id` 蓋 `alive`

**檔案**：`backend/crates/golish-db/src/repo/targets.rs`（`build_set_real_ip_by_id_sql` `:623`）——設 real_ip 是強存活信號：

```sql
UPDATE targets
   SET real_ip = $1, liveness_checked_at = NOW(),
       liveness_state = 'alive', liveness_reason = NULL,
       updated_at = NOW()
 WHERE id = $2 AND target_type::text NOT IN ('ip', 'ipv4', 'ip_address', 'cidr')
```

**驗證**：
```bash
cd backend && cargo test -p golish-db targets   # SQL 字符串断言（见 §9 Task 9.2）
just test-rust
```

---

## 5. Phase 3 — 下游讀路徑（gray-switch）

### Task 3.1：`coverage_truth` 加 dead 資產查詢

**檔案**：`backend/crates/golish-db/src/repo/coverage_truth.rs`

```rust
/// 已确认死亡的 in-scope 资产 canonical key 集合（liveness_state='dead'）。
/// 供 gate 把这些资产从覆盖率分母剔除（下游对死资产的技术格 = not_applicable）。
/// 只剔 'dead'，不剔 'unreachable'（不可达可能是临时网络问题，保守保留）。
pub async fn dead_asset_values(pool: &PgPool, org_id: Option<Uuid>) -> Result<HashSet<String>> {
    // SELECT value FROM targets WHERE scope='in' AND liveness_state='dead' [AND org 隔离]
    // 结果经 canonical_asset_key 归一后返回。
}
```

### Task 3.2：gate 分母扣掉 dead（gray-switch）

**檔案**：`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（`:1389` 注入 in-scope 資產處）

在 `in_scope_assets_created_before` / `in_scope_assets` 拿到資產後，若目標 stage spec 開了新 flag `skip_dead_assets`（EAS→enumeration 這幾個 spec 打開），減去 `dead_asset_values`：

```rust
let mut assets = /* 现有注入逻辑 */;
if spec.skip_dead_assets {
    let dead = self.repo.dead_asset_values(self.harness_org_id).await.unwrap_or_default();
    let before = assets.len();
    assets.retain(|a| !dead.contains(&canonical_asset_key(a).key));
    tracing::info!(target: "harness::hook", removed = before - assets.len(),
        "excluded confirmed-dead assets from coverage denominator");
}
```

對應在 `resources/harness/stages/{enumeration,vuln_triage}/spec.json` 加 `"skip_dead_assets": true`（EAS 本身**不**開——EAS 就是負責判死的階段，不能先把自己要探的資產濾掉）。`stage_spec.rs` 加該 bool 欄位（default false = 灰度關）。

### Task 3.3：seed JSON 帶 liveness + enumeration 過濾

**檔案**：`backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`

- `attack_surface_seeds_impl`（`:615`）/ `in_scope_targets_impl`（`:588`）的 JSON 加 `"liveness_state": t.liveness_state`，讓 EAS/enumeration specialist 能看見並排序。
- `in_scope_typed_assets_impl`（`:702`，enumeration 分母源）：`.filter(|t| t.liveness_state.as_deref() != Some("dead"))`。

**驗證**：
```bash
cd backend && cargo test -p golish-agent-kit gate   # dead 域名被剔出分母（见 §9 Task 9.3）
just test-rust
```

---

## 6. Phase 4 — 前端顯示

### Task 4.1：重生成 ts-rs 型別

```bash
cd backend && cargo test -p golish-app-core export_bindings   # 或项目既定 ts-rs 生成命令
# 预期：frontend/lib/generated/Target.ts 出现 liveness_state / liveness_reason
```
> `frontend/lib/generated/Target.ts` 由 ts-rs 生成，**禁止手改**（AGENTS.md I5）。

### Task 4.2：徽章元件

**檔案**：`frontend/components/TargetPanel/TargetGroupedView.tsx` / `TargetDetail.tsx`

```tsx
function LivenessBadge({ state }: { state: string | null }) {
  const map: Record<string, { label: string; cls: string }> = {
    alive: { label: "存活", cls: "bg-green-500/10 text-green-300" },
    dead: { label: "死亡", cls: "bg-red-500/10 text-red-300" },
    unreachable: { label: "不可達", cls: "bg-yellow-500/10 text-yellow-300" },
  };
  const v = state ? map[state] : undefined;
  if (!v) return <span className="text-muted-foreground text-[10px]">未探</span>;
  return <span className={cn("rounded px-1.5 py-0.5 text-[10px]", v.cls)}>{v.label}</span>;
}
```

在 target 行/詳情把 `<LivenessBadge state={target.liveness_state ?? null} />` 放到名稱旁。

**驗證**：
```bash
just check-fe && just test-fe
# 预期：typecheck 过；徽章渲染 4 态（见 §9 Task 9.4）
```

---

## 7. 資料流（改後）

```
EAS httpx/nmap 落庫
   │  update_recon_extended_by_id(probe_errored) / set_real_ip_by_id
   ▼
targets.liveness_state ∈ {alive, dead, unreachable}   （+ liveness_reason）
   │
   ├─▶ 前端徽章（TargetGroupedView / TargetDetail）
   │
   └─▶ 下游 gray-switch skip_dead_assets：
        - enumeration/vuln_triage 覆蓋率分母剔除 dead（execute.rs）
        - in_scope_typed_assets_impl 過濾 dead（recon.rs）
        → 死域名不再逼 JS/DIR/PARAM/JSAPI 格，自動 not_applicable
```

---

## 8. 邊界與紅線

- **未探 ≠ 死**：`liveness_checked_at IS NULL` 一律 `liveness_state=NULL`（未知），回填/寫點都不得把未探標成 dead（I8）。
- **只剔 `dead`，不剔 `unreachable`**：不可達可能是暫時網路問題或 WAF 擋探測，保守保留在分母，只在 UI 標黃。
- **EAS 自己不開 `skip_dead_assets`**：EAS 是判死的階段，濾掉會讓它無事可做；只有其下游（enumeration 起）開。
- **IP vs 域名**：IP 的「活」含開放埠（非 HTTP 也算活）；域名的「活」主要看 http_status/real_ip。`compute_liveness_state` 的 `open_ports` 入參對兩者都適用。
- **schema 改動是 I10 expand-first**：P1 只加 nullable 列 + 回填（讀路徑不依賴），P2 才寫，P3 才讀且灰度；任一期可單獨回滾。
- **高風險確認**：migration 執行前需用戶在對話裡確認（AGENTS.md §2.7 改 schema/migration）。

---

## 9. 測試計畫

| # | 測試 | 檔案 | 斷言 |
|---|---|---|---|
| 9.1 | `compute_liveness_state` 四態 | `golish-app-core/src/domain/targets.rs`(tests) | 有 http_status/real_ip/open_ports→alive；probe_errored→unreachable；探了皆空→dead |
| 9.2 | 寫點 SQL 含 liveness_state CASE | `golish-db/src/repo/targets.rs`(tests) | `update_recon_extended_by_id` / `set_real_ip_by_id` 生成的 SQL 含 `liveness_state =` |
| 9.3 | gate 剔除 dead 分母 | `golish-agent-kit` gate tests | 一個 `liveness_state='dead'` 的域名不出現在 enumeration coverage 分母；alive 域名仍在 |
| 9.4 | 徽章渲染 | `frontend/.../TargetDetail.test.tsx` | alive/dead/unreachable/NULL 四態 label 正確 |
| 9.5 | migration 回填 | 手動 / repo test | 已探行按 alive 判據回填、未探行保持 NULL |

**收口**：
```bash
just precommit   # fmt + check-fe + test-fe + lint-rust + test-rust-all 全綠才算完成
```

---

## 10. 分期落地順序（可獨立 commit / 回滾）

1. **P1**（Task 1.1–1.3）：加列 + 回填 + model/domain 欄位。上線後 inert，零行為變化。
2. **P2**（Task 2.1–2.2）：寫點蓋值。上線後 `liveness_state` 開始有真值，但無人讀，仍零行為變化。
3. **P3**（Task 3.1–3.3）：讀路徑 + `skip_dead_assets` gray-switch。**這期才真正改變下游行為**——先在一個 profile 灰度開，觀察 gate 日誌 `excluded confirmed-dead assets` 再全開。
4. **P4**（Task 4.1–4.2）：前端徽章。

> 每期結束更新 `agent-progress.md` + `feature_list.json`，動過的模組同步 `docs/modules/` 卡片（AGENTS.md §2.4 / §4）。
