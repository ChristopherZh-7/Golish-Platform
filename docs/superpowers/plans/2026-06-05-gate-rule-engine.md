# Gate 规则引擎（数据驱动 `gate_rules`）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `.cursor/skills/executing-plans` 逐任务实现此计划；每个任务遵循 `.cursor/skills/test-driven-development`（先写失败测试 → 跑红 → 实现 → 跑绿 → commit）。

**目标：** 给 harness stage gate 加一个数据驱动的 `gate_rules` 引擎——stage JSON 用固定积木 op 声明过关标准，纯函数解释器执行，新增标准零 Rust 改动。
**架构：** 新增 `gate/rule_engine.rs`（typed enum DSL + 纯函数 `eval`），在 `validate_stage_gate_with_skeleton` 末尾把 `eval` 的结果并进现有 `outcomes` 聚合；`StageSpec` 加 `#[serde(default)] gate_rules` 字段。fail-closed 由 serde typed enum 在 spec 加载期天然达成。
**技术栈：** Rust 2021、serde（internally-tagged enum）、cargo nextest；设计见 `docs/design/2026-06-05-gate-rule-engine.md`。

---

## 背景速读（零上下文工程师必读）

- gate 调度入口 `backend/crates/golish-agent-kit/src/harness/gate/mod.rs::validate_stage_gate_with_skeleton`：先跑 5 个结构性 check，再按 `spec.required_checks` 字符串经一个写死的 `match` 选跑语义 check，未命中名字走 `_ => continue`（静默忽略）。
- 已有"配置驱动"范本：`gate/finding_verification_check.rs`（纯读 spec 字段、DB-free、可单测）。本引擎是它的泛化。
- 求值输入 contract：`harness/types.rs::StageDeliverable { claims: Vec<StageClaim>, findings: Vec<HarnessFinding>, evidence_refs, skipped_checks, ... }`；`StageClaim { kind, subject, summary, evidence_ids }`；`HarnessFinding { finding_id, kind, subject, severity: FindingSeverity, evidence_refs }`。
- `FindingSeverity::rank()`（types.rs）给 Info=0..Critical=4，供 `severity_at_least` 比较。
- **唯一会被新字段破坏编译的字面构造点**：`finding_verification_check.rs` 测试里的 `spec_with(...)`（手写了全部 StageSpec 字段）。其余代码全走 `load_stage_spec_from_json` / `load_embedded_stage_spec`（serde 反序列化），靠 `#[serde(default)]` 自动兼容。

## 文件结构（创建/修改清单）

| 文件 | 职责 | 动作 |
|---|---|---|
| `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs` | DSL 类型 + 纯函数 `eval` + 单测 | **新建** |
| `backend/crates/golish-agent-kit/src/harness/gate/mod.rs` | `pub mod rule_engine;` + 调度入口并进 `eval` + 集成测试 | 改 |
| `backend/crates/golish-agent-kit/src/harness/stage_spec.rs` | `StageSpec` 加 `gate_rules` 字段 | 改 |
| `backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs` | `spec_with` 测试 helper 补 `gate_rules: vec![]` + 等价性测试 | 改 |
| `resources/harness/stages/verification.json` | 补一条样例 `gate_rules`（复刻其 finding_verification） | 改 |
| `docs/design/2026-06-02-harness-stage-spec-reference.md` | 追加 DSL 速查表 | 改 |

约束：DRY、YAGNI、TDD、每个任务单独 commit。所有命令在 repo 根跑。

---

## Task 1 · 新建 `rule_engine.rs`：DSL 类型 + serde 往返测试（红→绿）

