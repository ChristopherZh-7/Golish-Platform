# vuln_triage 技术矩阵 + 分母覆盖 实现计划

> **面向 AI 代理的工作者：** 必需子技能：`.cursor/skills/executing-plans` 逐任务实现；每任务 `.cursor/skills/test-driven-development`（先写失败测试 → 红 → 实现 → 绿 → commit）；收口 `.cursor/skills/verification-before-completion`。设计：`docs/design/2026-06-05-vuln-triage-technique-matrix.md`（D1-D8 全 ✅ decided）。

**目标：** 给 `vuln_triage` 配齐外网 web 的 15 类期望技术（记分层），并给 coverage 矩阵补「分母覆盖」——cell 必须对着 enumeration 产出的可测单元分母 M 报 `tested/total`，默认全覆盖、抽样需理由，杜绝「跑 3/5000 谎称 checked_empty」。

**架构：** 纯加性扩展现有 coverage matrix（`coverage-matrix` 设计已落地 Phase 1.5）。给 `CoverageCell` 加 3 个 `#[serde(default)]` 字段；给 `rule_engine` 加一个纯函数 gate op `CoverageDenominator`；给 `vuln_triage.json` 填 `expected_techniques` + 两条新规则；charter 补提示。全部向后兼容（旧 deliverable 不带新字段 = 旧行为）。

**技术栈：** Rust 2021（`golish-agent-kit`：`harness/types.rs` / `harness/gate/rule_engine.rs` / `harness/stage_spec.rs` / `harness/prompts/mod.rs`）+ JSON 资源（`resources/harness/stages/vuln_triage.json` / `resources/harness/evidence_kinds.json`）+ `golish-agent-app`（submit 工具 schema，如需）。

---

## 前置约定

- **受影响 crate**：`golish-agent-kit`（types / rule_engine / stage_spec / prompts）+ `golish-agent-app`（submit 工具 schema）。
- **先读这些文件再动手**（精确行级编辑依赖其当前内容）：
  - `backend/crates/golish-agent-kit/src/harness/types.rs`（`CoverageStatus` / `CoverageCell` / `StageDeliverable`，约 157-211 行）
  - `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`（`GateRule` 枚举、`eval_one`、`coverage_complete`、`block_from`/Block 构造辅助、`items()`）
  - `backend/crates/golish-agent-kit/src/harness/stage_spec.rs`（`StageSpec.expected_techniques`）
  - `backend/crates/golish-agent-kit/src/harness/prompts/mod.rs`（`stage_charter`）
  - `resources/harness/stages/vuln_triage.json`
- **验证命令**（每任务跑相关子集，收口跑全套）：
  - `cd backend && cargo nextest run -p golish-agent-kit -E 'test(rule_engine)|test(harness::types)|test(coverage)' --status-level fail`
  - `cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --status-level fail`
  - `cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings && cargo fmt --check`
  - `python3 -m json.tool resources/harness/stages/vuln_triage.json >/dev/null`
- **可回滚**：3 个新 cell 字段 `#[serde(default)]`（缺 = 0 / None）；`CoverageDenominator` 只在 spec 声明时生效；vuln_triage 改动是数据。
- **高风险确认**：本计划不碰 DB / migration / 已发布 ts-rs，属低风险；但落 commit / push 仍按 `AGENTS.md §2.6/§2.7` 走（precommit 全绿 + 用户授权 push）。

---

## 文件结构

