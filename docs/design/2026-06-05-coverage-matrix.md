# 2026-06-05 · 覆盖矩阵（Coverage Matrix）：把「全面/方法够多」做成可度量的交付物结构

> **承接** `docs/design/2026-06-05-gate-rules-migration.md`（gate 已彻底数据驱动，`eval(deliverable, spec, rules)` 已带 `spec`）。本设计给 `StageDeliverable` 加一个**结构化 coverage 段**，并给 `gate_rules` 加配套积木，让攻击阶段的「**每个资产 × 每类技术都有终态、零 not_attempted**」成为确定性可校验的过关标准——这是「做得全面、方法够多」唯一能落地的载体。
>
> 关联：`types.rs::StageDeliverable`（加 `coverage`）、`gate/rule_engine.rs`（加 `Collection::Coverage` + `coverage_complete` op）、`stage_spec.rs`（加 `expected_techniques`）、`harness_submit_tool.rs`（schema + 提示）、`sprint_contract.rs`（未来从 skeleton/资产数据生成 expected）。

---

## 1. 背景：为什么现在装不下「全面」

`StageDeliverable`（`types.rs:161`）只有 `claims[] / findings[] / evidence_refs[] / skipped_checks[]` —— **一组平列表**。后果（gate-rules-migration §为什么 已点出）：`gate_rules` 只能表达「≥N 个 finding」「每个 finding 挂证据」，**无法表达**「目标 A 的 IDOR 测过没」「每个资产是否每类漏洞都有终态」。

而渗透「完整性」的工程定义恰恰是一个**矩阵的填满度**（AGENTS.md I8「已检查为空 ≠ 未检查」正是矩阵单元格的状态语义）：

```
            sqli      xss       idor      ssrf   ...
api.ex.com  found     checked-  found     blocked
            (poc)     empty     (poc)     (waf)
www.ex.com  n-a       checked-  ???←漏!   checked-
                      empty                empty
```

「???」= `not_attempted` = 当前模型**根本表达不出来**，所以「全面」无从校验。本设计补上这个数据结构 + 它的 gate。

---

## 2. 不变量 / 目标 / 非目标

**不变量**
- I-A：5 个结构 check + 现有 `gate_rules`（含 named_check）行为不变；coverage 是**加性**层（`#[serde(default)]` 空 = 今天行为）。
- I-B：gate 确定性、DB-free 主路；coverage 求值只读 `StageDeliverable` + `StageSpec`（与 migration 后的 `eval(_, spec, _)` 一致）。
- I-C：**「已检查为空 ≠ 未检查」**（I8）——`checked_empty` 是显式终态且要证据/理由；缺失（不在矩阵）= `not_attempted` = 不过关。

**目标**
- G1：`StageDeliverable` 能携带结构化 coverage（资产 × 技术 × 终态 + 证据/理由）。
- G2：`gate_rules` 能声明「coverage 完整」标准（纯 JSON，零 Rust），且 `found` 单元格必须挂证据。
- G3：expected「该测哪些技术」对标 stage 配置（后续可挂 OWASP WSTG / MITRE ATT&CK id），不靠 AI 自判。
- G4：fail-closed + 全单测；输出复用 `GateCheckOutcome` / `HarnessRecoveryActions`。

**非目标**
- 不在本期定死「资产」从哪来（资产收集是同事在做的活，模型在变）——见 §6 开放问题，MVP 用 deliverable 自报资产 + 留 hook。
- 不内置完整 WSTG/ATT&CK 清单（先支持 `expected_techniques` 字符串数组，taxonomy 词典化后续）。
- 不做 UI（纯后端 gate 能力）。

---

## 3. 数据模型（`types.rs`）

