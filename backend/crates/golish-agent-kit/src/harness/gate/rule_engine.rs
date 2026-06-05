//! 数据驱动 gate 规则引擎（设计 `docs/design/2026-06-05-gate-rule-engine.md`）。
//!
//! stage JSON 用固定积木 op 声明过关标准；本模块是纯函数、DB-free、确定性解释器，
//! 输出复用 `GateCheckOutcome` / `HarnessRecoveryActions`，由 `gate/mod.rs` 并进聚合。
//! fail-closed：op / pred / over / field 全是 serde enum，写错的名字在 `StageSpec`
//! 反序列化期即报错（被 `resources` 的 `all_twelve_stage_specs_load` 单测抓住）；
//! 字段与集合不匹配（如对 claims 取 severity）在求值期返回 config-error Block。

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

/// MVP 只含有可寻址字段的两个集合（evidence_refs / skipped_checks 的计数已被
/// `vacuous_check` 覆盖，故不纳入 MVP）。
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
    /// 数组 / 字符串字段非空。
    NonEmpty { field: ItemField },
    /// 字符串字段等于 `value`（severity 按 snake_case 文本比较）。
    Eq { field: ItemField, value: String },
    /// finding.severity rank ≥ min。
    SeverityAtLeast { min: FindingSeverity },
}

/// 可寻址的元素字段（按集合解析；不适用的组合在求值期 fail-closed）。
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

/// 规则不满足时映射到 `GateCheckOutcome::Block` 的内容。
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

/// 纯函数：逐条规则求值，每条产出一个 outcome。无 IO / 无 DB / 确定性。
pub fn eval(deliverable: &StageDeliverable, rules: &[GateRule]) -> Vec<GateCheckOutcome> {
    rules.iter().map(|r| eval_one(deliverable, r)).collect()
}

fn eval_one(d: &StageDeliverable, rule: &GateRule) -> GateCheckOutcome {
    match rule {
        GateRule::CountAtLeast {
            over,
            filter,
            min,
            on_fail,
        } => match count_matching(d, *over, filter.as_ref()) {
            Ok(n) if n >= *min as usize => GateCheckOutcome::Pass,
            Ok(_) => block_from(on_fail),
            Err(e) => block_config_err(e),
        },
        GateRule::ForAll {
            over,
            filter,
            require,
            on_fail,
        } => match for_all_matching(d, *over, filter.as_ref(), require) {
            Ok(true) => GateCheckOutcome::Pass,
            Ok(false) => block_from(on_fail),
            Err(e) => block_config_err(e),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use golish_pentest::evidence_ledger::EvidenceAuditId;
    use uuid::Uuid;

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
        let d = deliverable(
            vec![finding("subdomain", FindingSeverity::Info, vec![1])],
            vec![],
        );
        assert!(eval(&d, &[rule])[0].is_pass());
    }

    #[test]
    fn count_at_least_blocks_when_short() {
        let rule = parse(
            r#"{ "op":"count_at_least","over":"findings",
                 "where":{"pred":"eq","field":"kind","value":"subdomain"},
                 "min":1,"on_fail":{"reason":"need subdomain"} }"#,
        );
        let d = deliverable(
            vec![finding("http_service", FindingSeverity::Info, vec![1])],
            vec![],
        );
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
            GateCheckOutcome::Block { reasons, .. } => assert!(reasons[0].contains("config error")),
            GateCheckOutcome::Pass => panic!("expected config-error Block"),
        }
    }
}