| 文件 | 职责 | 任务 |
|---|---|---|
| `harness/types.rs` | `CoverageCell` 加 `tested_units/total_units/sampling_rationale` | T1 |
| `harness/gate/rule_engine.rs` | 新 op `GateRule::CoverageDenominator` + `coverage_denominator()` 纯函数 + `eval_one` 分支 | T2 |
| `resources/harness/stages/vuln_triage.json` | `expected_techniques`=15 类 + `coverage_denominator` 规则（沿用已有 found/checked_empty 证据规则 + `coverage_complete`） | T3 |
| `harness/prompts/mod.rs` | `stage_charter` 补「每类证据契约 + 分母/全覆盖/抽样」提示 | T4 |
| `resources/harness/evidence_kinds.json` | 为新技术证据 kind 补 freshness 默认（可选） | T5 |
| `harness/gate/mod.rs` | vuln_triage 端到端集成测试（分母 Block / 全覆盖 Pass / 抽样 Pass） | T6 |
| `docs/design/2026-06-02-harness-stage-spec-reference.md` + progress/feature_list | DSL 速查补 `coverage_denominator` + 收口登记 | T7 |

---

## Task 1 · CoverageCell 加分母字段（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/types.rs`（实现 + serde 测试）

**1.1（红）** 在 types.rs 的 `#[cfg(test)] mod tests` 加一个 serde 往返测试：构造带 `tested_units/total_units/sampling_rationale` 的 `CoverageCell`，序列化再反序列化相等；并测「旧 JSON（不带这 3 字段）反序列化 → tested_units=0, total_units=0, sampling_rationale=None」。

```rust
#[test]
fn coverage_cell_denominator_serde_roundtrip_and_default() {
    let c = CoverageCell {
        asset: "api.example.com".into(),
        technique: "WSTG-INPV-05".into(),
        status: CoverageStatus::CheckedEmpty,
        evidence_refs: vec![EvidenceAuditId::new(1)],
        note: Some("scanned".into()),
        tested_units: 12,
        total_units: 12,
        sampling_rationale: None,
    };
    let j = serde_json::to_string(&c).unwrap();
    let back: CoverageCell = serde_json::from_str(&j).unwrap();
    assert_eq!(back.tested_units, 12);
    assert_eq!(back.total_units, 12);
    assert!(back.sampling_rationale.is_none());

    // 旧 JSON（无新字段）→ default
    let old: CoverageCell = serde_json::from_str(
        r#"{"asset":"a","technique":"t","status":"found","evidence_refs":[1]}"#,
    )
    .unwrap();
    assert_eq!(old.tested_units, 0);
    assert_eq!(old.total_units, 0);
}
```

**1.2（运行确认失败）** `cargo nextest run -p golish-agent-kit -E 'test(coverage_cell_denominator_serde)'` → 编译失败（字段不存在）。

**1.3（绿）** 给 `CoverageCell` 加 3 字段（保留现有 asset/technique/status/evidence_refs/note 不动）：

```rust
pub struct CoverageCell {
    pub asset: String,
    pub technique: String,
    pub status: CoverageStatus,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceAuditId>,
    #[serde(default)]
    pub note: Option<String>,
    // ── 分母覆盖（设计 2026-06-05-vuln-triage-technique-matrix §5）──
    /// N：该 (资产×技术) 实际测过的可测单元数。
    #[serde(default)]
    pub tested_units: u32,
    /// M：分母，来自 enumeration 的可测单元清单（接口/参数/路径/服务）。
    #[serde(default)]
    pub total_units: u32,
    /// 抽样时必填的理由；为 None 时按全覆盖（tested==total）要求。
    #[serde(default)]
    pub sampling_rationale: Option<String>,
}
```

**1.4** 修因加字段而炸的 `CoverageCell { … }` 字面构造（**逐字段构造的才炸；用 `..` 的不炸**）。定位：`rg "CoverageCell \{" backend/crates`。已知点：`gate/rule_engine.rs` 测试 helper（`cov_cell`）、`gate/mod.rs` 的 vuln_triage 集成测试 fixture。给每个补 `tested_units: 0, total_units: 0, sampling_rationale: None,`（或在 helper 里集中加）。

> 建议：把 rule_engine 测试里的 `cov_cell(asset, tech, status, evidence)` helper 升级成默认 `tested=total=证据数或1`，新增 `cov_cell_dn(asset, tech, status, evidence, tested, total, rationale)`，避免逐处改。

