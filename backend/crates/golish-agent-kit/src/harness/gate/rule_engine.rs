//! 数据驱动 gate 规则引擎（设计 `docs/design/2026-06-05-gate-rule-engine.md`）。
//!
//! stage JSON 用固定积木 op 声明过关标准；本模块是纯函数、DB-free、确定性解释器，
//! 输出复用 `GateCheckOutcome` / `HarnessRecoveryActions`，由 `gate/mod.rs` 并进聚合。
//! fail-closed：op / pred / over / field 全是 serde enum，写错的名字在 `StageSpec`
//! 反序列化期即报错（被 `resources` 的 `all_twelve_stage_specs_load` 单测抓住）；
//! 字段与集合不匹配（如对 claims 取 severity）在求值期返回 config-error Block。

use serde::{Deserialize, Serialize};

use super::super::stage_spec::StageSpec;
use super::super::types::{
    CoverageCell, CoverageStatus, FindingSeverity, HarnessFinding, HarnessRecoveryActions,
    StageClaim, StageDeliverable,
};
use super::{min_invocations_check, scope_check, surface_coverage_check, GateCheckOutcome};

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
    /// 逃生舱（设计 2026-06-05-gate-rules-migration §5.1）：按名调用保留下来的
    /// Rust 领域 check。仅 3 个固定值（非通用扩展点）；某 check 被数据化后从枚举删除。
    /// `on_fail=Some` 时，仅当被调 check 返回 Block 才用它覆盖 reason/recovery。
    NamedCheck {
        check: NamedCheckKind,
        #[serde(default)]
        on_fail: Option<OnFail>,
    },
}

/// `named_check` 逃生舱可调的内建 check（闭合枚举 → 写错名 serde 报错 fail-closed）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamedCheckKind {
    Scope,
    SurfaceCoverage,
    MinInvocations,
}

impl NamedCheckKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scope => "scope",
            Self::SurfaceCoverage => "surface_coverage",
            Self::MinInvocations => "min_invocations",
        }
    }
}

impl GateRule {
    /// agent 面向的 stage charter 用的简短描述（替代旧 `required_checks` 名字列表）。
    pub fn summary(&self) -> String {
        match self {
            GateRule::CountAtLeast { on_fail, .. } | GateRule::ForAll { on_fail, .. } => {
                on_fail.reason.clone()
            }
            GateRule::NamedCheck { check, on_fail } => on_fail
                .as_ref()
                .map(|o| o.reason.clone())
                .unwrap_or_else(|| format!("{} check", check.as_str())),
        }
    }
}

/// MVP 只含有可寻址字段的两个集合（evidence_refs / skipped_checks 的计数已被
/// `vacuous_check` 覆盖，故不纳入 MVP）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Claims,
    Findings,
    /// Coverage matrix 单元格（设计 2026-06-05-coverage-matrix）。
    Coverage,
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
    /// coverage cell 字段（设计 2026-06-05-coverage-matrix）。
    Asset,
    Technique,
    Status,
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

/// 逐条规则求值，每条产出一个 outcome。数据 op（count_at_least/for_all）是纯函数、
/// DB-free；`named_check` op 转发到保留的 Rust 领域 check（同样只读 deliverable(+spec
/// 配置)，无 IO）。`spec` 供 `named_check:min_invocations` 读 `spec.min_invocations`。
pub fn eval(
    deliverable: &StageDeliverable,
    spec: &StageSpec,
    rules: &[GateRule],
) -> Vec<GateCheckOutcome> {
    rules
        .iter()
        .map(|r| eval_one(deliverable, spec, r))
        .collect()
}

fn eval_one(d: &StageDeliverable, spec: &StageSpec, rule: &GateRule) -> GateCheckOutcome {
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
        GateRule::NamedCheck { check, on_fail } => {
            let base = match check {
                NamedCheckKind::Scope => scope_check::run(d),
                NamedCheckKind::SurfaceCoverage => surface_coverage_check::run(d),
                NamedCheckKind::MinInvocations => min_invocations_check::run(d, spec),
            };
            match (&base, on_fail) {
                // 仅在该 check 真的 Block 且作者提供了 on_fail 时覆盖其 reason/recovery；
                // 否则原样沿用 check 自身的结论与 recovery。
                (GateCheckOutcome::Block { .. }, Some(of)) => block_from(of),
                _ => base,
            }
        }
    }
}

