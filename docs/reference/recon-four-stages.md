# 偵察四階段參考：scoping → target_intel → external_attack_surface → enumeration

> **一句話定位**：這份文件把 harness 前四個偵察階段（範圍界定 → 被動情報 → 主動攻擊面 → 內容枚舉）的「邏輯 / 產出數據 / 用到的工具 / 工具對應哪個字段」整理成一份可長期複用的速查表，給 pentest 操作者與後續實作者（vuln_triage 及之後階段尚未實作）。
>
> **證據來源（本文件每條結論都可回溯到這些檔案）**：
> - stage spec / methodology：`resources/harness/stages/{scoping,target_intel,external_attack_surface,enumeration}/{spec.json,methodology.md}`
> - phase 分組：`resources/harness/graph/phases.json`；base DAG：`resources/harness/graph/operation_graph.json`
> - 技術類詞典：`resources/harness/technique_taxonomy.json`；證據時效：`resources/harness/evidence_kinds.json`
> - 工具白名單引擎：`backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`
> - 階段重排決策：`docs/design/2026-06-09-active-stage-verify-first.md`
>
> **狀態**：截至 2026-07-02，這四個階段已實作並上線；`vuln_triage / attack_candidate / verification` 等下游階段設計/實作進行中，不在本文件範圍。

---

## 0. 全局模型（讀懂四階段前必須先懂的五件事）

### 0.1 兩級 phase / stage 分組

`resources/harness/graph/phases.json` 把 12 個 StageKind 分成 5 個 phase，本文件的四階段落在最前面兩個：

| Phase | 包含的 stage | 進入本 phase 的審批（entry_approval） | 是否對目標發包 |
|---|---|---|---|
| `prep` | `scoping`、`target_intel` | 無 | 否（L0 授權確認 / L1 被動情報） |
| `active_recon` | `external_attack_surface`、`enumeration` | `active_scan` | 是（主動掃描） |

base DAG 邊（`operation_graph.json`）：`scoping → target_intel → external_attack_surface → enumeration → vuln_triage`（EAS 與 enumeration 也各有一條 `→ reporting` 的早退邊）。

### 0.2 覆蓋率格子 = 資產 × 技術類

每個階段的「完成」不是自然語言宣稱，而是一個**矩陣**：每個 in-scope 資產 × 該階段的每個期望技術類（`expected_techniques`），每格都必須走到**終態**之一：

- `found` + `evidence_refs`：技術真跑、真有東西
- `checked_empty` + `evidence_refs`：技術真跑、真沒東西（**「已檢查為空」≠「未檢查」**，不變量 I8）
- `blocked` + note：沒 provider / 沒憑證 / 來源不可用（**終態，清空該格，不會再要求補**）
- `not_applicable` + note：該資產類別不適用此技術（如純 IP 不適用子域）

漏一格（既非上述任一終態）= `not_attempted` = gate **BLOCK**。

### 0.3 `found` 由資料庫真相裁決，不是自報

多數階段開了 `facts_from_db_truth` / `coverage_complete.authoritative_found`：**工具真跑、資料真落到指定 DB 表/欄位，gate 才把該格判 `found`**。手寫 `found` 格子不算數。因此交付物是「瘦交付物」——`found` 留給 DB 補，submit 時只填 DB 推不出的 `checked_empty` / `blocked` / `not_applicable`。

### 0.4 `freshness_window`（時效窗）

四階段都開了 `freshness_window`：coverage 的 DB 真相投影**只認「本次 stage-run 開始（`operation_state.stage_started_at`）之後」採集的資料**。上一輪跑留下的舊行，本輪不算數（需要寫路徑的 `*_collected_at` 戳記配合）。

### 0.5 工具白名單 = deny-by-default

`tool_taxonomy.rs` 把工具映射到 `(category, subcategory)`；每個 stage 的 `allowed_tool_types` 是**類型選擇器**（`recon`、`recon/http`、或具體工具名）。**沒被列到的掃描工具一律 BLOCK**。meta / 編排工具（`recon_map_assets`、`submit_stage_deliverable`、`query_target_data`、`manage_organizations`…）不在掃描分類內，走 guard 層豁免。

---

## 1. `scoping`（範圍界定 / ROE）