**验证：** `cargo nextest run -p golish-agent-kit -E 'test(harness::types)'` 绿 + `cargo check -p golish-agent-kit --tests`（找全字面构造）。**commit**：`feat(harness): add denominator fields (tested/total/sampling) to CoverageCell`。

---

## Task 2 · rule_engine 新 op `CoverageDenominator`（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

**2.1（红）** 加测试（构造 deliverable.coverage 含分母字段）：
- found cell `tested<total` 且无 rationale → Block，reason 含 `tested N/M`。
- found cell `tested==total` → Pass。
- checked_empty cell `tested<total` 但有 `sampling_rationale` 且 `tested/total ≥ min_sample_ratio_pct` → Pass。
- blocked / not_applicable cell 不要求分母（即使 tested=0/total=0）→ Pass。
- `total_units==0` 且 status=checked_empty → Block（应改用 not_applicable）。
- serde：`coverage_denominator` 解析；`min_sample_ratio_pct` 缺省=100。

```rust
fn denominator_rule(min_ratio: Option<u8>) -> GateRule {
    let body = match min_ratio {
        Some(r) => format!(
            r#"{{ "op":"coverage_denominator","min_sample_ratio_pct":{r},
                 "on_fail":{{"reason":"coverage below denominator"}} }}"#
        ),
        None => r#"{ "op":"coverage_denominator",
                 "on_fail":{"reason":"coverage below denominator"} }"#
            .to_string(),
    };
    parse(&body)
}

#[test]
fn denominator_blocks_partial_without_rationale() {
    let rule = denominator_rule(None);
    let d = deliverable_with_coverage(vec![CoverageCell {
        asset: "a".into(), technique: "WSTG-INPV-05".into(),
        status: CoverageStatus::CheckedEmpty, evidence_refs: vec![EvidenceAuditId::new(1)],
        note: None, tested_units: 3, total_units: 5000, sampling_rationale: None,
    }]);
    let out = eval_one(&d, &test_spec(), &rule);
    match out {
        GateCheckOutcome::Block { reasons, .. } =>
            assert!(reasons.iter().any(|r| r.contains("3/5000"))),
        _ => panic!("expected Block"),
    }
}

#[test]
fn denominator_passes_full_and_sampled() {
    // 全覆盖
    let full = deliverable_with_coverage(vec![CoverageCell {
        asset: "a".into(), technique: "WSTG-INPV-05".into(),
        status: CoverageStatus::CheckedEmpty, evidence_refs: vec![EvidenceAuditId::new(1)],
        note: None, tested_units: 5000, total_units: 5000, sampling_rationale: None,
    }]);
    assert!(eval_one(&full, &test_spec(), &denominator_rule(None)).is_pass());

    // 抽样：80% ≥ 阈值 80
    let sampled = deliverable_with_coverage(vec![CoverageCell {
        asset: "a".into(), technique: "WSTG-INPV-05".into(),
        status: CoverageStatus::CheckedEmpty, evidence_refs: vec![EvidenceAuditId::new(1)],
        note: None, tested_units: 4000, total_units: 5000,
        sampling_rationale: Some("long-tail low-risk endpoints sampled".into()),
    }]);
    assert!(eval_one(&sampled, &test_spec(), &denominator_rule(Some(80))).is_pass());
}
```

**2.2（运行确认失败）** `cargo nextest run -p golish-agent-kit -E 'test(rule_engine)'` → 失败（op 不存在）。

**2.3（绿）** 在 `GateRule` 枚举加变体（紧随 `CoverageComplete`，沿用其 `#[serde]` 风格 + tag = `coverage_denominator`）：

