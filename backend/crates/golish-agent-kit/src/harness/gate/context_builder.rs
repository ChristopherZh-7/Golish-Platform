//! Single chokepoint that assembles a [`GateContext`] from already-fetched
//! inputs.
//!
//! 三个 gate 入口（主 agent stage-close `execute.rs`、per-org fan-out
//! `org_gate.rs`、submit 预检 `harness_submit_tool.rs`）此前各自手搓
//! `GateContext { .. }`：`(!x.is_empty()).then_some(x)` 归一、`typed →
//! HashMap` 转换、evidence-facts 合并散落 3 处且语义漂移（submit 预检甚至漏掉
//! `asset_types` / `expected_techniques`）。本 builder 把**纯组装**收成一处，
//! 让归一只在 [`GateContextBuilder::build`] 发生（单一真相源）。
//!
//! 设计 `docs/design/2026-06-23-unified-gate-context-builder.md`。仅做组装：repo
//! **查询**仍在各入口（跨 crate / freshness 与 subsidiary 语义有意不同），不在此。

use std::collections::{HashMap, HashSet};

use super::rule_engine::{EvidenceFact, GateContext, SourceQueryFact};
use crate::harness::handoff_catalog::CanonicalFactKey;

#[derive(Debug, Clone)]
pub struct GateContextWithCanonicalSources {
    pub context: GateContext,
    pub canonical_source_hints: Vec<CanonicalFactKey>,
}

/// 累加器式构造 [`GateContext`]。所有 setter 取所有权、可链式；普通空集合在
/// [`GateContextBuilder::build`] 统一折成 `None`。唯一例外是调用方明确通过
/// [`GateContextBuilder::authoritative_in_scope_assets`] 传入的 `Some([])`：它表示
/// 权威查询成功且分母确实为空，必须保留。
#[derive(Debug, Default, Clone)]
pub struct GateContextBuilder {
    in_scope_assets: Option<Vec<String>>,
    asset_types: HashMap<String, String>,
    web_capable_assets: HashSet<String>,
    not_applicable_coverage: HashSet<(String, String)>,
    eas_required_web_origins: Option<HashSet<String>>,
    eas_completed_web_origins: Option<HashSet<String>>,
    evidence_facts: Vec<EvidenceFact>,
    source_queries: Vec<SourceQueryFact>,
    expected_techniques: Option<Vec<String>>,
    candidate_work_item_keys: Option<Vec<String>>,
    verification_truth_required: bool,
    verification_truth_snapshots: Option<crate::harness::attack_execution::VerificationTruthSet>,
    reporting_truth: Option<crate::harness::ReportingGateTruth>,
    canonical_source_hints: Vec<CanonicalFactKey>,
}

impl GateContextBuilder {
    /// 空 builder（`build()` → `GateContext::default()`）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 权威 in-scope 资产集。空 ⇒ `build()` 折成 `None`（gate 回退自报）。
    /// 幂等覆盖（非追加）。
    pub fn in_scope_assets(mut self, assets: Vec<String>) -> Self {
        self.in_scope_assets = (!assets.is_empty()).then_some(assets);
        self
    }

    /// Authoritative asset axis from a successful coverage snapshot. Unlike
    /// [`Self::in_scope_assets`], `Some([])` is preserved: it means the DB proved
    /// that this org has no denominator, not that the caller failed to load it.
    pub fn authoritative_in_scope_assets(mut self, assets: Option<Vec<String>>) -> Self {
        self.in_scope_assets = assets;
        self
    }

    /// `(value, targets.type)` 列表，折成 `asset_types` map。空 ⇒ `None`。
    /// 覆盖既有 map。
    pub fn typed_assets(mut self, typed: Vec<(String, String)>) -> Self {
        self.asset_types = typed.into_iter().collect();
        self
    }

    /// 调用方已持有 `value -> targets.type` map 时直接喂入（覆盖）。空 ⇒ `None`。
    pub fn asset_types_map(mut self, map: HashMap<String, String>) -> Self {
        self.asset_types = map;
        self
    }

    /// EAS/httpx-proven IP/CIDR web targets for enumeration. Empty ⇒ `None`.
    pub fn web_capable_assets<I>(mut self, assets: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        self.web_capable_assets = assets.into_iter().collect();
        self
    }

    /// DB-derived terminal not_applicable cells, keyed by `(asset, technique)`.
    /// Empty ⇒ `None`.
    pub fn not_applicable_coverage<I>(mut self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        self.not_applicable_coverage = pairs.into_iter().collect();
        self
    }

