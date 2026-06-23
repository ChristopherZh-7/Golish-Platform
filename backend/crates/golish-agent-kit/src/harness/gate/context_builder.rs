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

use std::collections::HashMap;

use super::rule_engine::{EvidenceFact, GateContext};

/// 累加器式构造 [`GateContext`]。所有 setter 取所有权、可链式；空集合在
/// [`GateContextBuilder::build`] 统一折成 `None`（与各入口此前手搓的
/// `then_some` 行为逐字节一致）。
#[derive(Debug, Default, Clone)]
pub struct GateContextBuilder {
    in_scope_assets: Vec<String>,
    asset_types: HashMap<String, String>,
    evidence_facts: Vec<EvidenceFact>,
    expected_techniques: Option<Vec<String>>,
}

impl GateContextBuilder {
    /// 空 builder（`build()` → `GateContext::default()`）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 权威 in-scope 资产集。空 ⇒ `build()` 折成 `None`（gate 回退自报）。
    /// 幂等覆盖（非追加）。
    pub fn in_scope_assets(mut self, assets: Vec<String>) -> Self {
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

    /// 追加 evidence facts（ledger 投影 / DB 真值 / subsidiary 投影 …）。可多次
    /// 调用合并多个来源；顺序对 gate 无影响。
    pub fn extend_evidence_facts<I>(mut self, facts: I) -> Self
    where
        I: IntoIterator<Item = EvidenceFact>,
    {
        self.evidence_facts.extend(facts);
        self
    }

    /// 动态期望技术。`None` ⇒ gate 回退 `spec.expected_techniques`（静态）。
    /// 直接透传（不做空集合归一——`Some(vec![])` 与 `None` 语义不同，交给 gate）。
    pub fn expected_techniques(mut self, techniques: Option<Vec<String>>) -> Self {
        self.expected_techniques = techniques;
        self
    }

    /// 组装 [`GateContext`]：**唯一** `empty → None` 归一点。
    pub fn build(self) -> GateContext {
        GateContext {
            in_scope_assets: (!self.in_scope_assets.is_empty()).then_some(self.in_scope_assets),
            asset_types: (!self.asset_types.is_empty()).then_some(self.asset_types),
            expected_techniques: self.expected_techniques,
            evidence_facts: (!self.evidence_facts.is_empty()).then_some(self.evidence_facts),
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
        assert_eq!(ctx.expected_techniques, def.expected_techniques);
        assert!(ctx.evidence_facts.is_none() && def.evidence_facts.is_none());
    }

    #[test]
    fn empty_collections_normalize_to_none() {
        // 空 Vec / 空 map / 空 facts 都必须折成 None（与各入口手搓 then_some 一致）。
        let ctx = GateContextBuilder::new()
            .in_scope_assets(vec![])
            .typed_assets(vec![])
            .extend_evidence_facts(std::iter::empty::<EvidenceFact>())
            .build();
        assert!(ctx.in_scope_assets.is_none());
        assert!(ctx.asset_types.is_none());
        assert!(ctx.evidence_facts.is_none());
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
        assert!(facts.iter().any(|f| f.technique == "GOLISH-INTEL-DNS" && f.evidence_id == 7));
        assert!(facts.iter().any(|f| f.technique == "GOLISH-INTEL-CT"));
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
            expected_techniques: Some(vec!["GOLISH-INTEL-DNS".to_string()]),
            evidence_facts: Some(facts),
        };

        assert_eq!(built.in_scope_assets, manual.in_scope_assets);
        assert_eq!(built.asset_types, manual.asset_types);
        assert_eq!(built.expected_techniques, manual.expected_techniques);
        assert_eq!(built.evidence_facts, manual.evidence_facts);
    }
}