```rust
/// 分母覆盖（设计 2026-06-05-vuln-triage-technique-matrix §5.3）。
/// 对 status ∈ {found, checked_empty} 的每个 cell 核分母：
///   - 默认全覆盖（D6）：tested_units == total_units；
///   - 抽样例外：sampling_rationale 非空 且 tested*100 ≥ min_sample_ratio_pct*total。
/// blocked / not_applicable 不要求分母。total==0 的 found/checked_empty 视为缺口。
CoverageDenominator {
    #[serde(default = "default_sample_ratio_pct")]
    min_sample_ratio_pct: u8,
    on_fail: OnFail,
},
```

加默认值函数 + `summary()` 与 `eval_one()` 分支 + 求值纯函数：

```rust
fn default_sample_ratio_pct() -> u8 { 100 }

// GateRule::summary() 的 reason 抽取分支里把 CoverageDenominator 并入
// CountAtLeast | ForAll | CoverageComplete 那条 on_fail.reason 分支。

// eval_one match 里加：
GateRule::CoverageDenominator { min_sample_ratio_pct, on_fail } =>
    coverage_denominator(d, *min_sample_ratio_pct, on_fail),

fn coverage_denominator(d: &StageDeliverable, min_ratio_pct: u8, on_fail: &OnFail) -> GateCheckOutcome {
    let mut gaps: Vec<String> = Vec::new();
    for c in &d.coverage {
        if !matches!(c.status, CoverageStatus::Found | CoverageStatus::CheckedEmpty) {
            continue; // blocked / not_applicable 免分母
        }
        if c.total_units == 0 {
            gaps.push(format!(
                "{}×{}: total_units=0 with status {:?}; use not_applicable if no testable units",
                c.asset, c.technique, c.status
            ));
            continue;
        }
        let full = c.tested_units >= c.total_units;
        let sampled_ok = c
            .sampling_rationale
            .as_ref()
            .map(|r| !r.trim().is_empty())
            .unwrap_or(false)
            && (c.tested_units as u64) * 100 >= (min_ratio_pct as u64) * (c.total_units as u64);
        if !full && !sampled_ok {
            gaps.push(format!(
                "{}×{}: tested {}/{} without sampling_rationale (partial — D6 default is full coverage)",
                c.asset, c.technique, c.tested_units, c.total_units
            ));
        }
    }
    if gaps.is_empty() {
        GateCheckOutcome::Pass
    } else {
        // 复用本文件 coverage_complete 用的 Block 构造（gaps 拼进 reason 末尾 + on_fail.hints 进
        // HarnessRecoveryActions.hints）。若有 block_from(on_fail, gaps) 辅助则直接调用之。
        block_from(on_fail, gaps)
    }
}
```

> ⚠ `block_from` 是占位名——实现时**先看 `coverage_complete` 末尾怎么构造 Block**（它把缺口聚合进 reason），照搬同一构造（可能是内联 `GateCheckOutcome::Block { reasons, recovery }` 或一个私有 helper），不要新造风格。

**2.4（绿运行）** `cargo nextest run -p golish-agent-kit -E 'test(rule_engine)'` → 全绿。

**验证：** 同上 + `cargo clippy -p golish-agent-kit -- -D warnings`。**commit**：`feat(harness): coverage_denominator gate op (full coverage default + sampled exception)`。

---

## Task 3 · vuln_triage.json：15 类 expected_techniques + 分母规则（数据）

**文件：** `resources/harness/stages/vuln_triage.json`

**3.1** 把 `expected_techniques` 从 4 个替换为 15 类（设计 §3，D1 混用 id / D2 `GOLISH-NDAY`）：

```json
"expected_techniques": [
  "WSTG-INPV-05", "WSTG-INPV-01", "WSTG-INPV-12", "WSTG-INPV-18", "WSTG-INPV-19",
  "WSTG-ATHZ-04", "WSTG-ATHZ-01", "WSTG-ATHN-04", "WSTG-ATHN-02", "WSTG-SESS-02",
  "WSTG-CONF-05", "WSTG-CRYP-03", "WSTG-BUSL", "WSTG-INFO", "GOLISH-NDAY"
]
```