**文件：** 新建 `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

**步骤 1.1** 写文件，先放类型 + 一个 serde 解析测试（此时 `eval` 未实现，编译会因后续测试引用而红——本任务只引入类型与解析测试，`eval` 在 Task 2 加）：

```rust
//! 数据驱动 gate 规则引擎（设计 `docs/design/2026-06-05-gate-rule-engine.md`）。
//!
//! stage JSON 用固定积木 op 声明过关标准；本模块是纯函数、DB-free、确定性解释器，
//! 输出复用 `GateCheckOutcome` / `HarnessRecoveryActions`，由 `gate/mod.rs` 并进聚合。
//! fail-closed：op / pred / over / field 全是 serde enum，写错的名字在 StageSpec
//! 反序列化期即报错（被 `resources` 的 all_twelve_stage_specs_load 单测抓住）。

use serde::{Deserialize, Serialize};

use super::super::types::{
    FindingSeverity, HarnessFinding, HarnessRecoveryActions, StageClaim, StageDeliverable,
};
use super::GateCheckOutcome;

/// 一条规则 = 一个顶层积木 op；求值产出一个 `GateCheckOutcome`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GateRule {
    /// 满足 `where` 的元素至少 `min` 个。
    CountAtLeast {
        over: Collection,
        #[serde(default, rename = "where")]
        filter: Option<Pred>,
        min: u32,
        on_fail: OnFail,
    },
    /// 满足 `where` 的每个元素都必须满足 `require`（全称；空集合为真）。
    ForAll {
        over: Collection,
        #[serde(default, rename = "where")]
        filter: Option<Pred>,
        require: Pred,
        on_fail: OnFail,
    },
}

/// MVP 只含有可寻址字段的两个集合。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Claims,
    Findings,
}

/// 叶子谓词，对单个元素求值。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "pred", rename_all = "snake_case")]
pub enum Pred {
    /// 数组/字符串字段非空。
    NonEmpty { field: ItemField },
    /// 字符串字段等于 value（severity 按 snake_case 文本比较）。
    Eq { field: ItemField, value: String },
    /// finding.severity rank ≥ min。
    SeverityAtLeast { min: FindingSeverity },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemField {
    Kind,
    Subject,
    Summary,
    EvidenceRefs,
    EvidenceIds,
    Severity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnFail {
    pub reason: String,
    #[serde(default)]
    pub hints: Vec<String>,
    #[serde(default)]
    pub repair_tool_calls: Vec<String>,
    #[serde(default)]
    pub missing_evidence_kinds: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_count_at_least_rule() {
        let json = r#"{
            "op": "count_at_least",
            "over": "findings",
            "where": { "pred": "eq", "field": "kind", "value": "subdomain" },
            "min": 1,
            "on_fail": { "reason": "need a subdomain" }
        }"#;
        let rule: GateRule = serde_json::from_str(json).expect("parse");
        assert!(matches!(rule, GateRule::CountAtLeast { min: 1, .. }));
    }

    #[test]
    fn parses_for_all_rule() {
        let json = r#"{
            "op": "for_all",
            "over": "findings",
            "where": { "pred": "severity_at_least", "min": "high" },
            "require": { "pred": "non_empty", "field": "evidence_refs" },
            "on_fail": { "reason": "high+ finding needs evidence" }
        }"#;
        let rule: GateRule = serde_json::from_str(json).expect("parse");
        assert!(matches!(rule, GateRule::ForAll { .. }));
    }

    #[test]
    fn unknown_op_fails_closed() {
        let json = r#"{ "op": "coverage_matrix", "over": "findings", "min": 1,
                        "on_fail": { "reason": "x" } }"#;
        assert!(serde_json::from_str::<GateRule>(json).is_err());
    }

    #[test]
    fn unknown_pred_fails_closed() {
        let json = r#"{ "op": "for_all", "over": "findings",
            "require": { "pred": "magic", "field": "kind" },
            "on_fail": { "reason": "x" } }"#;
        assert!(serde_json::from_str::<GateRule>(json).is_err());
    }
}
```

**步骤 1.2** 在 `gate/mod.rs` 顶部模块声明区加一行（让新文件被编译）：

```rust
pub mod rule_engine;
```
（加在现有 `pub mod vacuous_check;` 一组声明之后。）

**步骤 1.3** 跑测试（此时应**编译通过 + 4 个 parse 测试绿**；`eval` 还没写，没人引用，不报错）：

```bash
just test-harness
```
预期：`golish-agent-kit` 编译通过，`rule_engine::tests::parses_*` / `unknown_*_fails_closed` 4 个测试 PASS，其余 harness 测试不变。

**步骤 1.4** Commit：

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/harness/gate/mod.rs
git commit -m "feat(harness): add gate_rules DSL types + serde fail-closed tests"
```