```rust
/// 一个 (资产 × 技术) 单元格的终态。缺失（不在矩阵）≡ not_attempted ≡ 不过关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Found,         // 测了且有发现 → 必须挂 evidence_refs
    CheckedEmpty,  // 测了、无发现 → 必须挂 evidence/note（I8：≠ 未测）
    Blocked,       // 被阻断（WAF/权限/越界）→ note 说明
    NotApplicable, // 该技术对该资产不适用 → note 说明
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageCell {
    pub asset: String,          // 如 "api.example.com" / "https://api.example.com/v1/orders"
    pub technique: String,      // 如 "idor" / "sqli"（后续可填 WSTG-id）
    pub status: CoverageStatus,
    #[serde(default)] pub evidence_refs: Vec<EvidenceAuditId>,
    #[serde(default)] pub note: Option<String>,  // checked_empty/blocked/n_a 的理由
}

// StageDeliverable 加（加性、向后兼容）：
#[serde(default)] pub coverage: Vec<CoverageCell>,
```

---

## 4. Gate 积木（`gate/rule_engine.rs`）

### 4.1 新集合 `Collection::Coverage` + 字段
让现有 `for_all` / `count_at_least` 能在 coverage 单元格上跑：
- `Collection::Coverage`
- `ItemField` 加 `Asset` / `Technique` / `Status`（`EvidenceRefs` 复用：coverage cell 也有 evidence_refs）。
- `Pred::Eq` 支持 `status`（按 snake_case 文本比较，复用现有 sev_to_str 模式加 status_to_str）。

例（**found 单元格必须挂证据**，纯数据规则）：
```json
{ "op":"for_all", "over":"coverage",
  "where":{"pred":"eq","field":"status","value":"found"},
  "require":{"pred":"non_empty","field":"evidence_refs"},
  "on_fail":{"reason":"every 'found' coverage cell must cite evidence"} }
```

### 4.2 新顶层 op `coverage_complete`（读 spec.expected_techniques）
表达「期望矩阵填满、零 not_attempted」。因需要外部「期望」集合，它**读 `spec`**（migration 后 `eval` 已带 spec，天然支持）：

```jsonc
{ "op":"coverage_complete",
  // 默认：deliverable.coverage 里出现过的 asset 集合 × spec.expected_techniques
  // 每个 (asset, technique) 必须有一个 status 终态的 cell（即在矩阵里）。
  "terminal_status": ["found","checked_empty","blocked","not_applicable"], // 可选，默认全部
  "on_fail": { "reason":"coverage incomplete: some (asset × technique) cells were never attempted",
               "hints":["fill every expected technique for each in-scope asset, or mark checked_empty/blocked/n_a with a reason"] } }
```

求值（纯函数）：
1. `techniques = spec.expected_techniques`（空 → 该 op 视为 Pass，no-op）。
2. `assets = deliverable.coverage 里出现的 distinct asset`（MVP；§6 资产源可换）。
3. 对每个 `(asset, technique)`：矩阵里若无对应 cell，或其 status 不在 `terminal_status` → 缺口。
4. 有缺口 → Block，reason 末尾附前 N 个缺失 `(asset, technique)`；recovery.hints 给补法。

> fail-closed：`expected_techniques` 缺省空 = 不强制（向后兼容）；`coverage_complete` / `CoverageStatus` / 新字段全 typed enum，写错名 spec 加载即报错（被 `all_twelve_stage_specs_load` 抓）。

### 4.3 `StageSpec` 加 `expected_techniques`
```rust
/// 本 stage 期望覆盖的技术类（coverage_complete 用）。空=不强制。
/// MVP 为自由字符串；后续可约束为 WSTG/ATT&CK id 词表。
#[serde(default)] pub expected_techniques: Vec<String>,
```

---

## 5. Agent 如何填（`harness_submit_tool.rs`）

- `parameters()` schema 加 `coverage` 数组（item: `{asset, technique, status, evidence_refs?, note?}`）+ 描述强调「checked_empty 必须有证据/理由，未测 = 不要进矩阵」。
- 构造零改动：`StageDeliverable` 走 `serde_json::from_value`，`#[serde(default)] coverage` 自动接。
- stage charter 提示（`prompts/mod.rs`）：当 `spec.expected_techniques` 非空，列出「本阶段必须对每个资产覆盖：<techniques>；每格给 found+证据 / checked_empty+证据 / blocked|n_a+理由」。

---

