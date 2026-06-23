# 2026-06-22 · intel 阶段：每维精确新鲜度 + 瘦身交付物（DB 真值唯一源收尾）

> 日期：2026-06-22
> 状态：设计（待审 §8 + 用户 sign-off schema 改动 §2.7）
> 作者：BaJie BajieAsk-agent-3（全栈工程师）· 与用户对话产出（DISPATCH off）
> 关联：
> - `docs/design/2026-06-15-db-truth-single-source-deliverable.md`（本文承接其 **PR3 提交通道简化**，并补齐其 §139 自称「freshness 保留」但实现未覆盖 DB-truth 投影的缝）
> - `docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`（证据投影模型）
> - `docs/design/2026-06-12-redteam-phase0-db-truth-authoritative-gate.md`（authoritative_found）
> - `docs/design/2026-06-18-canonical-asset-identity-and-coverage-join-key.md`（canon_asset join）
> 不变量：AGENTS.md I7（阶段交付必有 evidence）、I8（「已检查为空」≠「未检查」）、I10（schema 先扩字段→上写→上读→清旧）、§2.5（gate 是确定性规则）、§2.7（改 schema/migration 先确认）

---

## 0. 一句话

把 **intel（信息收集）阶段** 做成「DB/账本是唯一真相源」的收尾：
1. **瘦身交付物**：AI 不再手填「6 情报 × 资产」覆盖矩阵；只交极少数显式例外（`blocked`/`not_applicable`+note）。矩阵由 harness 从 DB/账本确定性投影。
2. **每维精确新鲜度**：gate 判 `found` 只认 **本次 stage-run 之后采到的数据**，杜绝「上次遗留的旧行让这次直接判过」。

「交卷」动作（`submit_stage_deliverable`）与明确的 PASS/BLOCK 回执 **保留不变**。

---

## 1. 问题

### 1.1 提交繁琐 → 死循环（来自 06-15 设计，已实证）
弱模型在 target_intel 反复 `submit → BLOCK` 撞迭代上限：工具真跑了、证据真进账本，卡的永远是「把现成证据打包成合规 deliverable」这最后一下。

### 1.2 DB 真值无新鲜度（本文新增的根因）
`coverage_complete` 的 `found` 在 authoritative 模式只认 `ctx.evidence_facts`（`rule_engine.rs:593`）。而 hook 注入的 fact 有两条来源（`execute.rs:1480 fetch_evidence_facts_for_gate`）：
- **账本投影**（`evidence_facts_for_session(sid)`，`execute.rs:1490`）：**会话级**、带真 `evidence_id` → 天然 run-anchored + 有出处。
- **DB 业务表投影**（`db_truth_facts` → `db_truth_facts_to_evidence`，`execute.rs:2459`）：**org 级**、哨兵 `evidence_id=0`、**无时间窗** → 纯 presence。

二者在 `has_fact()` 里 **OR 合并**，所以「org 业务表里任何一条历史遗留行」就能让今天的格判 `found`，哪怕本次 run 没跑该技术。`freshness_check`（`gate/freshness_check.rs`）只作用于 `deliverable.evidence_refs`，**够不到这些派生 fact**——即 06-15 设计 §139 自称的「freshness 保留」对 DB-truth 路径并不成立。

---

## 2. 现状取证（行号为 2026-06-22 工作树，实现时复核）

### 2.1 简化的地基已就位
- `vacuous_check`（`gate/vacuous_check.rs:30`）：`facts_from_db_truth=true` + 有真 fact ⇒ 空 deliverable 也过 vacuous；completeness 仍由 `coverage_complete` 把关。
- `coverage_complete`（`rule_engine.rs:439`）：authoritative 下 `found_ok = has_fact(Found)`，自报 cell / tagged claim 一律不算（`:593-610`）。
- `coverage_corroborated`（`rule_engine.rs:730`）：**只遍历自报的 `status==Found` cell**。模型不再自报 found cell ⇒ 该规则自动 no-op；`for_all coverage where found require evidence` 同理。
- target_intel/spec.json 已开 `authoritative_found + derive_from_evidence + facts_from_db_truth`。

> 结论：简化主要是「别再要求 AI 填一张 gate 已忽略的表」+ 收口 prompt，**不放松关卡**。

### 2.2 时间戳列分两类（已核 migration）
| 维度 | 落库表/列 | 时间戳 | 类别 |
|---|---|---|---|
| DNS / RDNS | `dns_records`（行） | `created_at` | 类1 行级✓ |
| SUBDOMAIN | `target_assets`（行） | `discovered_at` | 类1 行级✓ |
| SERVICE-FP | `fingerprints`（行） | `detected_at` | 类1 行级✓ |
| DIR | `directory_entries`（行） | `created_at` | 类1 行级✓ |
| PARAM/JSAPI | `api_endpoints`（行） | `discovered_at` | 类1 行级✓ |
| ASN/CT/WHOIS/OSINT | `organizations` 的 asns/certificates/whois/intel **列** | 仅整行 `updated_at` | 类2 列级⚠ |
| PORT/LIVENESS/IPWHOIS | `targets` 的 ports/http_status/real_ip/ip_whois **列** | 仅整行 `updated_at` | 类2 列级⚠ |

类2 的坑：整行 `updated_at` 任何改动都会 bump（绑 org、改 scope），不代表该维真被重采。

### 2.3 新鲜度锚点已存在
`operation_state.stage_started_at` 与 `stage_runs.started_at`（`migrations/20260601000001_evidence_ledger.sql:69,86`）= 本次 stage-run 起点，可直接做「这次」的时间锚。

---

## 3. 设计

