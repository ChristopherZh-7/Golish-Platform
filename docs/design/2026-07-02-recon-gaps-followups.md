# 偵察四階段其餘缺口的規劃 + 與 gate 任務的協調

> **隸屬總綱**：本文件的問題二/三/四協調結論已被跨階段 crediting 統一總綱吸收（問題三 = crediting 根因、問題一/二 = 分母旋鈕）。總綱見 [`docs/design/2026-07-02-cross-stage-crediting-unification.md`](2026-07-02-cross-stage-crediting-unification.md)（§5.5 掛接矩陣、§6 正交旋鈕）。
>
> **背景**：接續 `docs/reference/recon-four-stages.md`（四階段梳理）與 `docs/design/2026-07-02-dead-asset-liveness-state.md`（問題一：死資產標記）。使用者在 EAS/enumeration 上提出四個問題，本文件規劃**問題二（發現階段歸屬 + wave 重跑）**、**問題四（前端指紋顯示）**，並把**問題三（工具職責重疊）**整理成交給「另一個 AI 的 gate 任務」的交接簡報——因為問題三與那個任務是同一個根因，不應在此另起爐灶做重工。
>
> **狀態**：規劃草案，未動代碼。
>
> **關鍵協調結論（先看這個）**：四個問題按「是否依賴 gate crediting 邏輯」分兩組——問題四純前端、問題二主要 DB/編排，兩者**與 gate 任務解耦，現在就能推**；問題三 = gate crediting 根因，**歸另一個 AI**，我只給證據簡報 + 標出共用觸點；問題一（已寫方案）的 P3 會碰到 gate 分母注入，**需與 gate 任務對齊落地順序**。

---

## 0. 依賴地圖：哪些能現在做，哪些要等 / 協調

| 問題 | 本質 | 碰不碰 gate crediting？ | 建議 |
|---|---|---|---|
| 一 · 死資產標記 | targets 加 `liveness_state` + 下游剔除 | P1/P2/P4 不碰；**P3 碰分母注入** | P1–P2、P4 現在做；P3 與 gate 任務對齊 |
| 二 · 發現階段歸屬 + wave 重跑 | targets/ports 記「哪階段發現」+ 新資產自動新開一波 stage_run | 幾乎不碰（改的是分母的**資產集**與**波次調度**，不是 found 判定） | **現在可規劃 + 動手** |
| 三 · 工具職責重疊 | httpx/nmap 一專多能，但 gate 只按「單一 technique」給 found，逼補刀 | **就是 gate crediting 根因** | **歸另一個 AI**；我出證據簡報（§3） |
| 四 · 前端指紋顯示 | fingerprints 資料前端有、新 workbench 沒渲染 | 完全不碰 | **現在可動手，最小改動** |

**對「等不等那邊」的回答**：不要全域等。問題四、二與 gate 任務解耦，現在推進不會衝突；問題三讓那個 AI 主導、我把我讀到的精確根因（§3）作為輸入交過去，反而能幫它省勘驗時間；唯一要**同步落地順序**的是「死資產 P3 分母剔除」「問題二的 wave 分母凍結」都改到 `execute.rs` 的覆蓋率分母注入 + `stage_spec` flag——這三處誰先落，另一個 rebase 即可（見 §4 共用觸點）。

---

## 1. 問題二 · 發現階段歸屬 + wave 重跑

### 1.1 現狀（帶證據）

- **沒有「在哪階段發現」欄位**：`targets` 只有 `status`(階段進度 enum)、`created_at`、`source`(=`automated`/`ai`/`manual`，migration `20260409000003_operation_source.sql`，**不是 stage**)。埠存於 `targets.ports`(JSONB) / `network_endpoints` / `target_assets.port`，皆無發現階段歸屬。
- **分母凍結已做**：`asset_wave_barrier=true` + `asset_wave_cutoff_for_gate`（`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs:1421`）用 `stage_started_at` 前的資產當分母，新資產不灌當前卡。
- **自動新開下一波未做**：`stage_asset_waves`/`stage_asset_wave_items`（migration `20260625000001`）與 `expansion_queue`（migration `20260623000004`）表已建，但註解明說 *"Inert until write/read wiring lands"* / *"Runtime starts using these rows only after repo/trait wiring lands"*、*"automatic next-wave dispatch is deferred"*。

→ 使用者要的行為 = 凍結分母 ✅ 已有 + 新資產新開一波 ❌ 未接線。

### 1.2 方案（兩塊，可分別做）

**A. 發現階段歸屬（讓每個資產/埠帶「哪階段發現」）**