## 6. 开放问题（需你拍板，尤其与同事资产收集的接缝）

1. **资产维度从哪来？** MVP 用 deliverable 自报的 coverage cell asset 集合（简单、可立即跑，但 agent 可少报资产蒙混）。**更硬**的做法：assets 来自**继承证据 / 资产库**（同事的活），`coverage_complete` 从 `inherits_evidence_from` 或一个 caller 注入的 `asset_context` 取「in-scope 资产全集」。建议：MVP 自报 + 留 `eval_with_context` 注入资产集的 hook（与 evidence-KIND 同款 context 注入思路）。
2. **technique 词表**：MVP 自由字符串；是否现在就约束到 OWASP WSTG / MITRE ATT&CK id（+ taxonomy_ref 校验）？建议后续，先跑通结构。
3. **expected 放 spec 还是 skeleton？** spec（静态、按 stage）最简单、先用；skeleton（按 scope 动态生成，generator 现 deterministic）更贴「按目标定该测什么」，但要扩 generator。建议 MVP 放 spec.expected_techniques，留 skeleton 覆盖为 Phase 2。
4. **checked_empty/blocked 是否强制证据？** I8 严格意义上「检查为空」也要证据证明「确实检查过」。建议：`found` 强制 evidence（gate_rule）；`checked_empty` 至少要 `note` 或 evidence（可配）；MVP 先要 `found` 挂证据 + `checked_empty/blocked/n_a` 要 note。

---

## 6.5 决策与分期（2026-06-05 用户拍板）

用户对 §6 四问的选择 = **完整版**：①资产**从 DB**（不自报）；②technique **挂标准**（OWASP WSTG / MITRE ATT&CK id）；③expected **走 skeleton 动态生成**（按目标）；④`checked_empty` **也强制证据**。

但完整版的 ①/③ **强依赖同事尚未合并的资产库** + 碰 DB 按 AGENTS.md §2.7 需用户确认。故分两期：

- **Phase 1（2026-06-05 · MCP-4 已提交 `ca86a5ec`）= 数据模型 + 数据积木**：
  - 数据模型 `CoverageStatus` / `CoverageCell` / `StageDeliverable.coverage`（types.rs）。
  - gate 积木 `Collection::Coverage` + `ItemField::{Asset,Technique,Status}` + `Pred::Eq` 支持 status（rule_engine.rs）。
  - ④ 落地：数据规则 `for_all over coverage where status==found require non_empty evidence_refs`。

- **Phase 1.5（2026-06-05 · MCP-1 增量 = 确定性 coverage 闸端到端跑通，不依赖资产库）**：
  - `coverage_complete` op（rule_engine.rs，纯函数）：读 `spec.expected_techniques` × coverage **自报**资产集，逐 (asset × technique) 核终态；缺口聚合进 Block reason（前 8 个）。`terminal_status` 可选、缺省四态；expected 空 → no-op。
  - `StageSpec.expected_techniques`（stage_spec.rs，静态）。
  - submit 工具 `coverage` schema + `stage_charter` 当 expected 非空时列出技术 + 每格契约（prompts/mod.rs）。
  - 样例 `vuln_triage`：`expected_techniques` 用真实 **WSTG id**（②「挂标准」在数据层落地）+ `coverage_complete` + **found + checked_empty 双证据规则（④「checked_empty 也要证据」落地，I8）** + gate 集成测试。
  - 验证：单测 + nextest（kit+app）全绿 + clippy -D + fmt（见 agent-progress）。

- **Phase 2（deferred · 待资产库合入 + 用户 DB 确认）= 活体接线 + 标准/动态硬化**：
  - **①** `coverage_complete` 的资产维度从 **DB** 注入 in-scope 资产全集（阶段收尾外层查库 → 经 `eval_with_context` 注入，gate 仍纯函数），替代当前自报集合，堵「少报资产蒙混」（§8）。
  - **③** skeleton **动态生成** `expected_techniques`（扩 `DefaultSprintContractGenerator`，输入含真实目标/资产数据），替代当前静态 spec 字段。
  - **②** technique 字符串**约束/映射到 WSTG/ATT&CK 词典 + 校验**（当前仅字符串约定 + 样例用真实 WSTG id）。