- **檔案**：`resources/harness/stages/scoping/spec.json` + `methodology.md`
- **風險等級**：`low`｜**findings_allowed**：`false`｜**specialist**：無（org 層級，非 fan-out）
- **allowed_tool_types**：`["recon/osint"]`（ENScan 等工商情報，用來「定義」範圍，不 probe 目標）
- **human_approval**：`scope_expansion` 之前｜**requires_stages**：無（DAG 入口）｜**allowed_next_stages**：`target_intel`

### 邏輯

鎖定「被授權測誰」，產出**組織樹（org tree）**——這是本階段唯一交付物。這是 L0：可以查工商登記（OSINT，用來釐清測誰），但**不 probe / resolve / scan 目標主機**（runtime 會擋掉 dig/whois/subdomain/port/http 這類掃描工具）。

- **路徑 A（範圍是公司名）**：
  1. `manage_organizations(action="list")` 先查庫複用（已有 root org 就進 REUSE 模式，不重建）
  2. 對每個公司 `recon_lookup_company` 正規化法定名（以企查查為準，**讓使用者點選，絕不憑記憶寫全名**）
  3. 問使用者子公司是否納入、持股門檻（`ask_human(choice)`），**不要自己決定門檻**
  4. 納入才 `recon_discover_subsidiaries`（帶 `min_ownership_percent`）→ 回傳候選陣列，不自動建
  5. `ask_human(input_type="unit_review", context={"organization_id":"<id>"})` 讓使用者勾選
  6. `manage_organizations(action="create_batch", names=[勾選的], parent_id=<root>)` 一次落庫
  7. **到此為止 → 不記 target、不呼叫 `scope_review`、不把子公司變成域名**
- **路徑 B（範圍是具體 host/IP/URL）**：建立所屬 org，一輪 `ask_human` 確認即可。個別 host **不在此記**（`manage_targets` 本階段被移除），留給 `target_intel` 從 task context 讀。

### 產出數據（DB 落點）

- `organizations` 表：root org + 子公司（`parent_id` 掛樹）。路徑 A **只產 org 樹**；路徑 B 才落極少量使用者提供的具體 target。

### 工具 → 技術類 → 字段產出

| 工具 | 產出 | 對應技術類 | DB 落點 |
|---|---|---|---|
| `manage_organizations(list)` | 複用檢查（讀既有 org） | — | 讀 `organizations` |
| `recon_lookup_company` | 正規化法定名（餵給建 org） | — | 無（中間結果） |
| `recon_discover_subsidiaries` | 子公司候選（`name`/`ownership_percent`/`meets_threshold`） | `GOLISH-INTEL-SUBSIDIARY` | 無（候選，待勾選） |
| `manage_organizations(create/create_batch)` | root + 子公司落庫 | `GOLISH-INTEL-SUBSIDIARY`（DB 真相） | `organizations.parent_id` |

### 覆蓋率與 gate 要點

- 唯一技術類 `GOLISH-INTEL-SUBSIDIARY`，**且僅在 engagement 開了 `--include-subsidiaries` 時**由 harness hook 動態注入。不開子公司時 `expected_techniques` 為空，`coverage_complete` 是 no-op，scoping 退化成純授權確認（零行為改變）。
- gate 讀的是 `organizations.parent_id`（DB 真相）：有真的子 org 行才 `found`；發現了但沒過門檻 = `checked_empty`+證據；**不許捏造子公司**。
- 紅線：絕不把不相關公司或公開靶站（vulnweb / testphp / acunetix demo）拉進範圍。

---

## 2. `target_intel`（被動情報，零接觸）

- **檔案**：`resources/harness/stages/target_intel/spec.json` + `methodology.md`
- **風險等級**：`low`｜**findings_allowed**：`false`｜**specialist**：`recon`（`stage_run` 每個 org fan-out 一個）
- **allowed_tool_types**：`[]`（**不暴露任何 CLI 掃描工具**，只走 provider / registry 工具）
- **coverage_axis**：`DNS / WHOIS / ASN / CT / SUBDOMAIN / OSINT`
- **human_approval**：`active_scan` 之前｜**requires_stages**：`scoping`｜**allowed_next_stages**：`external_attack_surface`
- **開關**：`facts_from_db_truth` / `freshness_window` / `host_aware_coverage` / `coverage_anchor_only` 全開

### 邏輯