- 首選**復用既有 `stage_asset_wave_items`**（已有 `source` 列 + 其 wave 帶 `stage_kind`）作為「資產×階段」歸屬的事實源，而不是在 `targets` 加新列——因為一個資產可能在多階段被再確認，掛在 wave item 上比塞 targets 單列更貼。
- 若要在 target 卡直接顯示「首次發現階段」，再加一個 denormalized `targets.discovered_stage TEXT`（nullable，I10），寫點在資產首次落庫時蓋（`manage_targets` create / recon landing）。
- 埠層級歸屬：`targets.ports[].discovered_stage`（在 `update_recon_extended_by_id` 合併 ports 時，對新增的埠打當前 stage 標）。

**檔案**：migration（新 `discovered_stage` 列，若採 denormalized）、`golish-db/src/repo/targets.rs`（寫點）、`golish-db/src/models/pentest.rs` + `golish-app-core/.../targets.rs`（欄位 + ts-rs）、前端 target 卡渲染。

**B. 新資產自動新開一波 stage_run（把 deferred 的接線做出來）**

- 目標：一個 org 的 stage_run 跑完自己那波、EAS 掃出新資產後，**對新資產再 dispatch 一次同 stage 的 stage_run（新 wave）**，而不是灌回當前卡分母 retry。
- 建立在既有表上：一波結束時，把 `created_at > 本波 stage_started_at` 且未歸屬任何 wave 的 in-scope 新資產，寫進一個新的 `stage_asset_waves`(wave_index+1, parent_wave_id=本波) + `stage_asset_wave_items`，然後觸發 stage_run fan-out 消費下一波。
- **需先讀碼確認的接線點**（實作前必做，勿假設）：`backend/crates/golish/src/stage_run/`（fan-out 調度）、`execute.rs` 的 stage 完成/closeout 路徑（`:1607` 一帶）、以及 wave repo trait 尚未接的位置。收斂條件（避免無限開波）要定：wave 深度上限 / 新資產數為 0 即停 / 去重（`stage_asset_wave_items` 的 `UNIQUE(wave_id,target_id)` + 跨波去重）。

**風險 / 邊界**：波次爆炸（每波又發現新資產）→ 必須有深度上限與「無新資產即終止」；與死資產 P3 的分母剔除協同（dead 資產不該進下一波）。

### 1.3 分期

1. P1：發現階段歸屬（A，denormalized `discovered_stage` + 埠標記 + 前端顯示）——低風險、獨立。
2. P2：wave repo trait 接線（讀 `stage_asset_waves`/items）——先 inert 讀，不改調度。
3. P3：新資產 → 下一波寫入 + fan-out 消費（**這期才改行為**，灰度：先只寫 wave 表 + 日誌，再開自動 dispatch）。

---

## 2. 問題四 · 前端指紋顯示（最小改動，純前端）

### 2.1 現狀（帶證據）

- 後端 `fingerprints` 表齊全（`category/name/version/cpe/confidence/evidence`，`pentest.rs:342`）。
- 前端**資料其實已經在 VM 裡**：`surfaceHierarchy.ts:858` `attachFingerprintEvidence` 把 fingerprints 掛到 `WebOriginVM.fingerprints`（`:145`）。
- **只有舊的 `SurfaceTabView.tsx:125-151` 有 Fingerprints 區塊**；新 workbench 的 `NetworkEndpointsTab.tsx:64` 只渲染 `endpoint.service`（= network_endpoints 的 name+product+version 拼接），沒帶指紋。

→ 根因 = 前端渲染缺口，資料已到 VM，不需動後端。

### 2.2 方案

- 在新 workbench 補一個指紋呈現位。兩個選項：
  1. **WebOriginsTab / IdentityTab 加 Fingerprints 區塊**（複用 `SurfaceTabView` 那段渲染邏輯，讀 `origin.fingerprints`）——最省，因為指紋已掛在 origin 上。
  2. NetworkEndpointsTab 的 Service 欄加「詳情」展開，顯示該 host 對應的 fingerprints（version/cpe/confidence/evidence）。
- 建議先做選項 1（origin 級指紋區塊），因為 VM 已就緒；endpoint 級要先確認 fingerprints 能否 join 到 endpoint（目前掛在 origin，未必有 endpoint 綁定）。

**檔案**：`frontend/components/TargetPanel/surface/tabs/WebOriginsTab.tsx`（或 `IdentityTab.tsx`）加區塊；必要時把 `SurfaceTabView.tsx` 的 fingerprint 卡抽成共用小元件。純前端，`just check-fe && just test-fe` 收口。

**風險**：無後端 / schema / gate 觸動；最低風險，可作為「先出一個看得見的成果」的首發。

---

## 3. 問題三 · 工具職責重疊 → 交接給 gate 任務的證據簡報

> 這一塊是你已經交給另一個 AI 的「gate 判斷」任務的**同一個根因**。我不在此另寫實作方案（會和那邊重工/衝突），只把我讀到的精確證據整理成輸入，幫那個 AI 少走勘驗。

### 3.1 你給那個 AI 的描述可以更精確

