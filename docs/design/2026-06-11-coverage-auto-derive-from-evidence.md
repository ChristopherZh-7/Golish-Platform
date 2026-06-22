# Coverage 矩阵：从证据账本自动派生（弱模型交付物简化）

> 日期：2026-06-11
> 状态：设计（2026-06-11 待审 §7）。**状态更新（2026-06-22 核实）**：✅ **已落地**——`derive_from_evidence` 投影已接（`resources/harness/stages/target_intel/spec.json` + `golish-agent-kit/src/harness/gate/rule_engine.rs` + `stage_spec.rs` + `evidence_facts.rs`）；`(asset, technique, outcome)` 三列 migration `20260611000001_evidence_technique_outcome.sql` + `20260611000002_evidence_asset.sql` 已落、`golish-db/src/repo/audit/mod.rs` 已读写。
> 关联：`docs/design/2026-06-05-coverage-matrix.md`、`docs/design/2026-06-05-vuln-triage-technique-matrix.md`、`docs/design/2026-06-11-weak-model-submit-channel.md`、`docs/design/2026-06-02-submit-stage-deliverable-tool.md`
> 不变量：AGENTS.md I7（阶段交付必须有 evidence）、I8（「已检查为空」≠「未检查」）、§2.5（安全语义变更）

---

## 1. 问题（live run 实证，2026-06-11 mimo-v2.5-pro × moresec.cn）

弱模型在 `target_intel` 反复 BLOCK 直至 pause。剖出两个独立卡点：

1. **结构化提交没被 stage-close gate 吃到（运输丢失）**。模型经 `submit_stage_deliverable` 工具交了一份 2.8KB 的 StageDeliverable（6 claims + 6 coverage cells，技术标注齐全），可 `apply_harness_gate_hook` 只 `parse_deliverable_from_content(content)`（`task_orchestrator/subtask_phases/execute.rs:1771` / `:2018`）——**只认 agent 最终文本里的 ```json 代码块**，不直接读工具侧信道 `harness_last_deliverable`。该侧信道仅在 `bridge_executor/trait_impl.rs:93` 的「orchestrator 文本无 deliverable 签名时 append 一份」分支里间接回流，弱模型这条路没走通 → stage-close 见 `content_len=0` → `missing_deliverable` BLOCK（fail-closed）。

2. **coverage 矩阵的手写负担**。`coverage_complete`（`harness/gate/rule_engine.rs:279`）要求「每个 in-scope 资产 × 每类期望技术」都有终态 cell。被动情报 6 类技术（`GOLISH-INTEL-DNS/WHOIS/ASN/CT/SUBDOMAIN/OSINT`），资产含 root + 发现的 8 子域时矩阵最多 9×6=54 格，每格要手写 `status / evidence_refs / units`，且每条 claim 还要 `technique` 打标 + `subject` 对齐。弱模型 over-think + 复读退化（live run 6× `repetitive output detected`），产不出这坨大 JSON。

工具其实都跑了、evidence 都进了账本（live run evidence ids 1782–1829，共 18 条）。**活干完了，卡在「把现成证据誊成一份大交付物」这一步。**

---

## 2. 目标

把弱模型在 recon 阶段要手写的东西，从「claims + 完整 coverage 矩阵 + units」砍到**「确认 + 引用真实 evidence id」**；矩阵由 harness **确定性地从证据账本派生**。在**不削弱任何 gate 校验、不破 I7/I8** 的前提下完成。

非目标（明确排除）：
- 不动 active 阶段（EAS / enumeration / vuln_triage）的覆盖语义——本设计只先落 `target_intel`（被动情报）单阶段灰度。
- 不自动判定 `checked_empty`（见 §4 约束 2，这是 I8 红线）。
- 不改 evidence hash-chain / schema（只读账本 + 可选加一个可空 technique 标注列，见 §6 决策）。

---

## 3. 现状勘查（动手前先读，已核对）

| 机制 | 位置 | 现状 |
|---|---|---|
| 交付物解析 | `execute.rs:1771 parse_deliverable_from_content` | 只解析 agent 文本里的整体 JSON / ```json fence；**不读工具侧信道** |
| 工具侧信道 | `harness_submit_tool.rs:56 last_deliverable` + `agent_bridge` `harness_last_deliverable` | 工具把结构化交付物写进侧信道；`bridge_executor/trait_impl.rs:93` 仅在「文本缺签名」时 append 回 content |
| coverage_complete | `rule_engine.rs:279` | 资产维度取 `ctx.in_scope_assets`（①注入）否则自报；技术维度取 `ctx.expected_techniques`（③）否则 spec；逐格核终态 |
| derive_from_items | `rule_engine.rs:346-352` | 已支持：`Found` 格可由「`subject==asset && technique==tech` 的 claim/finding」派生（**从 claim 派生，尚未从 evidence 派生**） |
| coverage_corroborated | `rule_engine.rs:464` | 每个 `Found` cell 必须有 technique+subject 对齐的 claim/finding 佐证 |
| 技术词典 | `resources/harness/technique_taxonomy.json` | `GOLISH-INTEL-*` 等已登记；写错 id 由单测 fail-closed |
| evidence 落库 | `direct/mod.rs:431 evidence_append(... tool, kind, subject, raw)` | `kind` = 工具名（粗粒度：实测 `recon_enrich_assets` / `background_command`），**无 technique 字段** |