---

## Task 2 · 实现 `eval`（count_at_least + for_all + 谓词求值）（TDD）

**文件：** `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`

**步骤 2.1** 先在 `tests` 模块追加**失败测试**（引用尚未实现的 `eval`，会编译失败 = 红）：

```rust
    use super::super::GateCheckOutcome;
    use crate::harness::types::{FindingSeverity, HarnessFinding, StageClaim, StageDeliverable};
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

    fn finding(kind: &str, sev: FindingSeverity, refs: Vec<i64>) -> HarnessFinding {
        HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: kind.to_string(),
            subject: "x.example.com".to_string(),
            severity: sev,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
        }
    }

    fn deliverable(findings: Vec<HarnessFinding>, claims: Vec<StageClaim>) -> StageDeliverable {
        StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims,
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings,
            required_checks_done: vec![],
        }
    }

    fn parse(json: &str) -> GateRule {
        serde_json::from_str(json).expect("parse")
    }

    #[test]
    fn count_at_least_passes_when_enough() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"kind","value":"subdomain"},
                 "min":1,"on_fail":{"reason":"need subdomain"} }"#,
        );
        let d = deliverable(vec![finding("subdomain", FindingSeverity::Info, vec![1])], vec![]);
        assert!(eval(&d, &[rule])[0].is_pass());
    }

    #[test]
    fn count_at_least_blocks_when_short() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"kind","value":"subdomain"},
                 "min":1,"on_fail":{"reason":"need subdomain"} }"#,
        );
        let d = deliverable(vec![finding("http_service", FindingSeverity::Info, vec![1])], vec![]);
        match &eval(&d, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => assert_eq!(reasons[0], "need subdomain"),
            GateCheckOutcome::Pass => panic!("expected Block"),
        }
    }

    #[test]
    fn for_all_high_findings_need_evidence() {
        let rule = parse(
            r#"{ "op":"for_all","over":"findings",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"high+ needs evidence","missing_evidence_kinds":["poc"]} }"#,
        );
        // critical without evidence -> Block; low without evidence -> ignored.
        let blocked = deliverable(
            vec![finding("rce", FindingSeverity::Critical, vec![])],
            vec![],
        );
        match &eval(&blocked, std::slice::from_ref(&rule))[0] {
            GateCheckOutcome::Block { recovery, .. } => {
                assert!(recovery.missing_evidence_kinds.contains(&"poc".to_string()))
            }
            GateCheckOutcome::Pass => panic!("expected Block"),
        }
        let ok = deliverable(vec![finding("info", FindingSeverity::Low, vec![])], vec![]);
        assert!(eval(&ok, &[rule])[0].is_pass());
    }

    #[test]
    fn for_all_empty_collection_is_vacuously_true() {
        let rule = parse(
            r#"{ "op":"for_all","over":"findings",
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"x"} }"#,
        );
        assert!(eval(&deliverable(vec![], vec![]), &[rule])[0].is_pass());
    }

    #[test]
    fn field_not_valid_for_collection_blocks_as_config_error() {
        // severity_at_least over claims -> claims have no severity -> config error Block.
        let rule = parse(
            r#"{ "op":"count_at_least","over":"claims",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "min":1,"on_fail":{"reason":"x"} }"#,
        );
        let d = deliverable(
            vec![],
            vec![StageClaim {
                kind: "k".into(),
                subject: "s".into(),
                summary: "sm".into(),
                evidence_ids: vec![EvidenceAuditId::new(1)],
            }],
        );
        match &eval(&d, &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => {
                assert!(reasons[0].contains("config error"))
            }
            GateCheckOutcome::Pass => panic!("expected config-error Block"),
        }
    }
```