對 in-scope 的 root 建立**被動、零接觸**的情報畫像：資產清單、子域、DNS-adjacent 事實、whois、ASN、CT、OSINT——**不發任何碰目標主機的封包**。存活 / 埠 / 服務探測不在這裡（那是 EAS）。一個 org 跑一個 recon 子代理，只收這個 org 的 footprint 並把資產登記成綁定該 `organization_id` 的 in-scope target。

推薦順序：
1. `recon_map_assets` 先跑也是主路徑——ASM/情報 provider（quake / 0.zone / fofa / hunter / shodan / enscan）一次回傳 org、ICP、子域、ASN、憑證、資產欄位，並把每個域名配對其偵測到的 IP、範圍過濾後落成帶 `real_ip` 的 in-scope target。
2. `recon_lookup_whois`——RDAP WHOIS，每 org 一次。
3. provider 拿不到就停在終態（`blocked`+note / `checked_empty`+證據 / `not_applicable`+note）；**不做 CLI fallback、不中途裝工具、不換 flag 重試**。

**`coverage_anchor_only` 很關鍵**：覆蓋率分母是「錨定的 root 資產」，被動枚舉出的子域**不會**在本階段把自己算進分母（否則會出現「每個 org 跑到 40 次上限仍 BLOCK」的跑步機效應）。

### 工具 → 技術類 → 字段產出（本階段最重要的映射）

| 工具 | 對應技術類 | DB 落點 |
|---|---|---|
| `recon_map_assets` | `GOLISH-INTEL-SUBDOMAIN`、`GOLISH-INTEL-DNS` | 子域/資產配對 IP → `target_assets`（帶 `real_ip`）；DNS 記錄（provider 給時）→ `dns_records` |
| `recon_map_assets` | `GOLISH-INTEL-ASN` | `organizations.asns` |
| `recon_map_assets` | `GOLISH-INTEL-CT` | `organizations.certificates` |
| `recon_map_assets` | `GOLISH-INTEL-OSINT` | `organizations.intel` |
| `recon_lookup_whois` | `GOLISH-INTEL-WHOIS` | `organizations.whois` |

### 覆蓋率與 gate 要點

- 6 技術類：`GOLISH-INTEL-DNS / -SUBDOMAIN / -ASN / -CT / -WHOIS / -OSINT`，每個 in-scope 資產都要覆蓋。
- `found` = DB/ledger 真相（provider 工具真跑真落庫）；打 tag 或手寫格子都不算。
- 額外 gate `source_coverage`：每個 provider-backed 格子要有終態的 `source_query_log` 行（來源嘗試證明）；`blocked`/`not_applicable`+note 是終態，不強迫再呼叫 provider。
- `host_aware_coverage`：純 IP/CIDR **不要求** SUBDOMAIN / forward-DNS / CT（那是 domain 概念）；WHOIS/ASN/OSINT 對所有類都要。
- 效率紅線：不要逐一 resolve 每個子域（那是 EAS 的活）；不要 `| head`/`| tail` 截斷輸出（截斷的無法入庫）；**不要拿一個技術的證據去補另一個格子**（DNS 證據只能補 DNS）——這是本階段反覆 `needs_fix` 的第一大原因。

---

## 3. `external_attack_surface`（EAS，主動定義攻擊面）

- **檔案**：`resources/harness/stages/external_attack_surface/spec.json` + `methodology.md`
- **風險等級**：`medium`｜**findings_allowed**：`false`｜**specialist**：`prober`
- **allowed_tool_types**：`["recon/port-scan", "recon/http", "recon/visual"]`
- **coverage_axis**：`LIVENESS / PORT / SERVICE`
- **human_approval**：`active_scan` + `exploit_validation` 之前｜**requires_stages**：`scoping`, `target_intel`｜**allowed_next_stages**：`enumeration`, `reporting`
- **開關**：`facts_from_db_truth` / `freshness_window` / `host_aware_coverage` / `asset_wave_barrier` 全開
- **這是第一個碰目標的階段**，受 `active_scan` 審批把守；子域從 `target_intel` **繼承**，不重新枚舉。

### 邏輯

對每個 in-scope 資產確立三件事：(1) 存活、(2) 開放埠、(3) 服務/版本指紋。覆蓋率驅動（非死板管線）：先拉繼承的種子（`list_attack_surface_seeds`）→ `check_stage_asset_coverage`/`query_target_data` 看現況 → 按資產類分流：

