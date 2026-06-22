# 2026-06-15 · 数据库真值作为唯一真相源：交付物 = DB 投影 + agent 判断

> 日期：2026-06-15
> 状态：设计（2026-06-15 待审 §8）。**状态更新（2026-06-22 核实）**：🟡 **大部分已落地**——**PR1 落库闭环** ✅（`land_target_intel_coverage` 共享入口 + agent 路径 `agent_intel.rs:228`，plan `2026-06-15-pr1-recon-landing-closure.md`）；**PR2 evidence 列**（`evidence_technique`/`evidence_outcome` migrations `20260611000001`/`20260611000002`）+ per-technique facts ✅；**coverage `derive_from_evidence`** 投影已接（`resources/harness/stages/target_intel/spec.json` + `rule_engine.rs`）。后续「事实层单一真值源」方向由 `2026-06-18-canonical-asset-identity-and-coverage-join-key.md`（E1，PR-B/C/D 未做）+ `2026-06-18-slim-enrich-provider-rdap.md`（已落地）承接/调整。
> 作者：BaJie MCP-agent-4（无角色）
> 关联：
> - `docs/design/2026-06-14-target-intel-landing-and-tools.md`（落库管线根因，本文承接并落地其 §2③）
> - `docs/design/2026-06-14-deepseek-run-review-fixes.md`（deepseek 跑批复盘）
> - `docs/design/2026-06-12-unified-refiner.md`（统一 Refiner；本文解释其砍投影兜底「当时对、现在可解锁」）
> - `docs/design/2026-06-11-coverage-auto-derive-from-evidence.md`（§5.0 投影模型，本文是其逻辑终点）
> - `docs/design/2026-06-11-substantive-stage-evidence-projection-fallback.md`（被 06-12 废止的投影兜底）
> 不变量：AGENTS.md I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、§2.5（gate validator 是确定性规则）、§2.7（改 schema / 高风险先确认）

---

## 0. 一句话

让 **数据库成为一个阶段「收集到了什么」的唯一真相源**：事实部分（覆盖矩阵 / claims / evidence_refs）由 harness 从 DB 确定性投影，**agent 只负责「判断」部分**（findings 漏洞断言 + 极少数显式例外）。这样**任何模型只要真把活干了（数据落了库），阶段就能过 gate**，不再卡在「让弱模型把现成证据誊成一份结构化交付物」这一步。

**核心结论（与直觉相反）**：这套「查库判定」其实**已经做了一半**——`target_intel` 的 coverage gate 早已是 DB 真值驱动（`authoritative_found: true`）。卡住它的是两道**未闭合**的坎，下面逐一给证据。

---

## 1. 问题（跨模型复发，已实证）

弱模型在 `target_intel` 反复 `submit_stage_deliverable → BLOCK`，18~22 次/org，最终撞迭代上限：

- deepseek-v4-flash × pingan.com（`pentest-chat-1781447402541-1`）：原文 `intel coverage incomplete: never attempted (asset × SUBDOMAIN/CT/WHOIS)`。
- mimo-v2.5-pro × moresec.cn（06-12 unified-refiner §1）：模型自己数出真实 ledger ids、自述「只差提交」，真调了 submit 却交上 `claims=0/findings=0/evidence_refs=0` 的空壳 → vacuous BLOCK。

**共性**：工具真跑了、evidence 真进了账本（pingan 那轮到 483 条），**卡的永远是「把现成证据打包成一份合规 deliverable 并 submit」这最后一下**。这是「打包/运输」失败，不是「干活」失败——所以换强模型只是缓解，不治本。

---

## 2. 根因（已核源码，行号为 2026-06-15 工作树，实现时复核）

### 2.1 gate 已经是 DB 真值（half-built，关键认知）

`target_intel.json` 的 `coverage_complete` 规则已开 `derive_from_evidence: true` + `authoritative_found: true`，6 类 INTEL 技术全列入 `authoritative_techniques`（`resources/harness/stages/target_intel.json:45-50`）。注释白纸黑字：**"Tagging a claim or hand-writing a found cell no longer counts"**。