**步骤 2.2** 跑确认红：

```bash
just test-harness
```
预期：编译失败，错误指向未定义的 `eval`（红）。

**步骤 2.3** 在 `OnFail` 定义之后、`#[cfg(test)]` 之前实现 `eval` 及私有 helper：

```rust
/// 纯函数：逐条规则求值，每条产出一个 outcome。无 IO / 无 DB / 确定性。
pub fn eval(deliverable: &StageDeliverable, rules: &[GateRule]) -> Vec<GateCheckOutcome> {
    rules.iter().map(|r| eval_one(deliverable, r)).collect()
}

fn eval_one(d: &StageDeliverable, rule: &GateRule) -> GateCheckOutcome {
    match rule {
        GateRule::CountAtLeast { over, filter, min, on_fail } => {
            match count_matching(d, *over, filter.as_ref()) {
                Ok(n) if n >= *min as usize => GateCheckOutcome::Pass,
                Ok(_) => block_from(on_fail),
                Err(e) => block_config_err(e),
            }
        }
        GateRule::ForAll { over, filter, require, on_fail } => {
            match for_all_matching(d, *over, filter.as_ref(), require) {
                Ok(true) => GateCheckOutcome::Pass,
                Ok(false) => block_from(on_fail),
                Err(e) => block_config_err(e),
            }
        }
    }
}

enum ItemRef<'a> {
    Claim(&'a StageClaim),
    Finding(&'a HarnessFinding),
}

impl ItemRef<'_> {
    fn kind_name(&self) -> &'static str {
        match self {
            ItemRef::Claim(_) => "claim",
            ItemRef::Finding(_) => "finding",
        }
    }
}

enum FieldVal<'a> {
    Text(&'a str),
    List(usize),
    Sev(FindingSeverity),
}

fn items<'a>(d: &'a StageDeliverable, c: Collection) -> Vec<ItemRef<'a>> {
    match c {
        Collection::Claims => d.claims.iter().map(ItemRef::Claim).collect(),
        Collection::Findings => d.findings.iter().map(ItemRef::Finding).collect(),
    }
}

fn count_matching(
    d: &StageDeliverable,
    c: Collection,
    filter: Option<&Pred>,
) -> Result<usize, String> {
    let mut n = 0;
    for it in &items(d, c) {
        let keep = match filter {
            Some(p) => pred_holds(it, p)?,
            None => true,
        };
        if keep {
            n += 1;
        }
    }
    Ok(n)
}

fn for_all_matching(
    d: &StageDeliverable,
    c: Collection,
    filter: Option<&Pred>,
    require: &Pred,
) -> Result<bool, String> {
    for it in &items(d, c) {
        let in_scope = match filter {
            Some(p) => pred_holds(it, p)?,
            None => true,
        };
        if in_scope && !pred_holds(it, require)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn pred_holds(item: &ItemRef, p: &Pred) -> Result<bool, String> {
    match p {
        Pred::NonEmpty { field } => match resolve(item, *field)? {
            FieldVal::Text(s) => Ok(!s.trim().is_empty()),
            FieldVal::List(len) => Ok(len > 0),
            FieldVal::Sev(_) => Err("non_empty not applicable to severity field".to_string()),
        },
        Pred::Eq { field, value } => match resolve(item, *field)? {
            FieldVal::Text(s) => Ok(s == value),
            FieldVal::Sev(sev) => Ok(sev_to_str(sev) == value),
            FieldVal::List(_) => Err(format!("eq not applicable to list field {field:?}")),
        },
        Pred::SeverityAtLeast { min } => match resolve(item, ItemField::Severity)? {
            FieldVal::Sev(sev) => Ok(sev.rank() >= min.rank()),
            _ => Err("severity_at_least requires a severity field".to_string()),
        },
    }
}

fn resolve<'a>(item: &ItemRef<'a>, f: ItemField) -> Result<FieldVal<'a>, String> {
    match (item, f) {
        (ItemRef::Claim(c), ItemField::Kind) => Ok(FieldVal::Text(&c.kind)),
        (ItemRef::Claim(c), ItemField::Subject) => Ok(FieldVal::Text(&c.subject)),
        (ItemRef::Claim(c), ItemField::Summary) => Ok(FieldVal::Text(&c.summary)),
        (ItemRef::Claim(c), ItemField::EvidenceIds) => Ok(FieldVal::List(c.evidence_ids.len())),
        (ItemRef::Finding(f), ItemField::Kind) => Ok(FieldVal::Text(&f.kind)),
        (ItemRef::Finding(f), ItemField::Subject) => Ok(FieldVal::Text(&f.subject)),
        (ItemRef::Finding(f), ItemField::EvidenceRefs) => Ok(FieldVal::List(f.evidence_refs.len())),
        (ItemRef::Finding(f), ItemField::Severity) => Ok(FieldVal::Sev(f.severity)),
        (item, field) => Err(format!(
            "field {:?} not valid for {} item",
            field,
            item.kind_name()
        )),
    }
}

fn sev_to_str(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Info => "info",
        FindingSeverity::Low => "low",
        FindingSeverity::Medium => "medium",
        FindingSeverity::High => "high",
        FindingSeverity::Critical => "critical",
    }
}

fn block_from(on_fail: &OnFail) -> GateCheckOutcome {
    GateCheckOutcome::Block {
        reasons: vec![on_fail.reason.clone()],
        recovery: HarnessRecoveryActions {
            hints: on_fail.hints.clone(),
            repair_tool_calls: on_fail.repair_tool_calls.clone(),
            missing_evidence_kinds: on_fail.missing_evidence_kinds.clone(),
        },
    }
}

fn block_config_err(msg: String) -> GateCheckOutcome {
    GateCheckOutcome::Block {
        reasons: vec![format!("gate_rule config error: {msg}")],
        recovery: HarnessRecoveryActions::default(),
    }
}
```

