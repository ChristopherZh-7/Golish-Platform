# Plan · 2026-06-22 · intel + EAS 每维精确新鲜度 + 瘦身交付物

> 设计：`docs/design/2026-06-22-intel-perdim-freshness-slim-deliverable.md`
> 范围（用户定）：intel + EAS 一起；anchor=`stage_runs.started_at`；`freshness_window` 灰度开关；schema 改动落地前用户复看 SQL。
> 不变量：I7/I8/I10/§2.5/§2.7。每阶段结束跑对应 scoped 验证并记证据；全做完跑 `just precommit`。

---

## Phase 0 · 审计（只读，先把写点摸全 — 本设计成败关键）
- [x] 列出所有写 `organizations` 情报列的代码点 → 见下「Phase 0 审计结果」3 个写面 A/B/C。
- [x] 列出所有写 `targets` 的 ports / http_status / real_ip / ip_whois 的代码点 → 见下表。
- [x] 确认 stage-close hook 能取到 run_start → `operation_state.stage_started_at` 经 `operation_state::get` 可取（详见下）。
- **验证**：产出一份「写点清单」贴进本 plan；无清单不进 Phase A/D 的写点改造。

### Phase 0 审计结果（2026-06-22 · BajieAsk-agent-2 接手 MCP-3 执行 · 行号=当前工作树）

#### run_start 锚点 ✅ 可取（Phase 0 第 3 项）
`operation_state.stage_started_at`（`golish-db/src/repo/operation_state.rs:20`；`advance_stage` 每次切阶段 `stage_started_at = NOW()`，`:97`）= 本次 stage-run 起点。hook `fetch_evidence_facts_for_gate`（`execute.rs:1480`）已可经 `crate::db_shim::operation_state::get(&*self.repo, task_id)`（同文件 `:632`/`:661` 已在用）取到 → 直接做 `run_start`，**无需新管线**。等价于设计里的 `stage_runs.started_at`。

#### organizations 情报列写点 — 3 个写面（Phase A，关键）
> 写面归类：A=AI 工具/provider 路径（`store_organization_update`，`intel_providers.rs:192` + `output_store/mod.rs:211` db_action 均经此）；C=`repo::organizations::update_profile`（被 `asset_intel/service/hydrate.rs` enrich + GUI + 子公司提升共用——故 bump 只能放 enrich 采集点，不能放进通用 `update_profile`）；B=GUI org-recon `ProfileAccumulator`。

**写面 A · AI/pentest 工具路径**（`golish-pentest/src/output_store/organizations/writers.rs`，逐列 append 幂等）
| # | 函数:行 | 列 | 维度 → 应 bump |
|---|---|---|---|
| A1 | `append_string_array` `:60`（`column=="asns"`，SQL `:81`） | asns | ASN → `asns_collected_at` |
| A2 | `append_object_with_value` `:123`（`column=="certificates"`，SQL `:136`） | certificates | CT → `certificates_collected_at` |
| A3 | `merge_whois` `:249`（SQL `:254`） | whois | WHOIS → `whois_collected_at` |
| A4 | `append_contact` `:157` | contacts | OSINT → `osint_collected_at` |
| A5 | `append_social_account` `:183` | social_accounts | OSINT → `osint_collected_at` |
| A6 | `append_business_app` `:210` | business_systems | OSINT → `osint_collected_at` |
| A7 | `append_intel_record` `:270` | intel.records | OSINT → `osint_collected_at` |
> 注：`writers.rs` 同列的 `append_string_array` 白名单还含 ip_ranges/email_domains/github_orgs/subsidiaries/cloud_assets（非 4 维，不 bump）；`update_scalar_if_empty`(industry/credit_code)、`append_alias`、`append_domain` 同样不涉 4 维。dispatcher `mod.rs:217` 仅 `SET updated_at=NOW()`（非情报列，不计）。

**写面 B · GUI recon 路径**（`golish-recon-app/src/organization_recon/persistence.rs`）
| # | 位置 | 列 | 维度 → 应 bump |
|---|---|---|---|
| B1 | `ProfileAccumulator::write` `:904`（单条 UPDATE 一次写 9 列） | certificates / intel / business_systems / social_accounts / contacts（+domains/ip_ranges/email_domains/historical_vulns 非 4 维） | CT + OSINT → 同写 `certificates_collected_at` **且** `osint_collected_at` |
| B2 | land_whois `:564`（`UPDATE organizations SET whois=$1`） | whois | WHOIS → `whois_collected_at` |

**写面 C · 通用 profile patch 路径**（`golish-db/src/repo/organizations.rs::update_profile`，`patch_field!` 宏 `:322`，每列一条 UPDATE）
| # | 位置 | 涉 4 维列 | 维度 → 应 bump |
|---|---|---|---|
| C1 | `patch_field!(asns,…)` `:344` | asns | ASN → `asns_collected_at` |
| C2 | `patch_field!(certificates,…)` `:349` | certificates | CT → `certificates_collected_at` |
| C3 | `patch_field!(intel,…)` `:347` / `(business_systems)` `:351` / `(social_accounts)` `:354` / `(contacts)` `:356` | intel/business_systems/social_accounts/contacts | OSINT → `osint_collected_at` |
> 写面 C **不**写 whois（`patch_field!` 无 whois 项）→ WHOIS 仅 A3+B2 两个写点。`patch_field!` 是 per-field 宏，bump 需在宏内按列映射到对应 collected_at（C 最易漏，需单测每分支）。