DB 真值由 `backend/crates/golish-db/src/repo/coverage_truth.rs` 提供（已通读全文）：

| 技术 | gate 读的业务表 / 列 | coverage_truth.rs |
|---|---|---|
| DNS | `dns_records`（join in-scope `targets`） | `:265-268` → `dns_records::present_target_values` |
| SUBDOMAIN | `target_assets(asset_type='subdomain')` | `build_subdomain_target_values_sql` `:106-110` |
| ASN | `organizations.asns` 非空 | `build_org_intel_presence_sql` `:85` |
| CT | `organizations.certificates` 非空 | `:86` |
| WHOIS | `organizations.whois` 非空 | `:76` |
| OSINT | `organizations.intel.records / contacts / social_accounts / business_systems` 任一非空 | `:77-89` |

运行期：`authoritative_found=true` 时 `found_ok = has_fact(Found)` only，自报 cell 与 tagged claim 一律不算（`harness/gate/rule_engine.rs:390-391, 420-427`）。DB 真值经 stage-close hook 注入 `GateContext.evidence_facts`（`task_orchestrator/subtask_phases/execute.rs:1249-1331, 1960-1964`）。

> 结论：**「从 DB 判 found」的机制已就位**。问题不在「要不要改成查库」，在于下面两道坎让这套机制空转。

### 2.2 坎 1 · 落库断了（主要矛盾）

侦察有**两条落库管线**，agent 用的那条恰好没接 coverage 落库钩子：

- **GUI org-recon 路径**：`organization_recon/runner.rs:323` → `persist_normalized_records`（`organization_recon/persistence.rs:70`）→ commit 后调用 **landing 钩子**：
  - `land_subdomain_assets`（`persistence.rs:235`）→ 把 org 子域 promote 成 `target_assets(asset_type='subdomain')`；
  - `land_dns_records`（`persistence.rs:302`）→ 解析 in-scope 域 → `dns_records`；
  - `land_ct_and_whois`（`persistence.rs:422`）→ crt.sh 写 `certificates`、RDAP 写 `whois`。
  - 即 06-14 落库设计 §2③ **已实现，但只在 GUI 路径**。
- **AI agent 路径**：`recon_enrich_assets` → `agent_intel.rs:84 run_passive_intel` → `hydrate.rs:119 run_providers_for_org` → **只 `update_profile`（`hydrate.rs:226`）**，写 `organizations` 几个列。

证据（grep 全工作区）：`persist_normalized_records` / `land_*` 的**唯一调用方是 `runner.rs:323`**；`asset_intel/` 目录内**零引用** `persist_normalized_records` / `land_*` / `target_assets` / `dns_records` / `coverage_truth`。即 **agent 路径完全绕过落库钩子**。

逐技术现状（直查库 + 源码，见 06-14 doc §0）：

| 技术 | 落 gate 表？ | 缺口 |
|---|---|---|
| ASN | ✅ | enrich 写 `organizations.asns` |
| OSINT | 基本 ✅ | enrich 写 contacts/social/business（未写 `intel.records`，但 gate 任一即可） |
| DNS | ❌ | dig 只进 evidence 账本；`dns_records=0` |
| SUBDOMAIN | ❌ | enrich 写 `organizations.domains`；subfinder 写平级 `targets`；`target_assets(subdomain)=0` |
| CT | ❌ | agent 路径无采集器；`certificates` 全空 |
| WHOIS | ❌ | enrich `ProfilePatch` 无 whois 字段（`golish-db .../organizations.rs`）；`whois` 全 NULL |

净结果：gate 读的表是空的 → coverage 永判 `never attempted` → 子代理永远 needs_fix → 死循环（无熔断）。

### 2.3 坎 2 · 残留「逼 agent 手填」+ evidence_id=0 死结

