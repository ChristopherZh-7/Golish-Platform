# 跨階段 crediting 統一總綱：found/終態如何被裁決（傘設計）

- **作者**：BajieAsk-agent-4（接手 MCP-3 全棧工程師的 gate 設計脈絡）
- **日期**：2026-07-02
- **狀態**：Discussion Draft（總綱 / 傘設計，只讀勘驗未動代碼；落地走各子計畫的 writing-plans + `just precommit`）
- **讀者**：Golish 平台後續任何工程師 / AI agent
- **一句話定位**：把「一個覆蓋格（資產 × 技術）憑什麼判 `found` / 終態」這件事，從各階段各說各話，收斂成**一份跨階段統一契約**；並把 2026-07-01 起散落的六份 gate/recon/attack 文檔按「crediting 旋鈕 vs 分母旋鈕」歸位，定出**先定 crediting 再動分母**的落地順序與一道防退化的 CI 鎖。

> **本文不重複各子計畫的實作細節**（那些在各自的 plan 裡）。本文只做三件事：① 定義統一契約；② 給每個階段一個「目標態」並標出差距；③ 把六份文檔掛到同一總綱下、排落地順序。

---

## 0. 決策（TL;DR）

1. **統一契約 = found 權威三層（Tier）**：一個覆蓋格要判 `found`，只認兩種**確定性真值來源**——**Tier 1 業務表真值**（`coverage_truth.rs`，工具無關）或 **Tier 2 `technique_outcomes` 能力帳本**（確定性 handler upsert）。**Tier 3 自報負終態**（`checked_empty` / `blocked` / `not_applicable`）永遠只能關「非 found」的格，**絕不**用於 `found`。命令字串派生（`evidence_facts.rs` 按工具名映射單一技術）降級為裸 shell 兜底。
2. **每階段統一動作**：對每個 `expected_technique`，必須聲明它的 found 由 Tier 1 或 Tier 2 承接；然後把 `facts_from_db_truth` + `coverage_complete.authoritative_found` + `freshness_window` 三開關**全開**；並刪掉 `min_invocations` / `surface_coverage` 這類「按動作（跑沒跑某工具）」的殘留 check。
3. **兩個正交旋鈕，不要一起擰**：**crediting**（格怎麼判 found/終態）與**分母**（哪些資產、哪一波進矩陣）是兩件正交的事，共用 `execute.rs` 的注入觸點。**先把 crediting 契約定死並落地，再動分母**（死資產剔除、delta 波次），否則同一段語義要改兩遍、且互相踩。
4. **一道 CI 鎖**：加 spec 級不變量測試——每個階段的每個 `expected_technique` 都必須聲明了 found 權威來源（Tier 1 表或 Tier 2 handler upsert 點），否則 CI 紅。防止未來新增技術類時悄悄退回自報 / 命令派生。
5. **範圍**：本總綱覆蓋信息收集四階段（scoping / target_intel / EAS / enumeration）+ 攻擊段公式化掃描（vuln_triage）。attack_candidate / verification 是**狀態機範式**（candidate 逐項終態），不吃覆蓋矩陣的 crediting，屬本總綱的**顯式邊界外**（見 §7）。

---

## 1. 背景：為什麼 gate「一階段一個樣」

用戶原始痛點（2026-07-02，MCP-3 會話）：

> 「gate 判斷好像是要根據是不是調用了工具去判斷，這樣會導致某些工具具備多個能力、已經寫入數據庫某些字段了，但 gate 還是過不去，所以要單獨調用某個工具去補這個字段。」
>
> 「不光是 vuln 了，每個階段都應該統一一下，其實我現在的重心在 EAS 跟枚舉階段。」

拆開看是**兩個層次**的問題：

- **表層症狀**：多能力工具（httpx 一次拿 liveness + server header + title）跑完、DB 也寫了，但覆蓋格不亮 → 逼著再單獨跑 whatweb / nmap -sV 補刀。
- **深層根因**：「一個格憑什麼判 found」在四個階段有**四套不同標準**——有的認 DB 業務表、有的認命令字串映射、有的還允許模型自報。標準不統一，就沒有一個「跑一次多能力工具就關閉它覆蓋的所有格」的通用保證。

本總綱要解決的是**深層根因**：先統一 crediting 契約，症狀（補刀）自然消失。

---

## 2. 現狀勘驗（實讀證據，2026-07-02）