#### targets EAS 列写点（Phase D）
| 维度 | 列 | 写点 |
|---|---|---|
| PORT | ports | `golish-db/repo/targets.rs::update_ports_by_id :449`、`update_recon_extended_by_id :474`、`golish-pentest/output_store/targets.rs :67`、`golish-recon-app/targets/cmds.rs :228`（GUI）；landing.rs `:157` / intel_providers.rs `:320` / persistence.rs `:761`（Phase D 落地前再逐一核列） |
| LIVENESS | http_status / real_ip | `update_recon_extended_by_id :474`、`output_store/targets.rs :67`、`set_real_ip_by_id :542`、`backfill_real_ip_from_dns :528` |
| IPWHOIS | ip_whois | `golish-db/repo/targets.rs::set_ip_whois_by_id`（SQL builder `build_set_ip_whois_sql :562`） |

**Phase 0 结论**：3 个 org 写面（A 逐列 / B 多列单写 / C patch 宏）全部需在同一次写 bump 对应 `*_collected_at`；最高漏 bump 风险在写面 C（patch 宏）与写面 B1（多列单写要同时盖 CT+OSINT 两个时间戳）。run_start 锚点已就位。→ Phase A/D 写点改造可启动。

## Phase A · intel 列级新鲜度（organizations 4 维）
- [x] A1 migration `20260622000001_organizations_intel_collected_at.sql`（asns/certificates/whois/osint，nullable TIMESTAMPTZ，`ADD COLUMN IF NOT EXISTS`，无 default）。✅ 用户 2026-06-22 sign-off 后已写入。
- [x] A2 写点 bump（4 站点，**dedup-safe**，`cargo check -p golish-db -p golish-pentest -p golish-recon-app` 绿）：
      · 新增 golish-db `IntelDim` + `stamp_intel_collected_at`（`repo/organizations.rs`）= 列名单一真值源。
      · 写面 A `output_store/organizations/mod.rs` dispatcher 末尾按 present fields bump asns/cert/whois/osint（替换原 updated_at-only UPDATE）。
      · 写面 C `asset_intel/service/hydrate.rs` enrich 后按 patch 维度调 `stamp_intel_collected_at`（**不**放进通用 `update_profile`，避开 clear/promote/GUI 误 bump）。
      · 写面 B `organization_recon/persistence.rs` ProfileAccumulator 加 `touched_ct/touched_osint`（merge_record 按 record kind 置位）→ `write` 动态 SET。
      · 写面 B2 land_whois 加 `whois_collected_at = NOW()`。
      · 关键修正：dedup-gated 的逐值 append（NOT EXISTS）不能承载 bump（重采已知值会漏 bump→假 BLOCK）→ 全部改在「采集站点按维度」bump。
- [ ] A3 读：`coverage_truth.rs` `build_org_intel_presence_sql` 每维「列非空」→「列非空 AND `<dim>_collected_at >= $run_start`」；`coverage_truth_facts` 签名加 `run_start: Option<DateTime>`；hook 传入。`freshness_window` 关 → 旧 SQL（逐字节回退）。
- [ ] A4 TDD：① presence SQL 含时间窗谓词；② assemble：旧/NULL collected_at 不投影、新的投影；③ gate 单测：stale org 数据不再 PASS、fresh PASS。
- **验证**：`cargo nextest -p golish-db -p golish-agent-kit`；记 pass 数。

## Phase B · intel 行级新鲜度（DNS / SUBDOMAIN）
- [ ] B1 `build_subdomain_target_values_sql` + dns_records 存在查询加 `discovered_at/created_at >= $run_start`（受 `freshness_window` 控）。
- [ ] B2 TDD：行级时间窗单测（旧行不计、新行计）。
- **验证**：`cargo nextest -p golish-db`。

## Phase C · 瘦身交付物（PR3）
- [ ] C1 改 intel prompt（`task_orchestrator/prompts`）：删「逐格填覆盖矩阵」；改「跑采集工具落库即可；仅对确无数据源/被阻断的技术交 `blocked`/`not_applicable`+note」。
- [ ] C2 `coverage_corroborated`：authoritative 模式显式 no-op（gate 侧加开关或在 eval_one 判定），+单测「authoritative 下不要求 tagged claim」。
- [ ] C3 verify：弱模型空 deliverable + 真落库 → PASS；空库 → BLOCK（沿用 vacuous_check 现有保证 + 新增 e2e）。
- **验证**：`cargo nextest -p golish-agent-kit`；scoped。

## Phase D · EAS / enumeration 扩展
- [ ] D1 migration `add targets ports_scanned_at / liveness_checked_at / ip_whois_collected_at`（nullable）。**落地前用户复看 SQL**。
- [ ] D2 写点 bump：ports / http_status·real_ip / ip_whois 的写点（Phase 0 清单）。
- [ ] D3 读：`build_port_values_sql` / `build_liveness_values_sql` / `build_ipwhois_values_sql` 加列时间窗；行级 `fingerprints.detected_at` / `directory_entries.created_at` / `api_endpoints.discovered_at` 加 `>= $run_start`。
- [ ] D4 TDD：EAS 各维时间窗单测 + gate 单测。
- **验证**：`cargo nextest -p golish-db -p golish-agent-kit -p golish-recon-app`。

## Phase E · 收尾
- [ ] `just precommit` 全绿。
- [ ] 活体：真实 org 跑 intel（+EAS）一遍；预置一条旧数据，确认改动后不算「这次」；本次真采的算过；AI 提交体量明显变小、不再卡填表。
- [ ] 更新 `agent-progress.md`（证据）+ `feature_list.json`（新增条目 status）+ 本 plan / 设计文档状态。

---

## 风险与回滚
- **写点漏 bump**（最高）：穷举 + 每点测 + 灰度活体；`freshness_window=false` 一键回退到「presence-only」现行为。
- **migration**：nullable additive（I10），可单独回滚 DROP COLUMN（无代码引用时）；落地前用户复看。
- **重跑语义**：anchor=stage-run start ⇒ 重跑要求重采（用户已确认倾向）。