即使数据进了库，仍有**结构检查 + gate 规则**只读 AI 交的 deliverable、不读 DB：

- **vacuous_check**：`deliverable.evidence_refs.len() >= sum(spec.min_invocations)`（`harness/gate/vacuous_check.rs:39-51`）；`target_intel` 要 ≥2 顶层 refs（`min_invocations: {dns_resolve:1, subdomain_enum_passive:1}`，`target_intel.json:76-79`）。数 **AI 交的 refs**，不数库。
- **for_all 证据规则**：每条 claim 要 `evidence_ids`、每个 `found`/`checked_empty` 覆盖 cell 要 `evidence_refs`（`target_intel.json:18-43`），全读 deliverable 项（`rule_engine.rs:621-662`）。
- **coverage_corroborated**：每个**自报** `found` cell 必须有同 (subject, technique) 的 claim/finding 佐证（`rule_engine.rs:555-596`），**只读 deliverable，不看 evidence_facts**。
- **missing_deliverable = 永远 BLOCK**：解析不到 deliverable 直接 fail-closed（`execute.rs:1902-1921`），Refiner A 类只锁 tool_choice（`refiner.rs:94`），后端不再合成（PR-R2 已砍）。

**死结**：DB 真值投影出的 fact 用**哨兵 `evidence_id=0`**（`execute.rs:2136-2154`），**根本无法被 AI 引用**。于是 gate 一边说「按 DB 判 found」，一边 `coverage_corroborated` / `for_all` 又要 agent 给同一格手挂可引用证据——而那证据 agent 引用不了。弱模型在这个自相矛盾的契约里怎么试都过不去。

### 2.4 为什么 06-12 砍「投影兜底」是对的、现在能解锁

06-12 unified-refiner（§1）砍掉了「后端从账本投影合成 deliverable」，理由是它与 submit-only 锁互打。但真正让投影失效的是：**当时落库是坏的**——投影时账本只有「2 个 DB 哨兵（evidence_id=0）」，合成出空 deliverable → vacuous BLOCK（unified-refiner §1 attempt 1/2 逐字证据）。

> **即：投影模型不是错，是当年地基（坎 1 落库）没修好。坎 1 修好后，从 DB 投影就变干净、变可靠。** 这两道坎是连在一起的——这正是本设计把它们合到一份文档的原因。

---

## 3. 终态设计（最干净 / 最优雅）

### 3.1 两层分离：事实层（机器拥有）vs 判断层（agent 拥有）

一份阶段交付物拆成两半：

- **事实层（facts，机器拥有）**：覆盖矩阵 + 事实型 claims + evidence_refs。**永远由 harness 从 DB 真相确定性投影**（`coverage_truth` 业务表 + 账本 `(asset, technique, outcome)` facts）。agent 不再手写。
- **判断层（judgment，agent 拥有）**：findings（漏洞断言）+ 极少数显式例外（`blocked` / `not_applicable` + note）。**必须 agent 亲自提交**——findings 红线不破。

gate 校验的是「投影出的事实层 ⊕ agent 提交的判断层」合并体。因为事实层的 claims、coverage、evidence_refs **同源自一批真实 ledger facts**，`coverage_corroborated` / `for_all` / vacuous 这些「要 evidence」的检查**天然自洽通过**——不再需要弱模型手抄。

### 3.2 阶段分类（决定哪些阶段能投影事实层）

| 阶段 | 类型 | 事实层投影？ | 判断层（agent 必交） |
|---|---|---|---|
| scoping | confirm / facts | 子公司 SUBSIDIARY 可投影 | scope 确认 claim |
| **target_intel** | **facts-only** | ✅ 全 6 类 INTEL | findings 通常空 |
| external_attack_surface | facts-only | ✅ LIVENESS/PORT/SERVICE | findings 通常空 |
| enumeration | facts-only | ✅ DIR/PARAM/JSAPI | findings 通常空 |
| internal_discovery / objective_pathing / cleanup | facts-only | ✅（按需） | 通常空 |
| **vuln_triage** | **finding-producing** | coverage/分母可投影 | **findings 必交（不投影）** |
| **verification / access_validation / objective_simulation** | **finding-producing** | — | **findings 必交（fail-closed 不变）** |
| reporting | confirm-only | — | 报告确认 |