**关键现状缺口**：evidence 行的 `kind` 是工具名（粗），单凭 `kind` 无法可靠映射到 `GOLISH-INTEL-*` 技术类（一条 `background_command` 可能是 dig DNS、也可能是别的）。所以「从证据派生 coverage」要先解决**「evidence → 技术类」的可靠来源**。

---

## 4. 完整性约束（本设计的核心，对应用户「能保证数据完整性吗」）

派生必须满足以下 4 条，**任一不满足则该格不派生**（fail-closed，留 `not_attempted` → `coverage_complete` 照常 BLOCK）：

1. **只从真 evidence 行派生（保 I7）**：派生的 `Found` cell 的 `evidence_refs` 必须指向账本里真实存在的 `audit_role='evidence'` 行；现有 fabricated-ref 存在性校验（`enforce_evidence_existence`）照跑。派生比模型自报更强——来源是确定性账本，不是自然语言。
2. **绝不自动派生 `checked_empty`（保 I8，红线）**：账本无某 (资产×技术) 的证据 = 「没测」**或**「测了为空」二者无法区分；自动判 `checked_empty` 即把两者混为一谈，直接破 I8。故 `checked_empty` 永远只能显式产生（要求工具/模型给出「测了但空」的正向信号，如一次返回空结果集的探测 evidence）。派生**只产 `Found`**。
3. **映射确定且保守**：`evidence → (asset, technique)` 必须精确——`subject` 完全等于某 in-scope 资产串，且该 evidence 携带**无歧义**的技术归属（见 §5/§6 来源决策）。任一歧义 → 不派生该格。
4. **completeness / corroborated / denominator 三 gate 不动**：派生只是把 cell 加进「模型自报 ∪ 证据派生」的并集，随后跑**同一套**完整性/佐证/分母校验。所以派生**只能补「有真证据的格」，补不出完整性**——缺证据的 (资产×技术) 仍判缺口 BLOCK。

> 一句话保证：A 是「把模型本来要照着现成证据手抄的 `Found` 格，改成 harness 确定性地抄」。证据是同一批真 id，gate 是同一套校验。新增的唯一信任点 = 「evidence→技术」映射，由 §4.3 保守规则 + `coverage_corroborated` 双重兜底。**守住「不自动造 checked_empty」，I7/I8 都不破。**

---

## 5. 设计

> **目标态选定（用户：要最干净最优雅，token/工作量无所谓）= §5.0 投影模型。** §5.1/§5.2/§5.3 是通往它的可落地分层（C 先行、A1 落库 technique+outcome），不是终点。

### 5.0 目标态（最干净·推荐）：coverage = 证据账本的确定性投影

把 coverage 矩阵从「模型手写」彻底改为「harness 对证据账本的纯只读投影」。**单一事实源 = 证据账本**：

- 每次「在资产 A 上执行技术 T」都落一条 evidence 行，带 `(asset=A, technique=T, outcome ∈ {found, empty})`——**跑了但无结果也落一条 `empty` 行**。
- coverage 矩阵 = 对这些行的确定性投影：有 `found` 行 → `Found`；只有 `empty` 行 → `CheckedEmpty`；**无任何行 → `not_attempted`（缺口，BLOCK）**。blocked/not_applicable 仍由模型显式声明（带理由）。
- 模型**不再手写矩阵**；只提供无法从数据派生的判断：findings（漏洞）、极少数 `blocked`/`not_applicable` 例外。
- gate 在投影出的矩阵上跑**同一套** completeness/corroborated/denominator。

**这一步顺带最干净地解决 I8**：`checked_empty` 不再靠「缺证据」推断（那才会破 I8），而是靠一条**真实的「跑了→空」evidence 行**——「测了为空」是被记录的事实，与「没记录=没测」天然区分。于是 §4 约束 2 的红线变成「在证据层用 outcome 显式区分」，而非「禁止派生」。I7/I8 双双落到证据层，矩阵退化为只读视图。