- `ip`：掃埠 + 存活
- `domain`：探存活；若解析到 in-scope IP target，其 PORT/SERVICE **委派給那個 IP**（掃 IP 一次即可，gate 會 drop 該 domain 的 PORT/SERVICE）
- `url`：只探 URL 存活，不指派 PORT/SERVICE
- `cidr`：先審批再掃段，把掃到的存活 IP 登記為具體 target 再逐一掃

`asset_wave_barrier`：本波分母凍結在 `stage_started_at` 已存在的資產；掃描中新發現的資產記錄為 `new_assets`/下一波 pending，不阻塞本波 gate。

### 工具 → 技術類 → 字段產出

| 工具 | 對應技術類 | DB 落點 |
|---|---|---|
| `httpx`（建議 `-json -sc -title -td -server -silent` 批次） | `GOLISH-EAS-LIVENESS` | `targets`（`http_status` / `real_ip`） |
| `naabu` / `masscan` / `nmap`（`-iL {{input_file}}`） | `GOLISH-EAS-PORT` | `targets.ports` |
| `nmap -sV`（僅對確認開放埠）/ `whatweb --input-file=` | `GOLISH-EAS-SERVICE-FINGERPRINT` | `fingerprints` |
| `gowitness file -f {{input_file}}`（可選截圖） | —（recon/visual） | 截圖存證 |

### 覆蓋率與 gate 要點

- 3 技術類：`GOLISH-EAS-LIVENESS / -PORT / -SERVICE-FINGERPRINT`。`found` 自 DB 真相自動補格。
- `host_aware_coverage`：裸 URL 端點只保 LIVENESS（其 host 的 PORT/SERVICE 歸 host/IP target）；domain/IP/CIDR 保全 3。
- **無開放埠 → SERVICE-FINGERPRINT 記 `not_applicable`+note**，不要捏造服務、也不要 `checked_empty total_units=0`。
- **HTTP 存活 ≠ PORT，也 ≠ SERVICE**；情報源資料（FOFA/Shodan）只是線索，必須當場主動確認（verify-first，見 `docs/design/2026-06-09-active-stage-verify-first.md`）。
- 工具缺失/報錯：記 `skipped_checks` + fallback（如 httpx 不可用改用 `nmap -sV` / `nmap -Pn -p- --open`），不要迴圈重試、不要中途裝工具；只有一個 target 失敗別把整批降級。
- submit 前先 `check_stage_asset_coverage`，`ready_to_submit=true` 才 `submit_stage_deliverable`（不要拿 submit 當試探）。

---

## 4. `enumeration`（內容枚舉，產出 vuln_triage 分母）

- **檔案**：`resources/harness/stages/enumeration/spec.json` + `methodology.md`
- **風險等級**：`medium`｜**findings_allowed**：`false`｜**specialist**：`enumerator`
- **allowed_tool_types**：`["recon/crawler", "web/route-probe"]`（**外部目錄 fuzzer ffuf/gobuster/feroxbuster/dirb/dirsearch 一律禁用**）
- **coverage_axis**：`JS / DIR / PARAM / JSAPI`
- **human_approval**：`active_scan` 之前｜**requires_stages**：`external_attack_surface`｜**allowed_next_stages**：`vuln_triage`, `reporting`
- **開關**：`freshness_window` / `host_aware_coverage` / `enum_ip_web_coverage` 開；`coverage_complete` 是 `derive_from_evidence`（**非 authoritative**，自報 checked_empty 仍兜底）
- **埠/服務已在 EAS 做完**，本階段只在「EAS 確認存活的 web 服務」上挖內容。

### 邏輯

只對 EAS 證明存活的 web root 做：JS/API 端點、目錄/路徑、參數。這些「可測單元（端點/參數）」就是下游 `vuln_triage` 的覆蓋率分母（`total_units`）來源。推薦流程：