**步骤 2.4** 跑确认绿：

```bash
just test-harness
```
预期：Task 1 + Task 2 全部 `rule_engine::tests::*` PASS。

**步骤 2.5** clippy 清零：

```bash
cd backend && cargo clippy -p golish-agent-kit --all-targets -- -D warnings; cd ..
```
预期：零 warning。

**步骤 2.6** Commit：

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs
git commit -m "feat(harness): implement gate_rules eval (count_at_least / for_all / preds)"
```

---

## Task 3 · `StageSpec` 加 `gate_rules` 字段 + 修测试 helper

**文件：** `stage_spec.rs`、`finding_verification_check.rs`

**步骤 3.1** 在 `stage_spec.rs` 的 `StageSpec` 结构体里、`min_claims` 字段之后加：

```rust
    /// P2 · 数据驱动 gate 规则（设计 2026-06-05）。每条规则用固定积木 op 声明一条
    /// 过关标准，由 `gate::rule_engine::eval` 执行。缺省空 = 行为与旧版逐字节一致。
    #[serde(default)]
    pub gate_rules: Vec<super::gate::rule_engine::GateRule>,
```

**步骤 3.2** 在 `finding_verification_check.rs::spec_with` 的字段字面里、`min_claims: None,` 之后加一行（否则结构体字面缺字段编译失败）：

```rust
            min_claims: None,
            gate_rules: vec![],
```

**步骤 3.3** 在 `stage_spec.rs` 的 `#[cfg(test)] mod tests` 里加一个解析测试，确认空缺省 + 能解析内联规则：