### 2.1 gate 已有的 crediting 通道（`coverage_complete` 的 found 判定）

`backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` 的 `CoverageComplete`（`:51`）有四個相關旋鈕：

| 旋鈕 | 行 | 語義 |
|---|---|---|
| `derive_from_items` | `:59` | technique 標注的 claim/finding 當 found（**自報路徑**） |
| `derive_from_evidence` | `:66` | `ctx.evidence_facts` 的 Found 事實補格、Empty 事實補 checked_empty |
| `authoritative_found` | `:72` | **收緊**：found 只認 evidence_facts 的 Found 事實，自報 cell / tagged claim 不再算 found |
| `authoritative_techniques` | `:76` | 只對清單內技術收緊（None = 全部期望技術），灰度用 |

判定核心在 `:709-710`：`authoritative = authoritative_found && authoritative_techniques.is_none_or(命中)`；`:741-748` 據此讓 found 走「只認 `has_fact(Found)`」還是「自報 cell || tagged || derive」。

### 2.2 `evidence_facts`（gate 消費的真值事實）從哪來——三條並存投影

`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（`build_coverage_evidence_facts` 一帶）把三條來源 **union** 成 `ctx.evidence_facts`：

| # | 來源 | 行 | freshness | 性質 |
|---|---|---|---|---|
| ① | audit_log 帳本派生（`evidence_facts_for_session`） | `:1713` | 帳本行自帶 | 命令/handler 落賬 |
| ② | **DB 業務表真值**（`db_truth_facts` → `coverage_truth.rs`） | `:1750` | ✅ 套 `run_start`（`:1699`） | **Tier 1** |
| ③ | **technique_outcomes 帳本**（`technique_outcome_facts`） | `:1788-1817` | ❌ **沒套 `run_start`** | **Tier 2** |

> **關鍵漏洞（護欄 4，見 gate-capability-ledger 計畫 Phase 1）**：③ technique_outcomes 投影**始終** dual-read（`:1783` 註解「無灰度」），但**沒有**用 `run_start` 過濾，同 session 的舊 stage-run 行會洩漏。翻 authoritative 前必須先修。

### 2.3 Tier 1 業務表真值的覆蓋範圍（`coverage_truth.rs`）

`assemble_truth_facts_typed`（`:402`）+ `TruthInputs`（`:373`）目前投影的技術：

- 被動情報：ASN / CT / WHOIS / OSINT（org 級）、SUBDOMAIN / DNS（per-asset）、RDNS / IPWHOIS（IP 專屬）
- 主動攻擊面：EAS-LIVENESS（`targets.http_status/real_ip/ports`）、EAS-PORT（`targets.ports`）、EAS-SERVICE-FINGERPRINT（**只認 `fingerprints` 表**，`fingerprint_exists_sql` `:214`）
- 內容枚舉：ENUM-JS（`js_analysis_results`，設計已定 four-axis）、ENUM-DIR（`directory_entries`）、ENUM-PARAM（`api_endpoints.params`）、ENUM-JSAPI（`api_endpoints`）

**沒有業務表的技術類**：vuln_triage 的 10 個 WSTG 類（SQLi/XSS/IDOR/…）——它們**沒有** Tier 1，目前只能靠命令派生 + 自報。

### 2.4 四階段 crediting 現況對照（本總綱的差距地圖）

| 階段 | `facts_from_db_truth` | `authoritative_found` | `freshness_window` | Tier 1 覆蓋 | 現況判定 |
|---|---|---|---|---|---|
| `scoping` | —（subsidiary 特例） | ✅（spec `:41`） | — | SUBSIDIARY（org 樹） | ✅ 已統一 |
| `target_intel` | ✅（spec `:95`） | ✅（spec `:51`） | ✅ | 6 類全有表 | ✅ **樣板** |
| `external_attack_surface` | ✅（spec `:106`） | ✅（spec `:68`） | ✅ | LIVENESS/PORT ✅；**SERVICE-FP 判據窄** | ⚠️ 補刀源 |
| `enumeration` | ❌ | ❌（只 `derive_from_evidence` spec `:43`） | ✅ | JS/DIR/PARAM/JSAPI 有表但**未翻 authoritative** | ⚠️ 仍允許自報 found |
| `vuln_triage` | ✅（spec `:17`） | ❌（只 `derive_from_evidence` spec `:52`） | ✅ | **10 類 WSTG 無表、無 upsert** | ❌ 假全面風險 |

> `target_intel` 是唯一「三開關全開 + 全技術有 Tier 1」的**完成態樣板**。其餘三個階段各缺一塊：EAS 缺 SERVICE-FP 的權威判據對齊；enumeration 有表但沒翻 authoritative；vuln_triage 連 Tier 1/2 都沒有。

### 2.5 「按動作」的殘留 check（要清）

`NamedCheckKind`（`rule_engine.rs:147`）有 `Scope / SurfaceCoverage / MinInvocations`。其中：

- `min_invocations`：EAS / enumeration spec 都掛了 `{ "op": "named_check", "check": "min_invocations" }`，但 `min_invocations:{}` **全空** → 該 check 恆真、純誤導（enumeration spec `:40`、`:65`）。
- 這類「有沒有跑過工具 X」的判定，與統一契約（看**狀態**不看**動作**）方向相反，應隨各階段翻 authoritative 一併清掉。

---

## 3. 統一契約：found 權威三層（Tier）

這是本總綱的**核心規範**。任何階段的任何 `expected_technique`，其覆蓋格的終態裁決一律按下表：

| Tier | 來源 | 誰寫 | 可判 | 判定通道 |
|---|---|---|---|---|
| **Tier 1** | 業務表真值 | 工具落庫 handler（工具無關，只看表有沒有新鮮行） | `found` | `coverage_truth.rs` → `db_truth_facts` |
| **Tier 2** | `technique_outcomes` 能力帳本 | **確定性** handler upsert（每個真做的 `(asset, technique, outcome)` 一行） | `found` / `empty` / `error` | `technique_outcome_facts` dual-read union |
| **Tier 3** | 自報負終態 | 模型 submit | 僅 `checked_empty` / `blocked` / `not_applicable` | `coverage` cell（受 `require_note_for_other` 把關） |
| ~~兜底~~ | 命令字串派生 | `evidence_facts.rs` 按工具名映射 | 降級為裸 shell 兜底 | 保留但不作為主判據 |

### 3.1 三條紅線（貫穿所有階段）

1. **found 只能來自 Tier 1 或 Tier 2**。Tier 3 自報**永遠**不能構成 found（守 I7 證據可追溯、堵「dig 輸出冒充 whois」類假 found）。
2. **Tier 2 必須由確定性 handler（Rust 代碼）寫，絕不讓模型自報 upsert**。可信度前提 = 與「寫業務表」同級（handler 知道自己這次真做了什麼）。這是 Tier 2 能與 Tier 1 平權的唯一理由。
3. **「跑了→空」與「沒跑」嚴格區分（I8）**。工具對某技術跑了沒命中 = `empty`（Tier 2 upsert `empty` 或 Tier 3 自報 checked_empty + 證據）；工具對某技術根本沒跑 = 不 upsert、保持 `not_attempted` → gate BLOCK（fail-closed，不放水）。

### 3.2 為什麼這樣就消除「補刀」

用戶的補刀症狀 = 「httpx 寫了 `targets.webserver`，但 SERVICE-FINGERPRINT 只認 `fingerprints` 表」。統一契約下有兩條合規解（見 EAS 落地 §5.2）：

- **走 Tier 1**：擴 SERVICE-FP 的業務表判據，或讓 httpx 落庫時順手寫一條 `fingerprints` 行（單一事實源保權威）。
- **走 Tier 2**：httpx handler 對它這次確實覆蓋的 `(asset, SERVICE-FINGERPRINT, found)` upsert 一行。

無論走哪條，**「工具能力 → 落庫 → 技術格 found」三者對齊**，跑一次多能力工具就關閉它覆蓋的所有格，不再補刀。

---

## 4. 每階段統一動作（目標態清單）

把任一階段升到「完成態樣板」（= target_intel）需要三步：

1. **每個 `expected_technique` 落 Tier 1 或 Tier 2**：有業務表的走 Tier 1；沒業務表的（WSTG 類）由掃描 handler upsert `technique_outcomes`（Tier 2）。
2. **三開關全開**：`facts_from_db_truth: true` + `coverage_complete.authoritative_found: true` + `freshness_window: true`。灰度期可用 `authoritative_techniques` 只收緊「Tier 已就緒」的技術，其餘暫留自報。
3. **清「按動作」殘留**：刪 `min_invocations` / `surface_coverage` 這類空轉或動作導向的 named_check。

> **順序鐵律（護欄 2）**：某階段所有掃描 handler 的 Tier 1/2 落點補齊，才能翻該階段的 `authoritative_found`。否則「無表 + handler 沒 upsert」的技術類 = 永久 BLOCK 且模型無逃生口。

---

## 5. 各階段落地差距與掛接的子計畫

### 5.1 target_intel — ✅ 已是樣板，無需動

三開關全開、6 類全有 Tier 1。作為其它階段的參照基準。

### 5.2 EAS（用戶重心之一）— 對齊 SERVICE-FINGERPRINT 判據

- **差距**：LIVENESS/PORT 已 Tier 1 權威；SERVICE-FINGERPRINT 的 Tier 1 只認 `fingerprints` 表（`coverage_truth.rs:214`），httpx 落的 `targets.webserver` 不算 → 補刀。
- **落地**：走 gate-capability-ledger 計畫 **Phase 3.2**——httpx 落庫時順手寫一條 `fingerprints` 行（`category='server'`, `source='httpx'`），保持 `fingerprints` 單一事實源。順手刪 EAS 的 `min_invocations` 死條目。
- **子計畫**：`docs/superpowers/plans/2026-07-02-gate-capability-ledger.md` Phase 3。

### 5.3 enumeration（用戶重心之二）— 四軸落地後翻 authoritative

- **差距**：四個階段裡**唯一還允許自報 found** 的（spec `:43` 只開 `derive_from_evidence`，沒開 `authoritative_found`）。JS/DIR/PARAM/JSAPI 都有 Tier 1 業務表，但沒翻權威。
- **落地**：先落 four-axis 設計（新增 `GOLISH-ENUM-JS` 真值 + IP-web 納入），把 JS 軸的 Tier 1（`js_analysis_results`）補齊；**四軸真值齊了之後翻 `authoritative_found`**，即與 EAS/intel 一致。
- **子計畫**：`docs/design|plans/2026-07-01-enumeration-four-axis-and-ip-web.md`（本總綱把它定位為「enumeration 的 crediting 落地 + JS 軸 Tier 1 補齊」）。

### 5.4 vuln_triage — 建 Tier 2 能力帳本（無業務表的解）

- **差距**：10 類 WSTG **無業務表**（無 Tier 1）、handler **無 upsert**（無 Tier 2），只能靠 `derive_from_evidence` + 自報 = 假全面風險。
- **落地**：走 gate-capability-ledger 計畫 **Phase 2**——掃描 handler（nuclei/nikto/…）對它覆蓋的每個 `(asset, WSTG類, outcome)` upsert `technique_outcomes`（Tier 2），配 `wstg_mapping.rs` 純函數（nuclei tag → WSTG id）。**全 10 類 upsert 補齊後**（護欄 2）才 Phase 4 翻 `authoritative_found`。
- **子計畫**：`docs/superpowers/plans/2026-07-02-gate-capability-ledger.md` Phase 2/4。
- **注意**：vuln_triage 正被 attack-stage 設計重定義為「公式化掃描」（§7），本總綱的 Tier 2 落地與那個重構是**同向且互補**的（那份設計 §3.2 也明寫「經工具執行層歸一化寫 `technique_outcomes`」）。

### 5.5 掛接矩陣（六份文檔各管哪一塊）

| 文檔 | 類型 | 管的旋鈕 | 在本總綱的角色 |
|---|---|---|---|
| **本文** | 設計總綱 | crediting 契約（傘） | 定義 Tier 契約 + 排序 + CI 鎖 |
| `2026-07-02-gate-capability-ledger.md` | 計畫 | **crediting 核心引擎** | Tier 2（technique_outcomes）+ freshness 修 + EAS/vuln 落地 |
| `2026-07-02-recon-gaps-followups.md` | 設計 | 協調地圖 | 問題二/三/四依賴分組（本總綱吸收其 §3/§4 協調結論） |
| `2026-07-01-enumeration-four-axis-and-ip-web.md` | 設計+計畫 | crediting（enum）+ 分母（IP-web） | enumeration Tier 1 補齊 + 翻 authoritative |
| `2026-07-02-dead-asset-liveness-state.md` | 設計 | **分母**（剔死資產） | 正交旋鈕，crediting 定死後再落 P3 |
| `2026-07-02-asset-discovery-stage-and-delta-wave.md` | 計畫 | **分母**（波次/歸屬） | 正交旋鈕，crediting 定死後再落 Part B |
| `2026-07-02-attack-stage-formulaic-candidate-exploit.md` | 設計 | 階段結構 + 攻擊段 crediting | vuln_triage 公式化沿用矩陣範式；candidate/verification 走狀態機（邊界外） |

---

## 6. 兩個正交旋鈕與落地順序

### 6.1 crediting vs 分母（共用觸點，但語義正交）

- **crediting 旋鈕**：一個格**憑什麼**判 found/終態（Tier 契約）。落點：`coverage_truth.rs`（Tier 1 判據）、`technique_outcomes` handler upsert（Tier 2）、spec 的 `authoritative_found`。
- **分母旋鈕**：**哪些**資產、哪一波進覆蓋矩陣。落點：`execute.rs` 的 in-scope 資產注入（`:1747` 一帶）、`stage_spec` 的 `skip_dead_assets` / `asset_wave_barrier` flag。

兩者共用 `execute.rs` 的注入觸點與 spec flag，所以**改動會互相踩**。

### 6.2 落地順序（硬約束）

```
1. 先定 crediting（低風險、獨立）
   ├─ gate-ledger Phase 0（刪 min_invocations 空轉）
   ├─ gate-ledger Phase 1（修 technique_outcomes freshness — 護欄 4）
   ├─ gate-ledger Phase 2（vuln_triage handler upsert Tier 2）
   └─ gate-ledger Phase 3（EAS httpx 落 fingerprint 行）