> **灰度策略**（沿用既有 opt-in 风格）：先只对 `target_intel` 开「事实层投影」，验稳再推 EAS/enumeration。漏洞类**永不投影 findings**。

### 3.3 gate 完成语义（facts-only 阶段）

阶段完成 = **DB 里每个 (in-scope 资产 × 期望技术) 都到了终态**：有数据(found) / 跑了为空(checked_empty) / 显式 blocked|not_applicable。缺证据的格 = `not_attempted` = 仍 BLOCK（投影补「真做了的事实」，补不出「没做的覆盖」）。`checked_empty` 只来自真实「跑了→空」的 outcome 行，**绝不**从「缺证据」推断（I8）。

---

## 4. 红线（任一违反即否决实现）

1. **findings 永不由后端产生 / 投影**（沿用 06-11/06-12 红线）。投影只产 claims + coverage + evidence_refs。
2. **I8 不破**：`checked_empty` 只来自账本真实 `empty` outcome 行；无行 = not_attempted = BLOCK。
3. **finding-producing 阶段 fail-closed 不变**：vuln_triage/verification 等仍要 agent 交 findings + 证据，缺则 BLOCK。
4. **I7 不破**：投影的 evidence_refs 全部指向账本真实行；`enforce_evidence_existence` 照跑。
5. **不削弱 gate 判错**：completeness / corroborated / denominator / freshness / fabricated 校验全保留；本设计改的是「事实从哪来」与「判错后怎么纠」，不是「怎么判错」。
6. **schema 改动走 §2.7 + I10**：若新增列（见 §8 D3），先扩 nullable → 再上写代码 → 再上读路径；先核 evidence hash 输入口径。

---

## 5. 落地三步（每步独立可回滚、TDD 全绿）

### PR1 · 落库闭环（地基，最高优先，不动 gate）

**目标**：让 agent 路径的侦察结果落进 gate 读的业务表 + 把 `(asset, technique, outcome)` 记成可引用的账本 facts。

1. **复用现成 landing 钩子**：把 `land_subdomain_assets` / `land_dns_records` / `land_ct_and_whois`（`organization_recon/persistence.rs:235/302/422`）抽成 crate 内共享函数，在 agent 路径 `run_passive_intel`（`agent_intel.rs:140` 之后、enrich `update_profile` 之后）调用。**不是重写，是接现成的**。非致命：landing 失败只 warn，不回滚 enrich（沿用 `persistence.rs:144-189` 的 additive 语义）。
2. **WHOIS / CT 补齐**：`land_ct_and_whois` 已含 crt.sh + RDAP；agent 路径接上即覆盖 CT/WHOIS。（可选：`resources/toolsconfig/whois.json` 注册本地 `/usr/bin/whois`，见 06-14 doc §2①。）
3. **per-technique 账本 facts**：`recon_enrich_assets` 落库点（`golish-agent-app/.../direct/mod.rs:454-457` 现在显式不产 facts）改为对每个真正执行的技术 book 一条 `EvidenceInput { technique, asset, outcome }`（账本列已存在：migration `20260611000001/2`，`audit/mod.rs:195-199`）。这一步解开 §2.3 的 evidence_id=0 死结——投影可引用**真实 ledger id**。

**验证**：端到端跑一次 pingan target_intel，直查 `dns_records>0 / target_assets(subdomain)>0 / organizations.certificates 非空 / .whois 非空`；gate 出 `target_intel` 接近 PASS（此时仍受坎 2 阻，PR2 解决）。