```rust
    #[test]
    fn gate_rules_default_empty_and_parses() {
        let s = load_stage_spec_from_json(EXTERNAL_ATTACK_SURFACE_JSON).expect("parse");
        assert!(s.gate_rules.is_empty());

        let with_rules = r#"{
            "id":"verification","kind":"verification","risk_level":"critical",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "gate_rules":[
              { "op":"for_all","over":"findings",
                "where":{"pred":"severity_at_least","min":"high"},
                "require":{"pred":"non_empty","field":"evidence_refs"},
                "on_fail":{"reason":"high+ finding needs evidence"} }
            ]
        }"#;
        let s2 = load_stage_spec_from_json(with_rules).expect("parse with rules");
        assert_eq!(s2.gate_rules.len(), 1);
    }
```

**步骤 3.4** 跑：

```bash
just test-harness
```
预期：编译通过；`gate_rules_default_empty_and_parses` PASS；`resources::tests::all_twelve_stage_specs_load` 仍 PASS（证明 12 个内嵌 spec 不受影响）。

**步骤 3.5** Commit：

```bash
git add backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs
git commit -m "feat(harness): add StageSpec.gate_rules field (serde default empty)"
```

---

## Task 4 · 调度入口并进 `eval` + 集成测试（TDD）

**文件：** `gate/mod.rs`

**步骤 4.1** 在 `mod.rs` 的 `#[cfg(test)] mod tests` 里加失败测试（先引用尚未接线的行为）：

```rust
    #[test]
    fn gate_rules_block_propagates_through_aggregate() {
        use super::super::stage_spec::load_stage_spec_from_json;
        use super::super::types::{FindingSeverity, HarnessFinding, StageClaim};
        use golish_pentest::evidence_ledger::EvidenceAuditId;

        // spec：一条 gate_rule 要求每个 high+ finding 挂证据。其余字段最小化。
        let spec_json = r#"{
            "id":"verification","kind":"verification","risk_level":"critical",
            "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
            "gate_rules":[
              { "op":"for_all","over":"findings",
                "where":{"pred":"severity_at_least","min":"high"},
                "require":{"pred":"non_empty","field":"evidence_refs"},
                "on_fail":{"reason":"GATE_RULE: high+ finding needs evidence"} }
            ]
        }"#;
        let spec = load_stage_spec_from_json(spec_json).unwrap();

        // 一个能过基础结构 check、但违反 gate_rule 的 deliverable：
        // 有 1 个 critical finding 且它自身挂了证据（过 scope/freshness），
        // 但我们再加一个 critical finding 不挂证据来触发 gate_rule。
        let eid = EvidenceAuditId::new(1);
        let deliverable = StageDeliverable {
            stage_id: "verification".to_string(),
            stage_run_id: Uuid::new_v4(),
            claims: vec![StageClaim {
                kind: "exploit".to_string(),
                subject: "api.example.com".to_string(),
                summary: "verified".to_string(),
                evidence_ids: vec![eid],
            }],
            evidence_refs: vec![eid],
            skipped_checks: vec![],
            findings: vec![HarnessFinding {
                finding_id: Uuid::new_v4(),
                kind: "rce".to_string(),
                subject: "api.example.com".to_string(),
                severity: FindingSeverity::Critical,
                evidence_refs: vec![eid],
            }],
            required_checks_done: vec![],
        };

        // 基线（无 gate_rule 违反）应 pass：
        let base = validate_stage_gate(&deliverable, &spec, None);
        assert!(base.allowed, "baseline should pass: {:?}", base.reasons);

        // 追加一个不挂证据的 critical finding -> gate_rule 触发 -> 整体 Block。
        let mut bad = deliverable.clone();
        bad.findings.push(HarnessFinding {
            finding_id: Uuid::new_v4(),
            kind: "rce".to_string(),
            subject: "db.example.com".to_string(),
            severity: FindingSeverity::Critical,
            evidence_refs: vec![],
        });
        let blocked = validate_stage_gate(&bad, &spec, None);
        assert!(!blocked.allowed);
        assert!(blocked.reasons.iter().any(|r| r.contains("GATE_RULE")));
    }
```