**契约简化（自然结果）**：`submit_stage_deliverable` 对 recon 阶段退化为「确认 scope + 提交 findings/例外」；claims/coverage 不再要模型誊写（claims 可由投影补全或省略）。这消灭了 §1 的两个卡点的**根**：没有大矩阵要手写（卡点 2），提交体量小到弱模型也能稳定产出（缓解卡点 1）。

**代价（已获用户授权换干净）**：recon/active 工具必须一致地落 `(asset, technique, outcome)` evidence（含空结果）——把「真相」收敛到单一源的一次性投入；落点散在各 recon/扫描工具，需逐处对齐。

### 5.1 / 5.2 / 5.3 — 通往 §5.0 的分层落地

### 5.1 C — stage-close 直接认结构化提交（运输鲁棒性）

`apply_harness_gate_hook` 在 `parse_deliverable_from_content(content)` 失败时，**先读工具侧信道** `harness_last_deliverable`（已是序列化好的 StageDeliverable JSON），命中即用，而不是立刻判 `missing_deliverable`。

- 收益：去掉「调了工具还要再把整坨 JSON 复述进最终文本」这个弱模型最易翻车点。
- 完整性：零影响——同一份 StageDeliverable 过同一个 gate（`validate_stage_gate` 全套）。只是少了「提交在运输中丢失 → 假 BLOCK」。
- 边界：侧信道是 `submit_stage_deliverable` 工具写的（已过工具内联的 schema/fabricated 预检），权威性 ≥ 文本解析。仍以 stage-close gate 为终判。

### 5.2 A — coverage_complete 增 `derive_from_evidence` 模式（矩阵自动派生）

新增 gate rule 选项（沿用 `derive_from_items` 的加性、`#[serde(default false)]` 逐字节兼容风格）：

```
{ "op": "coverage_complete", "derive_from_items": true, "derive_from_evidence": true, "on_fail": {...} }
```

`derive_from_evidence:true` 时，对每个 (in-scope 资产 × 期望技术)，在「模型自报 cell」「claim/finding 派生」之外**再加一条派生来源**：账本里存在 `subject==asset` 且技术归属==该技术 的 evidence 行 → 该格记 `Found`（evidence_refs 填这些真实 id）。仍受 §4 四条约束。

派生在 gate 层做（纯函数仍 DB-free）：所需的「evidence (subject, technique, id) 三元组」由外层 hook 经 `GateContext` 注入（照抄 `in_scope_assets` / `expected_techniques` 的注入模式，新增 `GateContext.evidence_facts: Option<Vec<EvidenceFact{ subject, technique, evidence_id }>>`）。无注入 → `None` → 行为与今天逐字节一致（回退）。

### 5.3 「evidence → 技术」来源（§4.3 的关键，二选一，见 §7 决策）

- **方案 A1（推荐·治本）**：recon 工具**落库时带 technique 标注**。dns_resolve 知道自己是 DNS、subfinder 知道 SUBDOMAIN、recon_enrich 各子项知道 OSINT/ASN/CT…——在 `evidence_append` 增一个可空 `technique: Option<&str>`（GOLISH-INTEL-*），由工具侧权威写入。派生直接读这个标注，零歧义。需在 evidence 行存该标注（§6 决策：放 `audit_log.detail->>'technique'` JSON 字段，不改 schema）。
- **方案 A2（过渡·启发式）**：在外层 hook 用 `(tool, kind, raw 片段)` 做保守映射表（如 `whois→WHOIS`、`ct_log→CT`、`dns_*→DNS`、`subfinder→SUBDOMAIN`）。命中即派生，未命中/歧义不派生。不改落库，但映射脆弱、覆盖不全（粗 `kind=recon_enrich_assets/background_command` 时无法判技术）。

> A1 更干净也更可信（工具是技术归属的权威来源），且让 `coverage_corroborated` 与派生同源；A2 仅作不改落库时的兜底。倾向 A1。

---

## 6. 影响面（A1 路线）

