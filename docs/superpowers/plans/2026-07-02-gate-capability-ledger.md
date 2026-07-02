# Gate 從「動作」轉「狀態」：technique_outcomes 能力帳本 實現計畫（問題三 / gate）

> **隸屬總綱**：本計畫是跨階段 crediting 統一總綱的 **crediting 核心引擎**（Tier 2 = `technique_outcomes` 能力帳本）。總綱見 [`docs/design/2026-07-02-cross-stage-crediting-unification.md`](../../design/2026-07-02-cross-stage-crediting-unification.md)（§3 Tier 契約、§6.2 落地順序）。
>
> **面向 AI 代理的工作者：** 用 `.cursor/skills/executing-plans` 逐任務實現。每任務單獨 commit。
>
> **協調說明**：這份計畫的方向由另一位 AI 提出、我（本會話）驗證代碼後同意，並補了四條護欄。若那位 AI 也在推進 gate，本計畫可作為對齊基準；**兩邊改同一批觸點時以本計畫的四護欄為硬約束**。

**目標：** 把「覆蓋完成」的判定從「跑了哪個工具（命令字串→單一技術）」徹底轉成「DB/證據帳本裡這個 (資產×技術) 有沒有終態」；讓多能力工具跑一次就能關閉它覆蓋的所有技術格，消除「已寫 DB 卻要單獨補刀」。
**架構：** 不新造機制——`technique_outcomes` 物化表 + `execute.rs` 的 dual-read union 已在（`execute.rs:1783-1817`）。工作是：①修 technique_outcomes 投影的 freshness 漏洞；②讓多能力工具 handler 按「它真正覆蓋的每個技術」各 upsert 一行；③補 SERVICE-FINGERPRINT 讓 httpx 落的指紋欄也算數；④刪空轉的 min_invocations named_check；⑤在 upsert 覆蓋齊後才把 vuln_triage 收緊成 authoritative。
**技術棧：** Rust（golish-agent-kit harness / golish-db repo / golish-pentest-app 或掃描 handler / resources spec）。

---

## 四條護欄（硬約束，貫穿全計畫）

1. **upsert 必須由確定性工具 handler（Rust 代碼）寫，絕不讓模型自報。** technique_outcomes 的可信度前提 = 與「寫業務表」同級（handler 知道自己做了什麼）。現有 `direct/mod.rs:148` 只在確定性派生出 fact 時 upsert，守住這條。
2. **順序鐵律：某階段所有掃描 handler 的 upsert 補齊，才能翻 `authoritative_found`。** 否則「無業務表 + handler 沒 upsert」的技術類 = 永久 BLOCK 且模型無逃生口。
3. **canonical key**：每行 `asset` 必過 `canonical_asset_key().key`（EAS LIVENESS 用 endpoint key）。現有 `upsert_technique_outcome_impl`（`evidence.rs:87`）已做，handler 走這條就自動守住。
4. **technique_outcomes 投影要套 freshness**：`execute.rs:1788` 目前**沒**用 `run_start` 過濾（只 `db_truth_facts` 過了），同 session 舊 stage-run 的行會洩漏。翻 authoritative 前先修。

---

## 現狀（實讀證據）

- 命令派生一命令一技術：`evidence_facts.rs:129` `passive_intel_facts_from_command` 回 `Option<(technique,asset)>`，複合工具命中一個或 `None`。
- technique_outcomes 已 union 進 gate：`execute.rs:1783-1817`「始終」dual-read（無灰度）。upsert 已在多個寫點被呼叫（`bridge_config.rs`、`route_probe_paths.rs`、`js_extract_apis.rs`、`browser_collect_js_api.rs`、`output_store/endpoints.rs`、`recon persistence`、`direct/mod.rs` 自動派生）。
- trait/bridge 現成：`db_traits/repo.rs:453` `upsert_technique_outcome`；app impl `evidence.rs:76` `upsert_technique_outcome_impl`（已處理 canonical key）；投影 `technique_outcome_facts_impl`（`evidence.rs:185`）。
- vuln_triage：`spec.json:51` `coverage_complete.derive_from_evidence:true`、**無** `authoritative_found`；WSTG 10 類在 `coverage_truth.rs` 無業務表 → 靠 technique_outcomes + 自報。
- min_invocations 空轉：EAS spec `named_check min_invocations`（`external_attack_surface/spec.json:63`）、enumeration（`enumeration/spec.json:40`）；所有 `min_invocations:{}`。