> 注：`StageDeliverable` 已 `#[derive(Clone)]`（types.rs），`bad = deliverable.clone()` 可用。

**步骤 4.2** 跑确认红（应 Block 但当前未接线，断言 `contains("GATE_RULE")` 失败）：

```bash
just test-harness
```

**步骤 4.3** 在 `validate_stage_gate_with_skeleton` 里，`required_checks` 的 `for` 循环之后、`aggregate(outcomes)` 之前加：

```rust
    outcomes.extend(rule_engine::eval(deliverable, &spec.gate_rules));

    aggregate(outcomes)
```

**步骤 4.4** 跑确认绿：

```bash
just test-harness
```
预期：`gate_rules_block_propagates_through_aggregate` PASS；既有 gate 测试全绿。

**步骤 4.5** Commit：

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/mod.rs
git commit -m "feat(harness): wire gate_rules eval into validate_stage_gate aggregate"
```

---

## Task 5 · 等价性测试 + 给 `verification.json` 补样例规则

**文件：** `finding_verification_check.rs`（等价性测试）、`resources/harness/stages/verification.json`

**步骤 5.1** 在 `finding_verification_check.rs::tests` 加等价性测试，断言"同一 deliverable 下，`finding_verification` 现状判定 与 等价 `gate_rules` 判定一致"：

```rust
    #[test]
    fn gate_rule_reproduces_finding_verification_block() {
        use crate::harness::gate::rule_engine::{self, GateRule};

        // 现状路径：finding_verification 规则 min_severity=high。
        let rule = FindingVerificationRule {
            min_severity: FindingSeverity::High,
            require_evidence_kinds: vec![],
        };
        let d = deliverable(vec![finding(FindingSeverity::Critical, vec![])]);
        let legacy = run(&d, &spec_with(Some(rule)));

        // 等价 gate_rule：for_all findings where severity>=high require non_empty evidence_refs。
        let gr: GateRule = serde_json::from_str(
            r#"{ "op":"for_all","over":"findings",
                 "where":{"pred":"severity_at_least","min":"high"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"high+ finding needs evidence"} }"#,
        )
        .unwrap();
        let engine = &rule_engine::eval(&d, &[gr])[0];

        // 两者都 Block。
        assert!(!legacy.is_pass());
        assert!(!engine.is_pass());
    }
```

**步骤 5.2** 跑：

```bash
just test-harness
```
预期：`gate_rule_reproduces_finding_verification_block` PASS。

**步骤 5.3** 给 `resources/harness/stages/verification.json` 末尾（`finding_verification` 块之后、最后的 `}` 之前）加一条样例 `gate_rules`，作为"声明式表达同一意图"的活样例（与现有 `finding_verification` 并存，不冲突）：

```json
  "finding_verification": {
    "min_severity": "high",
    "require_evidence_kinds": ["poc", "exploit_verified"]
  },

  "$comment_gate_rules": "DSL sample (design 2026-06-05). Declarative twin of finding_verification: every high+ finding must carry at least one evidence ref. Add more rules here to extend pass-criteria with zero Rust.",
  "gate_rules": [
    {
      "op": "for_all",
      "over": "findings",
      "where": { "pred": "severity_at_least", "min": "high" },
      "require": { "pred": "non_empty", "field": "evidence_refs" },
      "on_fail": { "reason": "verification: every high/critical finding must carry evidence" }
    }
  ]
```

> 务必处理好 JSON 逗号：`finding_verification` 对象后要加逗号，再接 `$comment_gate_rules` 与 `gate_rules`。

**步骤 5.4** 跑确认 12 spec 仍全部加载（这步会真正反序列化 verification.json 的新 `gate_rules`）：

```bash
just test-harness
```
预期：`resources::tests::all_twelve_stage_specs_load` PASS（证明样例 JSON 合法、能反序列化为 `Vec<GateRule>`）。

**步骤 5.5** Commit：

```bash
git add backend/crates/golish-agent-kit/src/harness/gate/finding_verification_check.rs resources/harness/stages/verification.json
git commit -m "test(harness): gate_rule reproduces finding_verification; sample rule in verification.json"
```

---

## Task 6 · 文档：DSL 速查表

**文件：** `docs/design/2026-06-02-harness-stage-spec-reference.md`

**步骤 6.1** 在该文件末尾追加一节：

```markdown
## gate_rules DSL 速查（2026-06-05）