### PR2 · 事实层改 DB 投影 + 拆残留手填（facts-only 阶段）

1. **事实层投影**：facts-only 阶段，harness 从 `coverage_truth_facts` + 账本 facts 投影 claims/coverage/evidence_refs（引用 PR1 的真实 ids），合并 agent 提交的 findings/例外，再过 gate。投影当**主路径**（非「missing 时的兜底」），从机制上避开 06-12 的「投影 vs submit-only 锁互打」。
2. **拆残留**：facts-only 阶段，把 vacuous `min_invocations`、`for_all` 覆盖 evidence、`coverage_corroborated` 的判据从「读 deliverable」改为「读 DB 真值/投影」（或对投影出的自洽事实层免检）。**漏洞类阶段这些规则一律不动**。
3. **门控**：opt-in spec 字段（沿用 `derive_from_evidence` 的 `#[serde(default=false)]` 风格），先只 `target_intel`。

**验证**：弱模型 + target_intel 活体对照——修前 18+ 次 needs_fix → 修后凭「DB 落库 + 极简提交」过 gate；漏洞类阶段回归仍 fail-closed。

### PR3 · 提交通道 / Refiner 简化（收尾）

facts-only 阶段事实层不再靠 agent 打包后：

1. submit 契约对 facts-only 阶段退化为「确认 + findings + 例外」（多数为空）。
2. submit-only 锁（Refiner A 类）+ missing-deliverable 路径对 facts-only 阶段可简化：DB 真值覆盖完整即可 PASS，agent 的 submit 退为「挂 findings/例外」的检查点。finding-producing 阶段保持现状（fail-closed + A 类锁）。
3. C 类「vacuous/coverage 诊断」对 facts-only 阶段基本不再触发（事实层自洽）。

**验证**：facts-only 阶段无投影合成日志的同时 PASS；finding-producing 阶段行为逐字节不变。

---

## 6. 影响面