---

## Phase 0 — 刪空轉條目（零風險）

### Task 0.1：移除 EAS/enumeration 的 `min_invocations` named_check

**檔案**：`resources/harness/stages/external_attack_surface/spec.json`（刪 `:63` 的 `{ "op": "named_check", "check": "min_invocations" }`）、`resources/harness/stages/enumeration/spec.json`（刪 `:40` 同條目）。因所有 `min_invocations:{}`，該 check 恆真、純誤導。

**驗證**：
```bash
cd backend && cargo test -p golish-agent-kit -- spec   # spec 载入/gate 规则解析不回归
just test-rust
```
> 若有測試斷言 gate_rules 含該條目，同步更新。`min_invocations_check.rs` 代碼保留（其它 spec 未來可能用），只刪 spec 掛載。
**Commit**：`chore(harness): drop vacuous min_invocations named_check from EAS/enum specs`

---

## Phase 1 — 修 technique_outcomes 投影 freshness（護欄 4，翻 authoritative 前置）

### Task 1.1：`list_for_run` / `technique_outcome_facts` 支援 run_start 過濾

**檔案**：`backend/crates/golish-db/src/repo/technique_outcomes.rs`——加一個帶 cutoff 的讀：
```rust
const LIST_FOR_RUN_FRESH_SQL: &str = "\
SELECT asset, technique, outcome, source, evidence_ids, collected_at \
FROM technique_outcomes \
WHERE organization_id = $1 AND run_id = $2 \
  AND ($3::timestamptz IS NULL OR collected_at >= $3) \
ORDER BY seq";

pub async fn list_for_run_fresh(
    pool: &PgPool, organization_id: Uuid, run_id: &str, since: Option<DateTime<Utc>>,
) -> Result<Vec<TechniqueOutcomeRow>> { … bind $3 = since … }
```
> `collected_at` NULL 的行在 `since=Some` 時被排除（保守，對齊 db_truth 的 `>= $2` NULL→false 語義）。`since=None` = 舊行為（presence-only）。

**檔案**：`golish-agent-app/src/ai/db_bridge/evidence.rs`（`technique_outcome_facts_impl` `:185`）+ trait `db_traits/repo.rs`——投影方法加 `since: Option<DateTime<Utc>>` 參數，轉呼 `list_for_run_fresh`。