### 3.1 PR3 · 瘦身交付物（intel）
- **prompt 改写**（`task_orchestrator/prompts`）：intel 阶段指引删「逐格填覆盖矩阵」；改为「跑采集工具把数据落库即可；仅当某情报技术确实无数据源/被阻断时，交一个 `blocked`/`not_applicable`+note」。
- **gate 侧**：在 authoritative 模式显式跳过 `coverage_corroborated`（避免「已被 DB 行证明的 found 还要再要一个 tagged claim」的双重记账与误解）。其余规则因模型不再自报 found cell 自然 no-op，无需改。
- **不变**：`submit_stage_deliverable` 仍是显式「交卷」触发点；stage-close gate 仍回 PASS/BLOCK。

### 3.2 每维精确新鲜度（freshness）
核心：`coverage_truth_facts` / `fetch_evidence_facts_for_gate` 接受一个 `run_start: TIMESTAMPTZ` 参数，**只投影 `≥ run_start` 的数据**。

- **类1（行级，无 schema 改动）**：各 `build_*_values_sql` 加 `AND <row_ts> >= $run_start`（dns_records.created_at / target_assets.discovered_at / fingerprints.detected_at / directory_entries.created_at / api_endpoints.discovered_at）。
- **类2（列级，需加 per-维度时间列）**：
  - `organizations` 加 4 个 nullable 列：`asns_collected_at` / `certificates_collected_at` / `whois_collected_at` / `osint_collected_at`。
  - `targets` 加 3 个 nullable 列：`ports_scanned_at` / `liveness_checked_at` / `ip_whois_collected_at`（**EAS/enumeration 范围，本期 deferred**，先只做 intel 的 organizations 4 列）。
  - 读：`build_org_intel_presence_sql` 的每维判据从「列非空」改为「列非空 **AND** 对应 `*_collected_at >= $run_start`」。
- **写路径义务（关键风险点）**：所有写 asns/certificates/whois/intel 列的地方，必须 **同一次写** bump 对应 `*_collected_at = NOW()`。已知写点（需逐一审计）：
  - `golish-pentest/src/output_store/organizations/writers.rs`（merge_whois 等）
  - `golish-db/src/repo/organizations.rs`
  - `golish-recon-app/src/asset_intel/service/hydrate.rs`（update_profile）
  - `golish-recon-app/src/organization_recon/persistence.rs`（GUI 路径 land_ct_and_whois 等）
  - 漏 bump 任一写点 ⇒ 该维永远「旧」⇒ 假 BLOCK 死循环。**审计写点是本设计成败关键**。
- **anchor**：`run_start = stage_runs.started_at`（当前 stage-run）。语义＝重跑某阶段时不认上一次的数据；跨阶段共享的数据（如 intel 落的子域被 EAS 用）不受影响，因为各阶段读自己维度。

---

## 4. 迁移（I10 三步）
1. **扩字段**：`ALTER TABLE organizations ADD COLUMN IF NOT EXISTS <dim>_collected_at TIMESTAMPTZ`（nullable，NULL = 历史未知 = 不算「这次」）。additive、replayable。
2. **上写代码**：所有 org-intel 写点 bump 对应 collected_at。
3. **上读代码**：`coverage_truth` presence 改为带时间窗；hook 传 `run_start`。

NULL 语义：旧行 `*_collected_at IS NULL` ⇒ `NULL >= run_start` 为 false ⇒ 不投影 ⇒ 视为「这次没采」（保守、不放松 gate）。

## 5. 范围（用户 2026-06-22 决定：intel + EAS 一起做）
- **intel**：6 维（DNS/SUBDOMAIN 行级 + ASN/CT/WHOIS/OSINT 列级，organizations +4 列）。
- **EAS/enumeration**：targets +3 列（ports_scanned_at / liveness_checked_at / ip_whois_collected_at）+ 行级维度（fingerprints/directory_entries/api_endpoints）加时间窗。
- 用 spec 开关（`freshness_window: true`）灰度，关闭时逐字节回退现行为；按阶段分批打开（plan Phase A/B intel 先绿，再 Phase D EAS）。

## 6. 验证（TDD）
- **纯函数单测**（`coverage_truth.rs` assemble + SQL builder）：① 旧行（collected_at < run_start / NULL）不投影；② 新行（≥ run_start）投影；③ 行级维度时间窗正确。
- **gate 单测**：authoritative + 时间窗下，stale found 不再 PASS；fresh found PASS。
- **写点单测/集成**：每个 org-intel 写点写列后 collected_at 被 bump。
- **活体**：对一个真实 org 跑 intel；故意预置一条旧 org 数据，确认改动后它不再算「这次查到」；本次真采的算过。
- 退出门：`just precommit` 全绿 + 上述证据记入 `agent-progress.md`。

## 7. 风险
- **写点漏 bump**（最高）：任一 org-intel 写路径没 bump collected_at ⇒ 假 BLOCK。缓解：穷举写点 + 每点单测 + 灰度活体观察。
- **重跑语义**：anchor 用 stage-run start，重跑阶段会要求重采——需确认这是期望（用户已倾向「重跑不认上次」）。
- **schema 改动**（§2.7 高风险）：需用户批准 migration。
- **简化与全面性耦合**：模型不再自报后，缺投影的维只能 blocked+note；须确保 6 维投影齐全（已核 coverage_truth 覆盖 6 维）。

## 8. 待定 / sign-off
- ✅ 用户 2026-06-22 决定：**intel + EAS 一起做**；anchor = **stage-run start**；加 `freshness_window` 灰度开关；倾向「重跑阶段=重采」。
- ✅ 用户原则上批准 per-维度时间列方案（「我要每维精确时间戳」「一起搞」）。
- ⏳ **仍需最后过一眼**：migration（organizations +4 列、targets +3 列）实际落地前给用户复看一次 SQL（§2.7 高风险写操作的最终确认）。