**3.2** 在 `gate_rules` 数组**追加**一条 `coverage_denominator`（保留已有 claims/findings 证据规则 + found/checked_empty 证据规则 + `coverage_complete`）：

```json
{
  "op": "coverage_denominator",
  "min_sample_ratio_pct": 100,
  "on_fail": {
    "reason": "coverage below denominator: a tested/total cell did not cover its enumeration surface and gave no sampling_rationale",
    "hints": [
      "cover every enumerated unit (endpoint/param) for the technique, OR set sampling_rationale + raise tested ratio",
      "if the asset has no testable unit for this technique, use not_applicable + note instead of checked_empty"
    ]
  }
}
```

**3.3** 校验 JSON：`python3 -m json.tool resources/harness/stages/vuln_triage.json >/dev/null` → exit 0。

**验证：** `cargo nextest run -p golish-agent-kit -E 'test(all_twelve)|test(stage_spec)'` 绿（spec 解析 + 12 spec 全加载）。**commit**：`feat(harness): wire 15 expected techniques + coverage_denominator on vuln_triage`。

---

## Task 4 · charter 提示（prompts/mod.rs）

**文件：** `backend/crates/golish-agent-kit/src/harness/prompts/mod.rs`（`stage_charter`）

**4.1** 先读 `stage_charter` 现有「当 `spec.expected_techniques` 非空时列技术」那段。在其后追加一段（D3 charter 侧 + D6 分母）：

> 文案（拼进 charter 字符串，措辞按现有风格调整）：
> 「本阶段对**每个资产 × 每类期望技术**给终态：found+证据 / checked_empty+证据 / blocked|not_applicable+理由（§coverage）。
> **覆盖必须对着分母**：每格填 `tested_units / total_units`（total 来自 enumeration 的接口/参数清单）。默认要求 `tested == total`（全覆盖）；确需抽样时必须填 `sampling_rationale` 且覆盖率达标，否则视为未测完（partial）会被拦。
> 这是地板不是天花板：鼓励超出清单做串链 / 业务逻辑（BUSL 的 checked_empty 必须附"试过哪些逻辑路径"的证据）。」

**4.2** 若 `prompts` 有快照/字符串测试，更新断言；否则加一个最小测试：expected_techniques 非空时 charter 含「tested_units」「sampling_rationale」关键词。

**验证：** `cargo nextest run -p golish-agent-kit -E 'test(prompts)'`（若无则 `cargo check -p golish-agent-kit`）绿。**commit**：`feat(harness): stage charter explains denominator + per-technique evidence contract`。

---

## Task 5 · evidence_kinds.json freshness（可选 · D3 kind 侧）

**文件：** `resources/harness/evidence_kinds.json`

**5.1** 若希望新技术证据也受时效约束，按现有 `{ "<kind>": { "default_max_age_secs": N } }` 形态补条目（示例，按需裁剪）：

```json
"sqli_scan":   { "default_max_age_secs": 604800 },
"dir_scan":    { "default_max_age_secs": 604800 },
"nday_poc_run":{ "default_max_age_secs": 86400 }
```

**5.2** 校验 JSON + 既有 evidence_kinds 加载测试（`rg "evidence_kinds" backend/crates/golish-agent-kit/src` 找加载点与测试）。

> 注：「什么证据算测过」的**语义**契约主要落 charter（Task 4）；本任务只补 freshness 默认。若评审认为现在不需要，**跳过本任务**（YAGNI）。

**验证：** `python3 -m json.tool resources/harness/evidence_kinds.json >/dev/null` + 相关 nextest 绿。**commit**：`feat(harness): freshness defaults for vuln_triage evidence kinds`。

---

## Task 6 · gate/mod.rs 端到端集成测试

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/mod.rs`（`#[cfg(test)] mod tests`）