你說「gate 好像是按『有沒有調用工具』判斷」。我讀碼後的更精確結論：**recon/EAS 的 gate 主體是讀 DB 業務表真值（`facts_from_db_truth` / `coverage_complete.authoritative_found`），不是看『工具被調用』**。真正卡你的是**「技術類 → 認哪張表/哪個欄位」的映射太窄**：

- **SERVICE-FINGERPRINT 的 `found` 只從 `fingerprints` 表補**（`coverage_truth.rs` `TECH_EAS_SERVICE_FP`；`evidence_facts.rs:199` 只有 `whatweb`/`nmap -sV` → SERVICE-FINGERPRINT）。
- 但 **httpx 已經把指紋類資料寫進 `targets.webserver` / `http_title` / `content_type`**（`update_recon_extended_by_id`，`targets.rs:549-553`），這些欄位**不參與 SERVICE-FINGERPRINT 的 found 判定**。
- 於是：httpx 一次跑已經拿到 server/title/tech，DB 也有了，但 SERVICE-FINGERPRINT 格仍 not_attempted → gate 逼你**再單獨跑 whatweb / nmap -sV 補 `fingerprints` 表**。這正是你說的「工具具備多能力、已寫入某些欄位、但 gate 還過不去、要單獨補刀」。

### 3.2 命令派生那一路也確實「按工具」

`evidence_facts.rs::eas_facts_from_command`（`:175`）是**按工具名 + flag** 把一次運行映射到**單一** technique：`httpx→LIVENESS`、`nmap -sV→SERVICE`、`nmap 無flag→PORT`、`naabu→PORT`。所以一個一專多能的工具（httpx 同時產 liveness + fingerprint-ish 資料）在這條路只被記一種 technique，另一種能力被丟。

### 3.3 我建議給那個 AI 的方向（供參考，不代替它決策）

- **讓 crediting 以「能力/落庫欄位」為準，而非「跑了哪個工具」**：SERVICE-FINGERPRINT 的 DB 真值判據擴到「`fingerprints` 有行 **OR** `targets.webserver`/`http_title`/`os_info` 等指紋欄非空」。這樣 httpx 落的指紋欄也能補 SERVICE-FINGERPRINT 格，不必補刀。
- 或**在 httpx 落庫時同時寫一條 `fingerprints` 行**（webserver→category=server 的 fingerprint），讓單一事實源 `fingerprints` 保持權威。
- 兩條路二選一即可；關鍵是**「工具能力 → 落庫欄位 → 技術格 found」三者對齊**，而不是「技術格認死某一個工具」。

### 3.4 共用觸點（兩邊都會改，需協調）

- `backend/crates/golish-db/src/repo/coverage_truth.rs`：技術類 → DB 真值判據（SERVICE-FINGERPRINT 擴欄位就在這）。
- `backend/crates/golish-agent-kit/src/harness/evidence_facts.rs`：命令 → technique 映射。
- `resources/harness/stages/external_attack_surface/spec.json`：`coverage_complete.authoritative_techniques` / `derive_from_evidence`。
- `resources/harness/technique_taxonomy.json`：技術類詞典。

---

## 4. 與 gate 任務的協調（落地順序）

三處**共用觸點**，誰先落誰後 rebase：

1. **`execute.rs` 覆蓋率分母注入**（`:1389` 一帶）：死資產 P3（剔 dead）、問題二 wave（凍結/下一波）都改這裡。→ 建議死資產 P3 與問題二 B 由同一輪一起改，避免互相踩。
2. **`stage_spec.rs` + 各 stage `spec.json` 的 flag**：死資產加 `skip_dead_assets`、gate 任務可能改 `authoritative_techniques`。→ 各加各的 bool，互不覆蓋，衝突面小。
3. **`coverage_truth.rs` 技術判據**：gate 任務（問題三，擴 SERVICE-FINGERPRINT 判據）主改；死資產只新增 `dead_asset_values` 查詢，不動既有技術判據。→ 低衝突。

**建議節奏**：
1. **立刻並行**：問題四（純前端，先出可見成果）+ 死資產 P1/P2（加列+寫值，inert）。
2. **等 gate 任務有結論後**：再落死資產 P3（分母剔除）+ 問題二 B（wave 下一波），因為這兩個和 gate 的分母/crediting 語義最貼，讓 gate 方向定了再改一次分母注入，避免改兩遍。
3. **問題三**：不由我做，等那個 AI 的方案；我把 §3 的證據簡報同步過去（或你轉給它）。

---

## 5. 下一步（我這邊）

- 若你同意這個節奏，我**現在就能開工**的低風險項：問題四前端指紋（選項 1）、死資產 P1（migration+回填，需你確認 schema）、問題二 A（發現階段歸屬）。
- 需要等 / 協調的：死資產 P3、問題二 B、問題三——待 gate 任務結論。
- 這幾份都還沒動代碼。要不要我先把**問題四前端指紋**做掉（不碰後端、不碰 gate、最快看到效果）？