1. `stage_worklist_status` → `stage_worklist_next(prefer=["pending","error"])` 拿本階段工作項（每項是一個 資產×技術 格子）
2. `list_enumeration_web_roots(include_coverage=true)` 補 web-root 上下文（帶 `target_id` + root URL + org 邊界）
3. `browser_collect_js_api` 先跑瀏覽器基線 + 收 JS → 之後 `js_extract_apis` 抽端點（回傳 redacted secret/config/framework 候選 + rule_matches + ai_analysis 行號供定向審查；**rule match 只當候選，不要憑 AI 推斷發明端點**）
4. 種子正規化（合併 browser 請求 / JS 端點 / crawler URL / HTML 連結表單 / well-known 路徑，正規化 host/scheme/port/query 名/尾斜線/靜態噪音）
5. `route_probe_paths` 每個 web root 跑一次（帶 `target_id` + `base_url`，讀 DB 種子 + 內建字典，遞迴展開已驗證目錄命中，拒 soft-404）
6. 參數：從觀察到的請求/表單/`param_hints` 抽，寫入 `api_endpoints.params`；**不預設主動隱藏參數爆破**

### 工具 → 技術類 → 字段產出

| 工具 | 對應技術類 | DB 落點 |
|---|---|---|
| `browser_collect_js_api` | `GOLISH-ENUM-JS`（JS 資產收集） | `js_analysis_results`（0 個 JS = `checked_empty`+瀏覽器證據） |
| `js_extract_apis` | `GOLISH-ENUM-JSAPI`（從 JS/crawler 抽端點） | `api_endpoints`（`source IN (js_analysis, crawler)`） |
| `js_extract_apis`（`param_hints`）/ 觀察到的請求表單 | `GOLISH-ENUM-PARAM` | `api_endpoints.params` |
| `route_probe_paths` | `GOLISH-ENUM-DIR` | `directory_entries`（絕對路徑，帶 `target_id`） |

### 覆蓋率與 gate 要點

- 4 技術類：`GOLISH-ENUM-JS / -DIR / -PARAM / -JSAPI`。
- `enum_ip_web_coverage`：domain/URL 一律是內容枚舉對象；**裸 IP/CIDR 只有在 EAS/httpx 用 `targets.http_status` 證明了 HTTP 服務時才納入**。
- **分母很重要**：`found`/`checked_empty` 格子要填 `tested_units` / `total_units`；要抽樣大面必須設 `sampling_rationale` 並達比例，否則算 partial → BLOCK（測 3/5000 個端點就宣稱 checked_empty 是假覆蓋）。
- 效率紅線：不重掃埠/不重指紋（reuse EAS）；只枚舉 EAS 確認存活的服務；`route_probe_paths` 每服務跑一次別換字典無限迴圈；**不要用外部目錄工具**。
- 交付物 `findings: []`，用 claims 總結（`web_root_enumerated` / `directories_discovered` / `api_endpoints_discovered` / `params_discovered` / `js_candidates_reviewed`），每條引真實 evidence id；**不要在 enumeration 用 `record_finding`**。

---

## 5. 跨階段數據流（一條線串起來）

```
使用者範圍 / 公司名
   │  scoping：recon_lookup_company + recon_discover_subsidiaries + manage_organizations
   ▼
organizations（org 樹，parent_id）
   │  target_intel：recon_map_assets + recon_lookup_whois（零接觸，per-org fan-out=recon）
   ▼
target_assets(+real_ip) / dns_records / organizations.{asns, certificates, intel, whois}
   │  external_attack_surface：httpx + naabu/masscan/nmap + whatweb/nmap -sV（active，fan-out=prober）
   ▼
targets(http_status, real_ip) / targets.ports / fingerprints  ← 存活的 web 服務清單
   │  enumeration：browser_collect_js_api + js_extract_apis + route_probe_paths（fan-out=enumerator）
   ▼
api_endpoints(+params) / directory_entries / js_analysis_results  ← 可測單元
   │
   ▼
= vuln_triage 的覆蓋率分母（total_units）
```

## 6. 工具 → 技術類 → DB 字段 主表（速查）

| 階段 | 工具 | 技術類 | DB 字段 |
|---|---|---|---|
| scoping | recon_discover_subsidiaries / manage_organizations | GOLISH-INTEL-SUBSIDIARY | organizations.parent_id |
| target_intel | recon_map_assets | GOLISH-INTEL-SUBDOMAIN / -DNS | target_assets(real_ip) / dns_records |
| target_intel | recon_map_assets | GOLISH-INTEL-ASN / -CT / -OSINT | organizations.asns / .certificates / .intel |
| target_intel | recon_lookup_whois | GOLISH-INTEL-WHOIS | organizations.whois |
| EAS | httpx | GOLISH-EAS-LIVENESS | targets.http_status / real_ip |
| EAS | naabu / masscan / nmap | GOLISH-EAS-PORT | targets.ports |
| EAS | whatweb / nmap -sV | GOLISH-EAS-SERVICE-FINGERPRINT | fingerprints |
| enumeration | browser_collect_js_api | GOLISH-ENUM-JS | js_analysis_results |
| enumeration | js_extract_apis | GOLISH-ENUM-JSAPI / -PARAM | api_endpoints / api_endpoints.params |
| enumeration | route_probe_paths | GOLISH-ENUM-DIR | directory_entries |