| crate / 文件 | 改动 | PR | 风险 |
|---|---|---|---|
| `golish-recon-app/src/organization_recon/persistence.rs` | landing 三钩子抽共享函数 | PR1 | 低（提取，不改逻辑） |
| `golish-recon-app/src/asset_intel/agent_intel.rs` / `service/hydrate.rs` | agent 路径调 landing | PR1 | 中（与在途 `http.rs` rework 协调，见 §10） |
| `golish-recon-app/src/asset_intel/profile_patch.rs` + `golish-db .../organizations.rs` | 补 WHOIS 字段路径（若用 enrich 落 whois） | PR1 | 低 |
| `golish-agent-app/src/ai/.../direct/mod.rs` | enrich 落库点 book per-technique facts | PR1 | 中（落库点多，逐处对） |
| `golish-agent-kit/src/harness/gate/rule_engine.rs`、`vacuous_check.rs` | facts-only 投影 + 残留判据改读 DB | PR2 | 中（核心 gate，TDD 全覆盖） |
| `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 事实层投影主路径 + 注入 | PR2 | 中 |
| `golish-agent-kit/src/harness/stage_spec.rs` + `resources/harness/stages/target_intel.json` | opt-in 投影门控字段 | PR2 | 低 |
| `golish-agent-kit/src/task_orchestrator/refiner.rs` + `execute.rs` | facts-only submit/锁简化 | PR3 | 中 |

**ts-rs/IPC**：`StageSpec` / `StageDeliverable` / `EvidenceFact` 为 harness 内部类型，不跨 IPC（06-11 设计已核 0 命中）；实现时复核。**DB schema**：PR1 复用既有列/表，**不改 schema**；仅当选 §8 D3「正经列」方案才走 migration。

---

## 7. 风险与缓解

- **投影掩盖「没测全」**：投影只产有真证据的 found/empty 格；completeness gate 不动 → 缺证据仍 BLOCK。投影救「打包」，救不了「漏测」。
- **某情报阶段其实会产 finding**：opt-in 逐阶段评估；漏洞类永不开；findings 红线兜底（投影永不产 finding）。
- **与在途 `http.rs` rework 冲突**（812 行未提交，见 §10）：PR1 先与该工作对齐落库点，避免双改同区。
- **landing 钩子在 agent 高频路径的性能**：landing 非致命 + 幂等 upsert；必要时限频/去重（按 org+run）。
- **灰度回滚**：opt-in 门控默认 off → 一行回退；PR1 landing 失败只 warn，不影响主流程。

---

## 8. 待决策（请用户拍板）

- **D1 · 落库点（PR1）**：① 复用 `persist_normalized_records` 整条（最省，但 agent 路径要构造 `NormalizedReconRecord`）；② 抽 landing 三钩子为共享函数、agent 路径单独调（更解耦，推荐）。倾向 ②。
- **D2 · per-technique facts 来源**：PR1 让 enrich/CLI 落库时权威 book `(technique, asset, outcome)` ledger fact（推荐，解 evidence_id=0 死结）vs 继续用 DB 真值哨兵 0 + 改投影绕过引用校验。倾向前者（更干净、可引用）。
- **D3 · WHOIS 落点**：`organizations.whois` 走 `land_ct_and_whois` 的 RDAP（已现成，推荐）vs 给 `ProfilePatch` 加 whois 字段走 enrich provider。倾向 RDAP 现成。
- **D4 · 投影范围灰度**：先只 `target_intel`（推荐）vs 一次性给所有 facts-only 阶段开。
- **D5 · facts-only 是否还要求 agent 调 submit**：保留「submit 作为挂 findings/例外的检查点」（更稳，推荐）vs DB 覆盖完整即自动 PASS、submit 全可选（最激进最优雅，但动 fail-closed）。倾向先保留检查点。

---

## 9. 验证计划（实现阶段，TDD + 活体）

- **PR1 单测**：landing 共享函数纯函数部分（subdomain pair 配对、dns 解析落库幂等）；enrich book facts 的 `(technique, asset, outcome)` 正确。
- **PR1 活体**：pingan/moresec 跑 target_intel，直查 4 张表非空 + 账本 facts 三元组齐。
- **PR2 单测**：投影 deliverable 的 claims/coverage/evidence_refs 来自真实 facts；findings 必空；空账本不投影（回退 BLOCK）；漏洞类阶段门控 off 逐字节不变。
- **PR2 gate 集成**：投影体过 coverage_complete(authoritative) + corroborated；缺证据格仍 BLOCK。
- **PR2 活体对照**：弱模型 target_intel 修前 pause → 修后过 gate；证据落 `agent-progress.md`。
- **收口**：`just precommit` 全绿；`code-audit` 复核 findings 红线、I7/I8 未被触碰。

---

## 10. 与在途工作协调

工作树有未提交改动（`git diff --stat`）：`asset_intel/runtime/http.rs`（+812/-...）、`asset_intel/tests.rs`（+270）、`pentest-domain/models/asset_intel.rs`（+52）、`profile_patch.rs`（+1）。经查这是 **HTTP provider 运行时 / artifact 持久化（`persist_http_artifacts`）重构**，**不涉及** coverage 落库（`target_assets`/`dns_records`/`coverage_truth`）。PR1 动的是 `agent_intel.rs`/`hydrate.rs` 的落库接线，与该重构相邻但不同区——**实现前先确认该改动是否已提交/方向**，避免双改 `asset_intel/` 冲突。

---

## 11. 给审阅者的一句话

这不是「推翻 evidence 逻辑」，而是把**已经做了一半的 DB 真值机制闭合**：先让数据真落库（PR1，接现成钩子），再让事实层从库里投影、把弱模型从「手抄交付物」里解放出来（PR2），最后顺手简化提交通道（PR3）。门槛始终是「得有真证据」（I7/I8 不破）；变的只是「事实由系统从库里确定性地抄，agent 只下判断」。