声明式过关标准，缺省空数组（不写则行为不变）。引擎：`gate/rule_engine.rs::eval`。

**顶层 op**
- `count_at_least`：`{ op, over, where?, min, on_fail }` — 满足 where 的元素 ≥ min。
- `for_all`：`{ op, over, where?, require, on_fail }` — 满足 where 的每个元素都满足 require（空集合为真）。

**over**：`claims` | `findings`
**pred**（用于 where / require）：
- `{ "pred":"non_empty","field":<f> }`
- `{ "pred":"eq","field":<f>,"value":"<s>" }`
- `{ "pred":"severity_at_least","min":"info|low|medium|high|critical" }`（仅 findings）

**field**：`kind` | `subject` | `summary` | `evidence_refs`(finding) | `evidence_ids`(claim) | `severity`(finding)
**on_fail**：`{ reason, hints?, repair_tool_calls?, missing_evidence_kinds? }`

fail-closed：未知 op/pred/over/field 或字段-集合不匹配 → spec 加载报错或规则求值 Block，绝不静默忽略。

完整设计见 `docs/design/2026-06-05-gate-rule-engine.md`。
```

**步骤 6.2** Commit：

```bash
git add docs/design/2026-06-02-harness-stage-spec-reference.md
git commit -m "docs(harness): gate_rules DSL quick reference"
```

---

## Task 7 · 收口验证 + 进度登记

**步骤 7.1** 全量门禁：

```bash
just precommit
```
预期：`fmt + check-fe + test-fe + lint-rust + test-rust-all` 全绿。若 `lint-rust`（clippy）报 `useless_format` 等，按提示修后重跑。

**步骤 7.2** 把命令与关键输出（含退出码）复制进 `agent-progress.md` 的「已记录证据」段；在 `feature_list.json` 加/更新对应条目状态（`passing` 并填 `evidence`，或 `in_progress`）。

**步骤 7.3** Commit（仅 harness 文档/状态文件）：

```bash
git add agent-progress.md feature_list.json
git commit -m "chore(harness): record gate_rules engine evidence + feature status"
```

---

## 自检

**规格覆盖度**（对照设计 `2026-06-05-gate-rule-engine.md`）：
- §5 DSL（GateRule/Pred/Collection/ItemField/OnFail/eval）→ Task 1+2 ✅
- §5.4 fail-closed typed enum → Task 1（unknown_op/pred 测试）+ Task 3（12 spec 加载）✅
- §5.5 接入 validate_stage_gate → Task 4 ✅
- §6 复刻现有标准 → Task 5 等价性测试 ✅
- §7.4 verification.json 样例 → Task 5 ✅
- §7.5 文档 → Task 6 ✅
- §10 验证计划 → Task 4/5 单测 + Task 7 precommit ✅

**占位符扫描**：无 TODO/待定；每步含真实代码与精确命令。

**类型一致性**：`GateRule`/`Pred`/`Collection`/`ItemField`/`OnFail`/`eval`/`FieldVal`/`ItemRef` 在 Task 1-2 定义并贯穿；`spec_with` 新字段名 `gate_rules` 与 Task 3 字段名一致；测试 helper `finding`/`deliverable`/`parse` 在各任务内自洽（Task 2 与 Task 5 各自定义在其所属测试模块，无跨文件复用冲突）。

**YAGNI 复核**：MVP 只 2 op + 3 谓词 + 2 集合；and/or、evidence KIND、freshness、evidence_refs/skipped_checks 集合均明确留作未来积木，不在本计划。