**檔案**：`golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（`:1788`）——投影時傳入 `run_start`（已在同函數 `:1699` 算好，`freshness_window` on 時為 `Some(stage_started_at)`）：
```rust
        if let Some(org_id) = self.harness_org_id {
            let projected: Vec<EvidenceFact> = self.repo
                .technique_outcome_facts(org_id, sid, run_start)   // ← 新增 run_start
                .await
                …
```

**驗證**：
```bash
cd backend && cargo test -p golish-db technique_outcomes   # 新 SQL 断言含 collected_at >= $3
cargo test -p golish-agent-kit   # gate 投影不回归
just test-rust
```
**Commit**：`fix(harness): apply freshness window to technique_outcomes gate projection`

---

## Phase 2 — 多能力工具 handler 按覆蓋技術各 upsert（護欄 1/3）

> 目標階段：**vuln_triage**（最痛，WSTG 無業務表）。掃描 handler 每跑一個 (asset × WSTG 類) 的檢查，就 upsert 一行 outcome。

### Task 2.1：定位 vuln_triage 掃描落庫 handler

**步驟（讀碼，勿假設）**：vuln_triage specialist = `vuln_scanner`（`spec.json:15`）。找它跑 nuclei/nikto/wpscan/sqlmap（`web/scanner`/`web/cms`/`exploit/edb`）後**解析輸出落庫**的 handler——grep `record_finding`、`nuclei`、`EndpointTest`、`passive_scan_logs` 的落庫點（多半在 `golish-pentest-app/src/pentest_bridge/` 或 `golish-pentest/src/output_store/`）。確認每個掃描結果能對應到一個 `(asset, WSTG技術)`。

### Task 2.2：handler upsert technique_outcomes（每覆蓋類一行）

**檔案**：Task 2.1 定位到的 handler。跑完一個掃描（例如 nuclei 一批模板對一個 asset）後，對它**覆蓋到的每個 WSTG 類**：
```rust
// 伪代码：handler 已知 (asset, 本次覆盖的 technique 集, 是否命中)
for technique in covered_wstg_techniques {
    let outcome = if hit_for(technique) { "found" } else { "empty" }; // I8: 跑了没命中=empty
    if let Err(e) = repo.upsert_technique_outcome(
        org_id, run_id, asset, technique, outcome,
        Some(tool_name),           // source
        Some(&command_or_template),// query
        &evidence_ids,             // 指向刚落的 finding/EndpointTest 证据行
    ).await {
        tracing::warn!(target:"harness::evidence", error=%e, "vuln_triage technique_outcome upsert failed");
    }
}
```
- `covered_wstg_techniques` 由 handler **確定性**推導：nuclei 模板 tag → WSTG 類的映射表（例如 `sqli`→`WSTG-INPV-05`、`xss`→`WSTG-INPV-01`、`exposure`→`WSTG-CONF-05`、`ssl`→`WSTG-CRYP-03`、`default-login`→`WSTG-ATHN-02`、CVE 模板→`GOLISH-NDAY`…）。這張映射表放 `golish-agent-kit/src/harness/`（純函數 + 單測），**不是模型自報**。
- `outcome`：命中該類 = `found`；跑了該類模板但沒命中 = `empty`（I8）；工具對該類沒跑 = 不 upsert（保持 not_attempted）。
- `asset` 傳原值即可，`upsert_technique_outcome_impl` 會過 canonical key（護欄 3 自動守住）。

### Task 2.3：nuclei-tag→WSTG 映射純函數 + 單測

**檔案**：`backend/crates/golish-agent-kit/src/harness/wstg_mapping.rs`（新建）
```rust
/// nuclei/nikto 模板 tag / classification → 注册 WSTG technique id（vuln_triage 10 类）。
/// 只收录无歧义映射；未知 tag → None（不 upsert，保持 not_attempted，fail-closed）。
pub fn wstg_technique_for_tag(tag: &str) -> Option<&'static str> {
    match tag.to_ascii_lowercase().as_str() {
        "sqli" | "sql-injection" => Some("WSTG-INPV-05"),
        "xss" => Some("WSTG-INPV-01"),
        "rce" | "cmd-injection" | "command-injection" => Some("WSTG-INPV-12"),
        "idor" | "bola" => Some("WSTG-ATHZ-04"),
        "default-login" | "weak-password" | "brute" => Some("WSTG-ATHN-02"),
        "csrf" | "session" => Some("WSTG-SESS-02"),
        "exposure" | "config" | "misconfig" => Some("WSTG-CONF-05"),
        "ssl" | "tls" => Some("WSTG-CRYP-03"),
        "disclosure" | "info-leak" | "info" => Some("WSTG-INFO"),
        "cve" | "nday" => Some("GOLISH-NDAY"),
        _ => None,
    }
}
```
單測：每條映射 + 未知 tag→None + 大小寫。

**驗證**：
```bash
cd backend && cargo test -p golish-agent-kit wstg_mapping && just test-rust
```
**Commit**：`feat(harness): vuln_triage handler upserts technique_outcomes per covered WSTG class`

---

## Phase 3 — httpx 指紋也算 SERVICE-FINGERPRINT（消除 nmap/httpx 補刀）

> 直接回應用戶問題三例子：httpx 已把 server/title 寫進 `targets.webserver/http_title`（`targets.rs:549`），但 SERVICE-FINGERPRINT 的 found 只認 `fingerprints` 表 → 逼補 whatweb/nmap -sV。

### Task 3.1（二選一）：擴 SERVICE-FINGERPRINT 的 DB-truth 判據

**檔案**：`backend/crates/golish-db/src/repo/coverage_truth.rs`——`TECH_EAS_SERVICE_FP` 的 values SQL（現只讀 `fingerprints` 表），OR 上 `targets.webserver <> '' OR targets.os_info <> ''`（httpx 落的指紋欄），並套 freshness（`fingerprints.detected_at` / 或給 webserver 一個採集戳；若無戳則走 presence）。
> 風險：webserver 只是 server header，粒度比 nmap -sV 粗。若不想放寬 found 標準，改走 Task 3.2。

### Task 3.2（替代）：httpx handler 落一條 fingerprint 行

**檔案**：httpx 輸出落庫 handler（`update_recon_extended_by_id` 的呼叫方）——httpx 拿到 `webserver`/`server` header 時，除了寫 `targets.webserver`，也 INSERT 一條 `fingerprints`（`category='server'`, `name=<server>`, `source='httpx'`）。這樣單一事實源 `fingerprints` 保持權威，SERVICE-FINGERPRINT 自動 found，不必補刀。**建議選 3.2**（不放寬判據、保持 fingerprints 單源）。

**驗證**：
```bash
cd backend && cargo test -p golish-db && cargo test -p golish-agent-kit gate && just test-rust
# 预期：httpx 落库后 SERVICE-FINGERPRINT 格自动 found（无需 whatweb/nmap -sV）
```
**Commit**：`feat(eas): httpx server header lands a fingerprint row (no service-fp re-scan)`

---

## Phase 4 — 收緊 vuln_triage 成 authoritative（護欄 2，最後做）

> **前置條件**：Phase 2 的 vuln_triage handler upsert 已覆蓋 spec 的全部 10 個 WSTG 類，且 Phase 1 freshness 已修。否則不准做本 Phase。

### Task 4.1：vuln_triage 開 authoritative + 命令派生降級為兜底

**檔案**：`resources/harness/stages/vuln_triage/spec.json`——`coverage_complete` 加 `"authoritative_found": true`（配合 `facts_from_db_truth:true` 已在 `:17`）。此後 WSTG found 只認 technique_outcomes（handler 寫）+ 任何未來業務表，模型自報 found 不再算數（自報 checked_empty/blocked/not_applicable 仍是合法終態）。

**檔案**（可選，全域降級）：`evidence_facts.rs`——命令派生 `coverage_facts_from_command` 註記為「裸 run_pty_cmd/pentest_run 兜底」，並在 dual-read union 的優先級文檔化：DB業務表 > technique_outcomes(handler) > 命令派生(兜底) > 自報。代碼上 union 不需改（都是 additive），只把 min_invocations 那類「按動作」殘留清掉（Phase 0 已做）。

**驗證（關鍵——先證明不會永久 BLOCK）**：
```bash
cd backend && cargo test -p golish-agent-kit gate
# 新增测试：vuln_triage 一次多能力扫描 upsert 多个 WSTG 行 → 多格一起关闭；
#          某类 handler 没 upsert 且无业务表 → 该格 not_attempted（预期，验证逃生口=honest checked_empty）
just precommit
```
> **上線灰度**：先在一個 profile（如 `assessment`）開 vuln_triage authoritative，跑真實 run 看 gate 日誌 `merged technique_outcomes projection`，確認「一次多能力掃描關閉多格」成立、且沒有「明明掃了卻永久 BLOCK」，再推其它 profile。
**Commit**：`feat(harness): vuln_triage coverage authoritative on technique_outcomes`

---

## 落地順序

1. **Phase 0**（刪空轉）、**Phase 1**（freshness）——低風險、獨立，先做。
2. **Phase 2**（handler upsert）、**Phase 3**（httpx 指紋）——核心，做完驗證「一次多能力關多格」。
3. **Phase 4**（收緊 authoritative）——**必須**在 Phase 2 對該階段全類覆蓋後才做，且灰度上線。

> 與其它計畫的協調：本計畫改 `execute.rs` 投影（Phase 1）+ spec flag（Phase 0/4），與死資產 P3、問題二 wave 共用 `execute.rs` 分母/facts 注入 + spec flag。建議 **gate 計畫 Phase 0/1/2/3 先落**（它定義了 crediting 語義），死資產 P3 與問題二 B 再跟上，避免分母/facts 語義改多遍。

## 自檢

1. **規格覆蓋**：問題三「多能力工具跑一次卻要補刀」→ Phase 2（handler 多 upsert）+ Phase 3（httpx 指紋算數）覆蓋；「gate 按動作判定殘留」→ Phase 0 刪 min_invocations。
2. **四護欄落點**：①handler 寫（Phase 2 明寫「確定性 handler，非模型」）；②順序（Phase 4 前置條件）；③canonical key（走 `upsert_technique_outcome_impl` 自動）；④freshness（Phase 1）。
3. **無占位符**：nuclei-tag→WSTG 映射給了具體表；handler 定位標了「讀碼勿假設」的探查步驟（Task 2.1）。
