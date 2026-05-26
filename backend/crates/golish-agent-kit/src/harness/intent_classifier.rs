//! Deterministic intent classifier (Doc 3 §6.1).
//!
//! Phase 1c.2 skeleton · 仅保留默认词库框架. Task 1c.3 完整词库 + 单测.

use serde::{Deserialize, Serialize};

use super::types::{IntentAxis, StageKind};

/// Doc 3 §6.1 词库容器.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentClassifierConfig {
    pub passive_keywords: Vec<String>,
    pub active_probe_keywords: Vec<String>,
    pub vuln_validation_keywords: Vec<String>,
    pub exploit_keywords: Vec<String>,
}

/// 主分类器 · 不用 LLM (同源带偏); 词库 → IntentAxis 一一映射.
pub struct IntentClassifier {
    config: IntentClassifierConfig,
}

impl IntentClassifier {
    pub fn new(config: IntentClassifierConfig) -> Self {
        Self { config }
    }

    /// Phase 1c.2 skeleton 默认配置 · Task 1c.3 加完整中英文词库.
    pub fn default_skeleton() -> Self {
        Self::new(IntentClassifierConfig::default())
    }

    /// Doc 3 §6.1 classify · 命中关键词决定 IntentAxis; 未命中走 stage_kind 默认.
    pub fn classify(&self, user_intent: &str, stage_kind: StageKind) -> IntentAxis {
        let lower = user_intent.to_lowercase();
        if self.config.exploit_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::ExploitValidation;
        }
        if self.config.vuln_validation_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::VulnValidation;
        }
        if self.config.active_probe_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::ActiveProbe;
        }
        if self.config.passive_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::PassiveObserve;
        }
        // 默认按 stage_kind (Doc 3 §6.1 match 子句)
        match stage_kind {
            StageKind::Scoping
            | StageKind::TargetIntel
            | StageKind::ExternalAttackSurface => IntentAxis::PassiveObserve,
            StageKind::Enumeration => IntentAxis::ActiveProbe,
            _ => IntentAxis::PassiveObserve,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_skeleton_falls_back_to_stage_kind() {
        let c = IntentClassifier::default_skeleton();
        assert_eq!(
            c.classify("跑一下", StageKind::Enumeration),
            IntentAxis::ActiveProbe
        );
        assert_eq!(
            c.classify("跑一下", StageKind::ExternalAttackSurface),
            IntentAxis::PassiveObserve
        );
    }

    #[test]
    fn explicit_keyword_overrides_stage_default() {
        let config = IntentClassifierConfig {
            passive_keywords: vec!["看看".to_string()],
            active_probe_keywords: vec!["扫描".to_string()],
            ..Default::default()
        };
        let c = IntentClassifier::new(config);
        // 「看看」命中 passive_keywords, 即便 stage=Enumeration 默认 ActiveProbe 也不覆盖
        assert_eq!(
            c.classify("看看子域", StageKind::Enumeration),
            IntentAxis::PassiveObserve
        );
    }
}