| crate / 文件 | 改动 | 风险 |
|---|---|---|
| `golish-pentest` evidence_ledger `append` + `EvidenceInput` | 增可空 `technique`（写入 `detail.technique`） | 低（加性、可空、不改 schema/hash 输入口径需确认——见决策 D3） |
| `golish-agent-app` recon 工具落库点（`direct/mod.rs`、`bridge_config.rs` 背景 job） | 落库时传 technique（工具已知） | 中（多处落库点，逐处对） |
| `golish-db/repo/audit` | 新增「按 session 取 (subject, technique, id)」只读查询 | 低（只读 `SELECT`，无 migration） |
| `golish-agent-kit` `GateContext` + `coverage_complete` | 增 `evidence_facts` 注入 + `derive_from_evidence` 分支（纯函数） | 中（核心 gate，TDD 全覆盖） |
| `golish-agent-kit` `execute.rs` hook | C：读侧信道；A：注入 evidence_facts | 中 |
| `resources/harness/stages/target_intel.json` | gate rule 开 `derive_from_evidence:true`（先单阶段灰度） | 低 |

---

## 7. 决策（用户已选「最干净」→ 目标态 §5.0 投影模型）

已定（按用户「要最优雅、token 无所谓」）：
- **目标态 = §5.0**：coverage 是证据账本的只读投影；evidence 行带 `(asset, technique, outcome∈{found,empty})`，跑了为空也落 `empty` 行；矩阵不再由模型手写。I7/I8 收敛到证据层。
- **evidence→技术 = A1**：工具落库时权威写入 `(technique, outcome)`（不走 A2 启发式）。
- **C 先行**：stage-close 认工具侧信道为权威来源（运输鲁棒性，零完整性风险），与投影解耦先落。

用户已拍（2026-06-11）：
- **D-store = 法1（正经数据库列 + migration）**。在 `audit_log` 加两列 `evidence_technique TEXT NULL` + `evidence_outcome TEXT NULL`（仅对 `audit_role='evidence'` 行有意义；可空 → 旧行/非证据行为 NULL）。按 §2.7（用户已授权改 schema）+ I10「先扩字段（nullable，向后兼容）→ 再上写入新列的代码 → 再上读新列的投影」。可建索引，投影查询走列而非 JSON 抽取。**hash 链处理**：实现首步先核 evidence hash 输入口径（`golish-pentest/evidence_ledger`）——新列若不在 hash 输入则零影响；若在，则新列仅对新行生效、旧行哈希不变（加列不改既有行内容）。
- **D-scope = 法1（先 `target_intel` 单阶段灰度）**，验稳投影模型再推 EAS/enumeration/vuln_triage。
- **D-claims**：投影补全（模型只交 findings/例外）。

排期（先干净、后增量，每 PR 独立可回滚、TDD 全绿）：
- **PR1 = C（侧信道权威）**：stage-close 认 `harness_last_deliverable` 工具提交。不依赖 migration，最小最安全，先落。
- **PR2 = evidence schema 扩列 + 写入**：migration 加 `evidence_technique/evidence_outcome`（nullable）；recon 工具落库时权威写入 `(technique, outcome)`（含「跑了→空」的 `empty` 行）；新增按 session 取 `(asset, technique, outcome, id)` 只读查询。先扩字段+写入，不改读路径（零行为变更）。
- **PR3 = coverage 投影 + 接线 + 活体**：`coverage_complete` 增 `derive_from_evidence`（经 `GateContext.evidence_facts` 注入）；`target_intel.json` 开开关；弱模型活体对照（修前 pause → 修后凭「确认+findings」过 gate）。

---

## 8. 风险与缓解

- **映射写错把某格误标 Found**：A1 由工具权威标注（不是猜）；再加 `coverage_corroborated`（Found 必须有 technique+subject 对齐的 claim/finding）兜底；保守规则歧义即弃。
- **派生掩盖「其实没测全」**：派生只产 Found 且只在有真证据时；completeness gate 不动 → 缺证据的格仍 BLOCK；`checked_empty` 永不自动造（I8）。
- **弱模型仍可能连 submit 都调不出**：C 缓解（认工具提交），但有能力下限——submit-only 通道只能强制「调用动作」，救不了吐空内容的模型；该结论已由本轮 live run 证实，必要时换强模型。
- **灰度回滚**：`derive_from_evidence` 默认 false + 单阶段 JSON 开关 → 一行回退；无注入即回退旧行为。

---

## 9. 验证计划（实现阶段，TDD）

- 纯函数单测：`derive_from_evidence` 仅派生 Found；有真 evidence 的 (资产×技术) 派生、无的仍 BLOCK；`checked_empty` 不被自动产生；歧义/缺技术不派生；无注入逐字节兼容。
- C 单测：侧信道有结构化交付物时 stage-close 命中（不再 missing_deliverable）；侧信道空仍 fail-closed。
- 活体：弱模型 + `target_intel`，对照修前（pause）→ 修后应能凭「确认 + 引用 evidence id」过 gate；证据落 `agent-progress.md`。