    /// Activate the strict EAS exact-origin barrier. Empty sets are preserved:
    /// they mean the authoritative DB reads succeeded with an empty denominator,
    /// unlike `None` which means the caller did not provide this contract.
    pub fn eas_web_origin_barrier<I, J>(mut self, required: I, completed: J) -> Self
    where
        I: IntoIterator<Item = String>,
        J: IntoIterator<Item = String>,
    {
        self.eas_required_web_origins = Some(required.into_iter().collect());
        self.eas_completed_web_origins = Some(completed.into_iter().collect());
        self
    }

    /// 追加 evidence facts（ledger 投影 / DB 真值 / subsidiary 投影 …）。可多次
    /// 调用合并多个来源；顺序对 gate 无影响。
    pub fn extend_evidence_facts<I>(mut self, facts: I) -> Self
    where
        I: IntoIterator<Item = EvidenceFact>,
    {
        self.evidence_facts.extend(facts);
        self
    }

    /// 追加 source-query facts（source_query_log 读投影）。可多次调用；空集合在
    /// `build()` 归一成 None。
    pub fn extend_source_queries<I>(mut self, rows: I) -> Self
    where
        I: IntoIterator<Item = SourceQueryFact>,
    {
        self.source_queries.extend(rows);
        self
    }

    /// 动态期望技术。`None` ⇒ gate 回退 `spec.expected_techniques`（静态）。
    /// 直接透传（不做空集合归一——`Some(vec![])` 与 `None` 语义不同，交给 gate）。
    pub fn expected_techniques(mut self, techniques: Option<Vec<String>>) -> Self {
        self.expected_techniques = techniques;
        self
    }

    /// Exact Candidate V2 manifest axis loaded from the trusted WaveUnit. An
    /// explicit empty list is preserved and therefore fail-closed by the gate.
    pub fn candidate_work_item_keys(mut self, keys: Option<Vec<String>>) -> Self {
        self.candidate_work_item_keys = keys;
        self
    }

    /// Activate V2-only Verification truth. An explicit empty vector remains
    /// distinguishable from legacy mode and therefore blocks in the rule.
    pub fn verification_truth(
        mut self,
        truth: Option<crate::harness::attack_execution::VerificationTruthSet>,
    ) -> Self {
        self.verification_truth_required = true;
        self.verification_truth_snapshots = truth;
        self
    }

    /// Current DB-authoritative Reporting revision truth. `None` is preserved:
    /// the `report_revision_validated` rule treats a missing DB read as BLOCK.
    pub fn reporting_truth(mut self, truth: Option<crate::harness::ReportingGateTruth>) -> Self {
        self.reporting_truth = truth;
        self
    }

    /// Append server-derived keys that a final PASS may project into its
    /// handoff. These are hints only: the final-seal repository re-reads exact
    /// owner/timestamp/hash/evidence fields under locks.
    pub fn extend_canonical_source_hints<I>(mut self, hints: I) -> Self
    where
        I: IntoIterator<Item = CanonicalFactKey>,
    {
        self.canonical_source_hints.extend(hints);
        self
    }

    /// 组装 [`GateContext`]：普通集合的**唯一** `empty → None` 归一点；权威资产轴
    /// 已在 setter 中保留 `Some([])`。
    pub fn build(self) -> GateContext {
        self.build_with_canonical_source_hints().context
    }