**6.1** 用迁移后的 vuln_triage embedded spec（`load_embedded_stage_spec(StageKind::VulnTriage)`），加测试：
- **Pass**：一个资产对 15 类全给终态，found/checked_empty 挂证据，每格 `tested==total` → `validate_stage_gate` allowed。
- **Block（缺技术）**：删掉 `GOLISH-NDAY` 那格 → `coverage_complete` Block，reason 含 `GOLISH-NDAY`。
- **Block（分母）**：某 checked_empty 格 `tested=3,total=5000,无 rationale` → `coverage_denominator` Block，reason 含 `3/5000`。
- **Pass（抽样）**：同格补 `sampling_rationale` 且 `tested=5000`（或阈值内）→ allowed。

```rust
#[test]
fn vuln_triage_denominator_blocks_partial_and_passes_when_full() {
    use super::super::resources::load_embedded_stage_spec;
    use super::super::types::{CoverageCell, CoverageStatus, StageKind, /* … */};
    let spec = load_embedded_stage_spec(StageKind::VulnTriage).unwrap();
    assert_eq!(spec.expected_techniques.len(), 15);
    // 构造 full-coverage deliverable（15 格，tested==total，found/checked_empty 挂证据）→ Pass
    // 复制其中一格改成 tested=3,total=5000,无 rationale → Block 且 reason 含 "3/5000"
    // …（参照 rule_engine 测试 fixture 拼装；evidence_refs 用 EvidenceAuditId::new(1)）
}
```

**验证：** `just test-harness`（或 `cargo nextest run -p golish-agent-kit -E 'test(gate)'`）+ `all_twelve_stage_specs_load` 绿。**commit**：`test(harness): vuln_triage technique-matrix + denominator integration tests`。

---

## Task 7 · 文档 + 收口

**7.1** `docs/design/2026-06-02-harness-stage-spec-reference.md` §8 DSL 速查补 `coverage_denominator`（字段 `min_sample_ratio_pct` + 语义）+ 指针到本设计/计划。

**7.2** 收口验证：
```bash
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-app --no-fail-fast --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -- -D warnings && cargo fmt --check
python3 -m json.tool resources/harness/stages/vuln_triage.json >/dev/null
```
（纯 Rust + JSON，无前端面；如需绝对门禁再 `just precommit`。）

**7.3** 证据复制进 `agent-progress.md`（命令 + 通过计数）；`feature_list.json` 加/更 `vuln-triage-technique-matrix-2026-06-05`（passing + evidence）。**commit**：`chore(harness): record vuln_triage technique-matrix evidence + feature status`。

---

## 自检

**规格覆盖度**（对照设计）：§3 技术矩阵 → T3；§5.1-5.3 分母/全覆盖/抽样 → T1（字段）+ T2（gate op）+ T3（规则）；§5.4 诚实底线（not_applicable 免分母、total=0 报缺口）→ T2 求值分支；D3 evidence 契约 → T4（charter）+ T5（kind）；§4 cell 端到端 → T6 集成测试；§9 落地次序 → T1-T7。

**占位符扫描**：仅一处显式标注的占位 `block_from`（T2.3），已给「先看 coverage_complete 怎么构造 Block」的明确指引，非 TODO。其余均有具体代码/命令。

**类型一致性**：`tested_units: u32` / `total_units: u32` / `sampling_rationale: Option<String>` 在 T1 定义，T2/T6 一致引用；`CoverageDenominator { min_sample_ratio_pct, on_fail }` 在 T2 定义，T3 JSON 字段名 `min_sample_ratio_pct` 与之一致；`GOLISH-NDAY` 在 T3 出现、T6 断言引用一致。

**YAGNI / 加性 / 可回滚**：3 字段 + 1 op 全 `#[serde(default)]` / spec 声明才生效；T5 可跳过；动态裁剪 / DB 资产注入 / WSTG 词典均为设计 Phase 2，不在本计划。

**TDD / 频繁 commit**：T1-T6 均「红 → 绿 → commit」，每任务一 commit。