> 即：本次（Phase 1.5）coverage 闸已**端到端可用**（自报资产 + 静态期望 + WSTG id 约定）；「按 DB 资产核完整性 + 按目标动态期望 + 标准词典对标」三项硬化待资产库到位、用户授权 DB 后接上。

---

## 7. 集成点（文件级）

| # | 改动 | 位置 |
|---|---|---|
| 7.1 | `CoverageStatus` + `CoverageCell` + `StageDeliverable.coverage` | `harness/types.rs` |
| 7.2 | `Collection::Coverage` + `ItemField::{Asset,Technique,Status}` + `Pred::Eq` status 支持 + `resolve` coverage 分支 | `gate/rule_engine.rs` |
| 7.3 | 新 op `GateRule::CoverageComplete` + 求值（读 spec.expected_techniques） | `gate/rule_engine.rs` |
| 7.4 | `StageSpec.expected_techniques` | `stage_spec.rs` |
| 7.5 | submit 工具 `coverage` schema + 提示 | `harness_submit_tool.rs` + `prompts/mod.rs::stage_charter` |
| 7.6 | 样例：给 1 个攻击 stage（如 vuln_triage / verification）配 `expected_techniques` + coverage gate_rules | `resources/harness/stages/<stage>.json` |
| 7.7 | DSL 速查补 coverage + 设计指针 | `docs/design/2026-06-02-harness-stage-spec-reference.md` |

依赖方向：仍只 `harness::types` + `gate::GateCheckOutcome` + `stage_spec::StageSpec`，无新 crate 依赖、无环。

---

## 8. 风险与边界

- **agent 少报资产**：MVP 自报资产可被绕（少报 asset → 矩阵小）。缓解：§6.1 的资产集注入（Phase 2）；MVP 期先靠 `min_findings`/expected_findings 数量下限 + 人工抽查。
- **矩阵爆炸**：资产 × 技术可能很大。缓解：`expected_techniques` 按 stage 精选；coverage 只要求 expected 子集，不是笛卡尔全集。
- **与 ledger 解耦**：`found` 单元格的 evidence_refs 仍只校验「存在 id」（deliverable 层）；KIND 校验沿用 caller-side（与 finding 一致）。
- **风险等级**：中（gate 核心链路 + 改 deliverable contract）。TDD + `just test-harness` + clippy -D + 受影响 crate nextest/fmt 收口；按 AGENTS.md §2.5 先设计后实现。

---

## 9. 验证计划

1. 单测（rule_engine）：coverage 数据规则（found 缺证据 Block）；`coverage_complete`（缺 (asset×technique) Block、全终态 Pass、expected 空时 no-op、status 写错 serde Err）。
2. 类型 serde 往返：CoverageCell / CoverageStatus；StageDeliverable 带/不带 coverage 都解析。
3. 集成：带 `expected_techniques` + coverage gate_rules 的内联 spec，矩阵有缺口 → `validate_stage_gate` Block。
4. 回归：`all_twelve_stage_specs_load` + 现有 harness 测试全绿（coverage 缺省空，旧 stage 零影响）。
5. 门禁：`cargo nextest -p golish-agent-kit -p golish-agent-app` + `clippy -D warnings` + `fmt --check` 全绿；证据留痕 `agent-progress.md`。

---

## 10. 落地次序（供 writing-plans 展开）

1. 数据模型（types：CoverageStatus/CoverageCell/coverage 字段）+ serde 测试。
2. rule_engine：Coverage 集合 + 字段 + Eq status；for_all over coverage 测试。
3. rule_engine：`coverage_complete` op + 求值（读 spec.expected_techniques）+ 全分支测试。
4. stage_spec：`expected_techniques` 字段 + 解析测试。
5. submit 工具 schema + stage charter 提示。
6. 样例 stage 配 expected_techniques + coverage gate_rules + 集成测试。
7. 文档（DSL 速查 + 指针）+ 收口验证 + progress/feature_list 登记。