2. 各階段翻 authoritative（Tier 補齊後）
   ├─ enumeration：four-axis 落地 → 翻 authoritative_found
   └─ vuln_triage：Phase 2 全類補齊 → Phase 4 翻 authoritative_found（灰度）
3. 最後動分母（crediting 語義已凍結）
   ├─ 死資產 P3（剔 dead 出分母）
   └─ delta 波次 Part B（新資產下一波）
```

> **為什麼先 crediting 後分母**：分母改的是「矩陣有哪些格」，crediting 改的是「每個格怎麼判」。若先動分母，等 crediting 契約定了還要再改一遍 `execute.rs` 注入 + spec；反之先把 crediting 凍結，分母只是在穩定的判定語義上做集合增減，改一次即可。死資產 P3 與 delta 波次也天然要協同（dead 資產不該進下一波）。

---

## 7. 邊界：attack_candidate / verification 不吃這套契約

本總綱只管**覆蓋矩陣範式**（資產 × 技術 → 終態）的階段。attack-stage 設計把攻擊段拆成三段，其中：

- **vuln_triage（公式化掃描）= 矩陣範式** → **吃本總綱契約**（Tier 2 落地）。
- **attack_candidate（候選合成）= 推理範式** → gate 是 `candidate_grounded`（每假設有依據），不是覆蓋矩陣。
- **verification（真打驗證）= 狀態機範式** → gate 是 `candidate_disposition_complete`（每 candidate 逐項終態 verified/refuted/blocked），不是覆蓋矩陣。

`rule_engine.rs:128-141` 已有 `CandidateGrounded` / `CandidateDispositionComplete` 兩個 op。**這兩段的「完成」語義與 crediting Tier 契約正交**，不要硬套；本總綱在此劃清邊界，避免把矩陣 crediting 的規則誤加到狀態機階段。

---

## 8. 一道防退化的 CI 鎖

統一契約最怕「當下對齊、日後新增技術類又偷偷退回自報」。加一道 spec 級不變量測試：

- **斷言**：對每個 stage spec 的每個 `expected_technique`，該技術必須在一張「found 權威登記表」裡聲明了它的 Tier 1 業務表判據（`coverage_truth.rs` 有對應 SQL）**或** Tier 2 handler upsert 點（`wstg_mapping.rs` / 落庫 handler 覆蓋）。
- **否則 CI 紅**：新增 `expected_technique` 卻沒給它 found 權威來源 = 編譯期/測試期即失敗，逼開發者補 Tier 1/2，而不是讓它默默退回自報 / 命令派生。
- **落點建議**：`golish-agent-kit` 的 harness 測試（與 `technique_taxonomy` fail-closed 檢查同址），或新增 `harness/crediting_registry.rs` 純函數 + 單測。

> 這道鎖是本總綱相對各子計畫**新增**的護欄——各子計畫各自落 Tier，這道鎖保證「所有階段永遠不漏 Tier」。

---

## 9. 與 AGENTS.md 不變量對齊

- **I7（證據可追溯）**：Tier 1/2 皆落盤可追溯；found 永不來自自報（§3.1 紅線 1）。
- **I8（已檢查 ≠ 未檢查）**：跑了沒命中 = empty；沒跑 = not_attempted → BLOCK（§3.1 紅線 3）。Tier 2 的 outcome 精確區分二者。
- **I10（schema 向後兼容）**：Tier 2 用既有 `technique_outcomes` 表（無新 schema）；EAS 落 fingerprint 行為加性寫入；各 spec flag 缺省 false = 逐字節不變、可灰度回滾。
- **§2.7（gate BLOCKING 變更需 sign-off）**：翻任一階段的 `authoritative_found` = gate 行為變更，走灰度開關 + parity 測 + 合併前 sign-off。

---

## 10. 開放問題（實現前需拍板）

1. **Tier 2 outcome 的 `found` 是否對所有 WSTG 類都算權威**？attack-stage 設計 §11.3 建議先只對 NDAY/DIR/WEAKPW/TLS 這類確定性強的收緊，SQLi/XSS 工具命中先非權威（避免工具假陽性直接升 found）。本總綱傾向採納：用 `authoritative_techniques` 清單分批收緊。
2. **EAS SERVICE-FP 走 Tier 1 擴判據 vs Tier 2 upsert**？本總綱與 gate-ledger 計畫一致傾向 **Tier 1（httpx 落 fingerprint 行）**，保持 `fingerprints` 單一事實源、不放寬 found 標準。
3. **CI 鎖的「found 權威登記表」用手維護 vs 從 spec/代碼反射**？MVP 建議手維護一張 `(stage, technique) → Tier` 表 + 單測比對 spec 的 `expected_techniques`，語義反射 deferred。
4. **enumeration 翻 authoritative 的時機**：four-axis 兩個 PR（四軸拆分 / IP-web）都落地後一起翻，還是四軸拆分後即翻、IP-web 分母另算？建議 crediting（四軸真值）與分母（IP-web 納入）分開——先翻 crediting，IP-web 作為分母旋鈕隨後。

---

## 11. 驗證策略（本總綱落地後的 DoD 摘要）

各子計畫各自帶 TDD + `just precommit`；本總綱層面的驗收：

- **CI 鎖生效**：故意加一個沒有 Tier 來源的 `expected_technique` → 測試紅；補上 Tier 1/2 → 綠。
- **crediting parity**：翻 authoritative 前後，對「Tier 已就緒」的技術，gate 判定不因移除自報路徑而回歸（parity 測）。
- **補刀消除（端到端）**：一次多能力掃描（httpx / nuclei 批量）跑完，其覆蓋的多個格**一起**關閉，日誌 `merged technique_outcomes projection` / `merged DB business-table truth` 可見，無「明明掃了卻永久 BLOCK」。
- **順序無回歸**：先 crediting 後分母的順序下，死資產 P3 / delta 波次落地時 `execute.rs` 注入只改一次。

---

## 12. 關聯文檔

| 文件 | 作用 |
|---|---|
| 本文 | 跨階段 crediting 統一總綱（傘設計） |
| `2026-07-02-gate-capability-ledger.md` | Tier 2 引擎 + freshness 修 + EAS/vuln 落地（crediting 核心計畫） |
| `2026-07-02-recon-gaps-followups.md` | 問題二/三/四協調地圖（本總綱吸收其協調結論） |
| `2026-07-01-enumeration-four-axis-and-ip-web.md` | enumeration Tier 1 補齊 + IP-web 分母 |
| `2026-07-02-dead-asset-liveness-state.md` | 分母旋鈕：剔死資產 |
| `2026-07-02-asset-discovery-stage-and-delta-wave.md` | 分母旋鈕：發現階段歸屬 + delta 波次 |
| `2026-07-02-attack-stage-formulaic-candidate-exploit.md` | 攻擊段三階段重構（vuln_triage 公式化吃本契約；candidate/verification 邊界外） |
| `docs/reference/recon-four-stages.md` | 四階段工具→技術→字段速查（Tier 1 判據來源） |

> 下一步：用戶審查本總綱 → 拍板 §10 開放問題 → 各子計畫按 §6.2 順序用 writing-plans / executing-plans 落地。本總綱為新增獨立 markdown，不覆蓋舊文檔（AGENTS.md I6 / §2.4）。