## 7. 覆蓋率格子狀態機（四階段通用）

| 狀態 | 意義 | 需附 | 是否終態 |
|---|---|---|---|
| `found` | 技術真跑且有結果 | evidence_refs（多為 DB 自動補） | 是 |
| `checked_empty` | 技術真跑但無結果（≠ 未檢查，I8） | evidence_refs（該技術自己的跑批證據） | 是 |
| `blocked` | 無 provider/憑證、來源不可用 | note（點名失敗/缺席的來源） | 是（清空該格，別重試迴圈） |
| `not_applicable` | 資產類別不適用該技術 | note | 是 |
| （未填） | not_attempted | — | 否 → gate BLOCK |

## 8. 各階段 gate 開關對照表

| 開關 | scoping | target_intel | EAS | enumeration |
|---|---|---|---|---|
| risk_level | low | low | medium | medium |
| allowed_tool_types | recon/osint | （空，provider-only） | port-scan/http/visual | crawler/route-probe |
| findings_allowed | 否 | 否 | 否 | 否 |
| facts_from_db_truth | —（authoritative subsidiary） | 是 | 是 | derive_from_evidence（非 authoritative） |
| freshness_window | — | 是 | 是 | 是 |
| host_aware_coverage | — | 是 | 是 | 是 |
| 特有開關 | 子公司 hook（--include-subsidiaries 才注入） | coverage_anchor_only / source_coverage | asset_wave_barrier / coverage_denominator | enum_ip_web_coverage |
| specialist（fan-out） | 無 | recon | prober | enumerator |
| human_approval before | scope_expansion | active_scan | active_scan + exploit_validation | active_scan |

## 9. 常見 BLOCK 原因與破解

| 症狀 | 根因 | 破解 |
|---|---|---|
| target_intel 反覆 needs_fix | 拿一個技術的證據補多個格子（DNS 證據當 ASN/CT/OSINT 用） | 每格引該技術自己那次跑批的 evidence_id |
| 某技術 provider 不可用一直迴圈 | 把 blocked 當非終態、反覆重試同 provider | 提交該格 `blocked`+note（點名缺席來源），blocked 是終態 |
| EAS SERVICE 格子填不上 | 無開放埠卻要 found，或 `checked_empty total_units=0` | 無開放埠 → `not_applicable`+note |
| enumeration checked_empty 被判 partial | 測了 3/5000 端點就宣稱空 | 填 `tested_units/total_units`，抽樣要設 `sampling_rationale` 並達比例 |
| 掃描工具被 BLOCK | 用了該階段 `allowed_tool_types` 外的工具（如 enumeration 用 ffuf） | 用階段允許的工具；deny-by-default |
| 上一輪資料不算數 | `freshness_window` 只認本次 stage-run 之後採集的資料 | 本輪重跑該技術讓資料重新落庫 |

## 10. 檔案索引（要改行為就改這些）

| 想改 | 檔案 |
|---|---|
| 某階段允許工具 / gate 規則 / 技術類 | `resources/harness/stages/<stage>/spec.json` |
| 某階段給 AI 的方法論指引 | `resources/harness/stages/<stage>/methodology.md` |
| phase 分組 / entry_approval | `resources/harness/graph/phases.json` |
| stage DAG 邊 | `resources/harness/graph/operation_graph.json` |
| 新增/改技術類 id | `resources/harness/technique_taxonomy.json`（改完有單測 fail-closed 檢查） |
| 證據時效閾值 | `resources/harness/evidence_kinds.json` |
| 工具 → category/subcategory 映射、白名單 | `backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs` |
| 6 個 gate check / rule engine | `backend/crates/golish-agent-kit/src/harness/gate/` |
| 為何埠前移 EAS / JS 移 enumeration | `docs/design/2026-06-09-active-stage-verify-first.md` |