enum ItemRef<'a> {
    Claim(&'a StageClaim),
    Finding(&'a HarnessFinding),
    Coverage(&'a CoverageCell),
}

impl ItemRef<'_> {
    fn kind_name(&self) -> &'static str {
        match self {
            ItemRef::Claim(_) => "claim",
            ItemRef::Finding(_) => "finding",
            ItemRef::Coverage(_) => "coverage cell",
        }
    }
}

enum FieldVal<'a> {
    Text(&'a str),
    List(usize),
    Sev(FindingSeverity),
    Status(CoverageStatus),
}

fn items<'a>(d: &'a StageDeliverable, c: Collection) -> Vec<ItemRef<'a>> {
    match c {
        Collection::Claims => d.claims.iter().map(ItemRef::Claim).collect(),
        Collection::Findings => d.findings.iter().map(ItemRef::Finding).collect(),
        Collection::Coverage => d.coverage.iter().map(ItemRef::Coverage).collect(),
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
            FieldVal::Status(_) => Err("non_empty not applicable to status field".to_string()),
        },
        Pred::Eq { field, value } => match resolve(item, *field)? {
            FieldVal::Text(s) => Ok(s == value),
            FieldVal::Sev(sev) => Ok(sev_to_str(sev) == value),
            FieldVal::Status(st) => Ok(status_to_str(st) == value),
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
        (ItemRef::Coverage(c), ItemField::Asset) => Ok(FieldVal::Text(&c.asset)),
        (ItemRef::Coverage(c), ItemField::Technique) => Ok(FieldVal::Text(&c.technique)),
        (ItemRef::Coverage(c), ItemField::Status) => Ok(FieldVal::Status(c.status)),
        (ItemRef::Coverage(c), ItemField::EvidenceRefs) => {
            Ok(FieldVal::List(c.evidence_refs.len()))
        }
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

fn status_to_str(s: CoverageStatus) -> &'static str {
    match s {
        CoverageStatus::Found => "found",
        CoverageStatus::CheckedEmpty => "checked_empty",
        CoverageStatus::Blocked => "blocked",
        CoverageStatus::NotApplicable => "not_applicable",
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
            coverage: vec![],
        }
    }

    fn parse(json: &str) -> GateRule {
        serde_json::from_str(json).expect("parse")
    }

    /// Minimal spec for data-op tests (they ignore `spec`); named_check tests
    /// that need `spec.min_invocations` build their own.
    fn test_spec() -> StageSpec {
        crate::harness::stage_spec::load_stage_spec_from_json(
            r#"{"id":"verification","kind":"verification","risk_level":"critical",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate"}"#,
        )
        .expect("minimal spec parses")
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
        assert!(eval(&d, &test_spec(), &[rule])[0].is_pass());
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
        match &eval(&d, &test_spec(), &[rule])[0] {
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
        match &eval(&blocked, &test_spec(), std::slice::from_ref(&rule))[0] {
            GateCheckOutcome::Block { recovery, .. } => {
                assert!(recovery.missing_evidence_kinds.contains(&"poc".to_string()))
            }
            GateCheckOutcome::Pass => panic!("expected Block"),
        }
        let ok = deliverable(vec![finding("info", FindingSeverity::Low, vec![])], vec![]);
        assert!(eval(&ok, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn for_all_empty_collection_is_vacuously_true() {
        let rule = parse(
            r#"{ "op":"for_all","over":"findings",
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"x"} }"#,
        );
        assert!(eval(&deliverable(vec![], vec![]), &test_spec(), &[rule])[0].is_pass());
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
        match &eval(&d, &test_spec(), &[rule])[0] {
            GateCheckOutcome::Block { reasons, .. } => assert!(reasons[0].contains("config error")),
            GateCheckOutcome::Pass => panic!("expected config-error Block"),
        }
    }

    // ── named_check 逃生舱（设计 2026-06-05-gate-rules-migration） ─────────────

    #[test]
    fn parses_named_check_rule() {
        let rule: GateRule =
            serde_json::from_str(r#"{ "op":"named_check","check":"surface_coverage" }"#)
                .expect("parse");
        assert!(matches!(
            rule,
            GateRule::NamedCheck {
                check: NamedCheckKind::SurfaceCoverage,
                ..
            }
        ));
    }

    #[test]
    fn unknown_named_check_fails_closed() {
        assert!(
            serde_json::from_str::<GateRule>(r#"{ "op":"named_check","check":"bogus" }"#).is_err()
        );
    }

    #[test]
    fn named_check_scope_blocks_on_claim_without_evidence() {
        // 经 named_check 转发到 scope_check：claim 缺 evidence_ids → Block。
        let rule = parse(r#"{ "op":"named_check","check":"scope" }"#);
        let d = deliverable(
            vec![],
            vec![StageClaim {
                kind: "http_service".into(),
                subject: "api.example.com".into(),
                summary: "200".into(),
                evidence_ids: vec![],
            }],
        );
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn named_check_surface_coverage_blocks_on_missing_jsapi() {
        // 经 named_check 转发到 surface_coverage_check：只有 Surface 无 JsApi → Block。
        let rule = parse(r#"{ "op":"named_check","check":"surface_coverage" }"#);
        let d = deliverable(
            vec![finding("http_service", FindingSeverity::Info, vec![1])],
            vec![],
        );
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn named_check_min_invocations_uses_spec_and_blocks_when_tool_absent() {
        // 经 named_check 转发到 min_invocations_check，读 spec.min_invocations；
        // required_checks_done 为空 → Block。
        let spec = crate::harness::stage_spec::load_stage_spec_from_json(
            r#"{"id":"enumeration","kind":"enumeration","risk_level":"medium",
                "deliverable_schema":"StageDeliverable","gate_validator":"validate_stage_gate",
                "min_invocations":{"dns_resolve":1}}"#,
        )
        .unwrap();
        let rule = parse(r#"{ "op":"named_check","check":"min_invocations" }"#);
        let d = deliverable(vec![], vec![]);
        assert!(!eval(&d, &spec, &[rule])[0].is_pass());
    }

    // ── coverage matrix 数据积木（设计 2026-06-05-coverage-matrix） ────────────

    fn cov_cell(
        asset: &str,
        technique: &str,
        status: CoverageStatus,
        refs: Vec<i64>,
    ) -> CoverageCell {
        CoverageCell {
            asset: asset.to_string(),
            technique: technique.to_string(),
            status,
            evidence_refs: refs.into_iter().map(EvidenceAuditId::new).collect(),
            note: None,
        }
    }

    fn deliverable_with_coverage(cells: Vec<CoverageCell>) -> StageDeliverable {
        let mut d = deliverable(vec![], vec![]);
        d.coverage = cells;
        d
    }

    #[test]
    fn coverage_found_cell_must_cite_evidence() {
        // #4：for_all over coverage where status==found require non_empty evidence_refs。
        let rule = parse(
            r#"{ "op":"for_all","over":"coverage",
                 "where":{"pred":"eq","field":"status","value":"found"},
                 "require":{"pred":"non_empty","field":"evidence_refs"},
                 "on_fail":{"reason":"found cell needs evidence"} }"#,
        );
        // found 缺证据 → Block。
        let bad = deliverable_with_coverage(vec![cov_cell(
            "api.ex.com",
            "idor",
            CoverageStatus::Found,
            vec![],
        )]);
        assert!(!eval(&bad, &test_spec(), std::slice::from_ref(&rule))[0].is_pass());
        // found 有证据 + checked_empty 缺证据（被 where 过滤掉）→ Pass。
        let ok = deliverable_with_coverage(vec![
            cov_cell("api.ex.com", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("api.ex.com", "xss", CoverageStatus::CheckedEmpty, vec![]),
        ]);
        assert!(eval(&ok, &test_spec(), &[rule])[0].is_pass());
    }

    #[test]
    fn coverage_count_found_status() {
        // count_at_least over coverage where status==found min 2。
        let rule = parse(
            r#"{ "op":"count_at_least","over":"coverage",
                 "where":{"pred":"eq","field":"status","value":"found"},
                 "min":2,"on_fail":{"reason":"need >=2 found"} }"#,
        );
        let d = deliverable_with_coverage(vec![
            cov_cell("a", "idor", CoverageStatus::Found, vec![1]),
            cov_cell("a", "xss", CoverageStatus::CheckedEmpty, vec![1]),
        ]);
        // 只有 1 个 found → Block。
        assert!(!eval(&d, &test_spec(), &[rule])[0].is_pass());
    }
}