    /// Build the ordinary pure Gate context together with the separately
    /// bounded final-seal source hints. Keeping them outside rule evaluation
    /// prevents model claims from becoming Gate truth.
    pub fn build_with_canonical_source_hints(self) -> GateContextWithCanonicalSources {
        let canonical_source_hints = self.canonical_source_hints;
        let context = GateContext {
            in_scope_assets: self.in_scope_assets,
            asset_types: (!self.asset_types.is_empty()).then_some(self.asset_types),
            web_capable_assets: (!self.web_capable_assets.is_empty())
                .then_some(self.web_capable_assets),
            not_applicable_coverage: (!self.not_applicable_coverage.is_empty())
                .then_some(self.not_applicable_coverage),
            eas_required_web_origins: self.eas_required_web_origins,
            eas_completed_web_origins: self.eas_completed_web_origins,
            expected_techniques: self.expected_techniques,
            candidate_work_item_keys: self.candidate_work_item_keys,
            verification_truth_required: self.verification_truth_required,
            verification_truth_snapshots: self.verification_truth_snapshots,
            reporting_truth: self.reporting_truth,
            evidence_facts: (!self.evidence_facts.is_empty()).then_some(self.evidence_facts),
            source_queries: (!self.source_queries.is_empty()).then_some(self.source_queries),
        };
        GateContextWithCanonicalSources {
            context,
            canonical_source_hints,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::gate::rule_engine::EvidenceOutcome;

    fn fact(asset: &str, technique: &str, outcome: EvidenceOutcome, id: i64) -> EvidenceFact {
        EvidenceFact {
            asset: asset.to_string(),
            technique: technique.to_string(),
            outcome,
            evidence_id: id,
        }
    }

    #[test]
    fn empty_builder_equals_default_context() {
        let ctx = GateContextBuilder::new().build();
        let def = GateContext::default();
        assert_eq!(ctx.in_scope_assets, def.in_scope_assets);
        assert_eq!(ctx.asset_types, def.asset_types);
        assert_eq!(ctx.web_capable_assets, def.web_capable_assets);
        assert_eq!(ctx.not_applicable_coverage, def.not_applicable_coverage);
        assert_eq!(ctx.eas_required_web_origins, def.eas_required_web_origins);
        assert_eq!(ctx.eas_completed_web_origins, def.eas_completed_web_origins);
        assert_eq!(ctx.reporting_truth, def.reporting_truth);
        assert_eq!(ctx.expected_techniques, def.expected_techniques);
        assert_eq!(ctx.candidate_work_item_keys, def.candidate_work_item_keys);
        assert!(ctx.evidence_facts.is_none() && def.evidence_facts.is_none());
        assert!(ctx.source_queries.is_none() && def.source_queries.is_none());
    }

    #[test]
    fn authoritative_empty_in_scope_axis_is_preserved() {
        let ctx = GateContextBuilder::new()
            .authoritative_in_scope_assets(Some(Vec::new()))
            .build();
        assert_eq!(ctx.in_scope_assets, Some(Vec::new()));
    }

    #[test]
    fn canonical_source_hints_stay_out_of_gate_truth_and_reach_final_seal_builder() {
        let target_id = uuid::Uuid::new_v4();
        let built = GateContextBuilder::new()
            .extend_canonical_source_hints([CanonicalFactKey::Target { target_id }])
            .build_with_canonical_source_hints();
        assert_eq!(built.context.in_scope_assets, None);
        assert_eq!(
            built.canonical_source_hints,
            vec![CanonicalFactKey::Target { target_id }]
        );
    }

    #[test]
    fn empty_collections_normalize_to_none() {
        // 空 Vec / 空 map / 空 facts 都必须折成 None（与各入口手搓 then_some 一致）。
        let ctx = GateContextBuilder::new()
            .in_scope_assets(vec![])
            .typed_assets(vec![])
            .web_capable_assets(Vec::new())
            .not_applicable_coverage(Vec::new())
            .extend_evidence_facts(std::iter::empty::<EvidenceFact>())
            .extend_source_queries(std::iter::empty::<SourceQueryFact>())
            .build();
        assert!(ctx.in_scope_assets.is_none());
        assert!(ctx.asset_types.is_none());
        assert!(ctx.web_capable_assets.is_none());
        assert!(ctx.not_applicable_coverage.is_none());
        assert!(ctx.eas_required_web_origins.is_none());
        assert!(ctx.eas_completed_web_origins.is_none());
        assert!(ctx.evidence_facts.is_none());
        assert!(ctx.source_queries.is_none());
    }

    #[test]
    fn non_empty_in_scope_assets_become_some() {
        let ctx = GateContextBuilder::new()
            .in_scope_assets(vec!["a.com".into(), "b.com".into()])
            .build();
        assert_eq!(
            ctx.in_scope_assets,
            Some(vec!["a.com".to_string(), "b.com".to_string()])
        );
    }

    #[test]
    fn typed_assets_fold_into_map() {
        let ctx = GateContextBuilder::new()
            .typed_assets(vec![
                ("a.com".into(), "domain".into()),
                ("1.2.3.4".into(), "ip".into()),
            ])
            .build();
        let map = ctx.asset_types.expect("asset_types should be Some");
        assert_eq!(map.get("a.com").map(String::as_str), Some("domain"));
        assert_eq!(map.get("1.2.3.4").map(String::as_str), Some("ip"));
    }

    #[test]
    fn asset_types_map_setter_takes_prebuilt_map() {
        let mut m = HashMap::new();
        m.insert("a.com".to_string(), "domain".to_string());
        let ctx = GateContextBuilder::new().asset_types_map(m).build();
        assert_eq!(
            ctx.asset_types.and_then(|m| m.get("a.com").cloned()),
            Some("domain".to_string())
        );
    }

    #[test]
    fn web_capable_assets_normalize_to_some_when_present() {
        let ctx = GateContextBuilder::new()
            .web_capable_assets(vec!["1.2.3.4".to_string()])
            .build();
        let assets = ctx
            .web_capable_assets
            .expect("web_capable_assets should be Some");
        assert!(assets.contains("1.2.3.4"));
    }

    #[test]
    fn extend_evidence_facts_merges_multiple_sources() {
        // 模拟各入口的「ledger facts + db_truth facts」两段合并。
        let ledger = vec![fact("a.com", "GOLISH-INTEL-DNS", EvidenceOutcome::Found, 7)];
        let db_truth = vec![
            fact("a.com", "GOLISH-INTEL-ASN", EvidenceOutcome::Found, 0),
            fact("b.com", "GOLISH-INTEL-CT", EvidenceOutcome::Found, 0),
        ];
        let ctx = GateContextBuilder::new()
            .extend_evidence_facts(ledger)
            .extend_evidence_facts(db_truth)
            .build();
        let facts = ctx.evidence_facts.expect("evidence_facts should be Some");
        assert_eq!(facts.len(), 3);
        assert!(facts
            .iter()
            .any(|f| f.technique == "GOLISH-INTEL-DNS" && f.evidence_id == 7));
        assert!(facts.iter().any(|f| f.technique == "GOLISH-INTEL-CT"));
    }

    #[test]
    fn extend_source_queries_merges_multiple_sources() {
        let rows = vec![SourceQueryFact {
            source: "rdap".to_string(),
            query: "lookup_whois".to_string(),
            target: String::new(),
            technique: Some("GOLISH-INTEL-WHOIS".to_string()),
            status: "found".to_string(),
            evidence_ids: vec![9],
        }];
        let ctx = GateContextBuilder::new()
            .extend_source_queries(rows)
            .build();
        let rows = ctx.source_queries.expect("source_queries should be Some");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "rdap");
        assert_eq!(rows[0].technique.as_deref(), Some("GOLISH-INTEL-WHOIS"));
    }

    #[test]
    fn expected_techniques_passthrough_none_and_some() {
        let none_ctx = GateContextBuilder::new().expected_techniques(None).build();
        assert!(none_ctx.expected_techniques.is_none());

        let some_ctx = GateContextBuilder::new()
            .expected_techniques(Some(vec!["GOLISH-INTEL-DNS".into()]))
            .build();
        assert_eq!(
            some_ctx.expected_techniques,
            Some(vec!["GOLISH-INTEL-DNS".to_string()])
        );
    }

    #[test]
    fn empty_some_expected_techniques_is_not_normalized_to_none() {
        // Some(vec![]) 与 None 语义不同：前者「显式空期望」，不能被折成 None。
        let ctx = GateContextBuilder::new()
            .expected_techniques(Some(vec![]))
            .build();
        assert_eq!(ctx.expected_techniques, Some(vec![]));
    }

    #[test]
    fn full_build_mirrors_manual_struct_literal() {
        // 等价性：builder 产出 == 手搓 GateContext{}（接线行为保持的铁证）。
        let assets = vec!["a.com".to_string()];
        let facts = vec![fact("a.com", "GOLISH-INTEL-DNS", EvidenceOutcome::Found, 7)];
        let mut types = HashMap::new();
        types.insert("a.com".to_string(), "domain".to_string());

        let built = GateContextBuilder::new()
            .in_scope_assets(assets.clone())
            .asset_types_map(types.clone())
            .extend_evidence_facts(facts.clone())
            .expected_techniques(Some(vec!["GOLISH-INTEL-DNS".into()]))
            .build();

        let manual = GateContext {
            in_scope_assets: Some(assets),
            asset_types: Some(types),
            web_capable_assets: None,
            not_applicable_coverage: None,
            eas_required_web_origins: None,
            eas_completed_web_origins: None,
            expected_techniques: Some(vec!["GOLISH-INTEL-DNS".to_string()]),
            candidate_work_item_keys: None,
            verification_truth_required: false,
            verification_truth_snapshots: None,
            reporting_truth: None,
            evidence_facts: Some(facts),
            source_queries: None,
        };

        assert_eq!(built.in_scope_assets, manual.in_scope_assets);
        assert_eq!(built.asset_types, manual.asset_types);
        assert_eq!(built.web_capable_assets, manual.web_capable_assets);
        assert_eq!(
            built.not_applicable_coverage,
            manual.not_applicable_coverage
        );
        assert_eq!(built.expected_techniques, manual.expected_techniques);
        assert_eq!(built.evidence_facts, manual.evidence_facts);
    }
}
