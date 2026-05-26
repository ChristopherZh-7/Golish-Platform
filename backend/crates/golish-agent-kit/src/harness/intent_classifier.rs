//! Deterministic intent classifier (Doc 3 §6.1).
//!
//! 不用 LLM (同源带偏); 词库查表 deterministic, agent 不可绕过.
//!
//! Phase 1c.3 完整词库版本: 中英文双语关键词覆盖 4 档 IntentAxis.
//! 优先级 (Doc 3 §6.1 match 顺序, 高 > 低):
//!   ExploitValidation > VulnValidation > ActiveProbe > PassiveObserve.
//! 全部未命中 → stage_kind 默认.

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

impl IntentClassifierConfig {
    /// Phase 1c.3 默认词库 · 中英文 + 安全行业常用术语.
    ///
    /// 设计原则:
    ///   1. 每档至少 8-10 个关键词覆盖常见用户表述
    ///   2. 词库小写存储, classify 时输入也 lower-case → ASCII 大小写不敏感
    ///   3. 词库**不重叠** (一个词不会同时进 ActiveProbe + VulnValidation),
    ///      保证 match 优先级确定
    ///   4. 中文不分词, 直接子串匹配 (containment)
    pub fn default_keywords() -> Self {
        Self {
            // PassiveObserve: 公开数据库 / passive DNS / CT log / 资产盘点
            passive_keywords: vec![
                // 中文
                "看看".to_string(),
                "调研".to_string(),
                "查一下".to_string(),
                "盘点".to_string(),
                "梳理".to_string(),
                "列举".to_string(),
                "罗列".to_string(),
                "了解".to_string(),
                "枚举资产".to_string(),
                "查询".to_string(),
                // 英文
                "passive".to_string(),
                "observe".to_string(),
                "list".to_string(),
                "enumerate assets".to_string(),
                "inventory".to_string(),
                "lookup".to_string(),
                "research".to_string(),
                "read-only".to_string(),
                "intel".to_string(),
            ],

            // ActiveProbe: 低风险主动探测 (DNS / HTTP / 子域枚举 / shodan)
            active_probe_keywords: vec![
                // 中文
                "扫描".to_string(),
                "探测".to_string(),
                "主动".to_string(),
                "枚举子域".to_string(),
                "枚举端口".to_string(),
                "端口".to_string(),
                "指纹".to_string(),
                "抓包".to_string(),
                "请求".to_string(),
                "测试连通".to_string(),
                // 英文
                "scan".to_string(),
                "probe".to_string(),
                "active".to_string(),
                "fingerprint".to_string(),
                "enumerate subdomains".to_string(),
                "port scan".to_string(),
                "ping".to_string(),
                "http probe".to_string(),
                "subdomain".to_string(),
            ],

            // VulnValidation: 非破坏性漏洞验证
            vuln_validation_keywords: vec![
                // 中文
                "验证漏洞".to_string(),
                "复现".to_string(),
                "poc 验证".to_string(),
                "漏洞确认".to_string(),
                "trial".to_string(),
                "试一下漏洞".to_string(),
                "漏洞测试".to_string(),
                "确认存在".to_string(),
                // 英文
                "vuln validation".to_string(),
                "vulnerability check".to_string(),
                "poc check".to_string(),
                "validate vuln".to_string(),
                "confirm vuln".to_string(),
                "reproduce".to_string(),
                "trial poc".to_string(),
                "check cve".to_string(),
            ],

            // ExploitValidation: 实际利用 / payload
            exploit_keywords: vec![
                // 中文
                "利用".to_string(),
                "exploit".to_string(),
                "打 payload".to_string(),
                "提权".to_string(),
                "rce".to_string(),
                "拿权限".to_string(),
                "拿 shell".to_string(),
                "横移".to_string(),
                "后渗透".to_string(),
                "权限维持".to_string(),
                // 英文
                "payload".to_string(),
                "weaponize".to_string(),
                "lateral movement".to_string(),
                "privilege escalation".to_string(),
                "shell".to_string(),
                "post-exploit".to_string(),
                "metasploit".to_string(),
                "msf".to_string(),
            ],
        }
    }
}

/// 主分类器 · 不用 LLM (同源带偏); 词库 → IntentAxis 一一映射.
pub struct IntentClassifier {
    config: IntentClassifierConfig,
}

impl IntentClassifier {
    pub fn new(config: IntentClassifierConfig) -> Self {
        Self { config }
    }

    /// Phase 1c.3 默认配置 · 使用内置中英文词库.
    pub fn with_default_keywords() -> Self {
        Self::new(IntentClassifierConfig::default_keywords())
    }

    /// Phase 1c.2 skeleton fallback · 空词库, 仅走 stage_kind default.
    pub fn default_skeleton() -> Self {
        Self::new(IntentClassifierConfig::default())
    }

    /// Doc 3 §6.1 classify · 命中关键词决定 IntentAxis; 未命中走 stage_kind 默认.
    ///
    /// **match 顺序硬编码** (优先级最高在前):
    ///   ExploitValidation > VulnValidation > ActiveProbe > PassiveObserve.
    pub fn classify(&self, user_intent: &str, stage_kind: StageKind) -> IntentAxis {
        let lower = user_intent.to_lowercase();
        if self.config.exploit_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::ExploitValidation;
        }
        if self
            .config
            .vuln_validation_keywords
            .iter()
            .any(|k| lower.contains(k))
        {
            return IntentAxis::VulnValidation;
        }
        if self
            .config
            .active_probe_keywords
            .iter()
            .any(|k| lower.contains(k))
        {
            return IntentAxis::ActiveProbe;
        }
        if self.config.passive_keywords.iter().any(|k| lower.contains(k)) {
            return IntentAxis::PassiveObserve;
        }
        // 默认按 stage_kind (Doc 3 §6.1 match 子句末)
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

    fn classifier() -> IntentClassifier {
        IntentClassifier::with_default_keywords()
    }

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
    fn passive_chinese_keywords_match() {
        let c = classifier();
        // stage 设 Enumeration 默认 ActiveProbe, passive 关键词应覆盖
        for k in &["看看", "调研", "盘点", "梳理", "了解"] {
            let intent = format!("{}域名", k);
            assert_eq!(
                c.classify(&intent, StageKind::Enumeration),
                IntentAxis::PassiveObserve,
                "expected PassiveObserve for: {}",
                intent
            );
        }
    }

    #[test]
    fn passive_english_keywords_match() {
        let c = classifier();
        // 注意: 不要在 passive 测试中混入 active 关键词 (如 "subdomain"/"scan"/"probe"),
        // 因为 ActiveProbe 优先级高于 PassiveObserve.
        for k in &["passive", "observe", "list", "inventory", "research"] {
            let intent = format!("please {} the assets", k);
            assert_eq!(
                c.classify(&intent, StageKind::Enumeration),
                IntentAxis::PassiveObserve,
                "expected PassiveObserve for: {}",
                intent
            );
        }
    }

    #[test]
    fn active_probe_keywords_match() {
        let c = classifier();
        for k in &["扫描", "探测", "主动", "端口", "指纹"] {
            let intent = format!("{}所有域名", k);
            assert_eq!(
                c.classify(&intent, StageKind::Scoping),
                IntentAxis::ActiveProbe,
                "expected ActiveProbe for: {}",
                intent
            );
        }
        for k in &["scan", "probe", "fingerprint", "subdomain"] {
            let intent = format!("please {} the asset", k);
            assert_eq!(
                c.classify(&intent, StageKind::Scoping),
                IntentAxis::ActiveProbe,
                "expected ActiveProbe for: {}",
                intent
            );
        }
    }

    #[test]
    fn vuln_validation_keywords_match() {
        let c = classifier();
        for k in &["验证漏洞", "复现", "poc 验证", "漏洞测试"] {
            assert_eq!(
                c.classify(k, StageKind::ExternalAttackSurface),
                IntentAxis::VulnValidation,
                "expected VulnValidation for: {}",
                k
            );
        }
        for k in &["validate vuln", "confirm vuln", "reproduce", "check cve"] {
            assert_eq!(
                c.classify(k, StageKind::ExternalAttackSurface),
                IntentAxis::VulnValidation,
                "expected VulnValidation for: {}",
                k
            );
        }
    }

    #[test]
    fn exploit_keywords_match() {
        let c = classifier();
        for k in &["利用", "rce", "拿 shell", "提权", "横移"] {
            assert_eq!(
                c.classify(k, StageKind::ExternalAttackSurface),
                IntentAxis::ExploitValidation,
                "expected ExploitValidation for: {}",
                k
            );
        }
        for k in &["payload", "lateral movement", "metasploit", "msf"] {
            assert_eq!(
                c.classify(k, StageKind::ExternalAttackSurface),
                IntentAxis::ExploitValidation,
                "expected ExploitValidation for: {}",
                k
            );
        }
    }

    #[test]
    fn priority_exploit_beats_vuln_when_both_present() {
        let c = classifier();
        // 同时含 "复现" (vuln) + "payload" (exploit) → ExploitValidation 优先
        assert_eq!(
            c.classify("用 payload 复现漏洞", StageKind::ExternalAttackSurface),
            IntentAxis::ExploitValidation
        );
    }

    #[test]
    fn priority_vuln_beats_active_when_both_present() {
        let c = classifier();
        // 同时含 "扫描" (active) + "验证漏洞" (vuln) → VulnValidation 优先
        assert_eq!(
            c.classify("扫描后验证漏洞", StageKind::Scoping),
            IntentAxis::VulnValidation
        );
    }

    #[test]
    fn priority_active_beats_passive_when_both_present() {
        let c = classifier();
        // 同时含 "看看" (passive) + "扫描" (active) → ActiveProbe 优先
        assert_eq!(
            c.classify("看看再扫描一下", StageKind::Scoping),
            IntentAxis::ActiveProbe
        );
    }

    #[test]
    fn case_insensitive_english() {
        let c = classifier();
        assert_eq!(
            c.classify("PROBE the network", StageKind::Scoping),
            IntentAxis::ActiveProbe
        );
        assert_eq!(
            c.classify("RCE attempt", StageKind::Scoping),
            IntentAxis::ExploitValidation
        );
    }

    #[test]
    fn stage_kind_default_routing_with_empty_intent() {
        let c = classifier();
        assert_eq!(
            c.classify("", StageKind::Scoping),
            IntentAxis::PassiveObserve
        );
        assert_eq!(
            c.classify("", StageKind::TargetIntel),
            IntentAxis::PassiveObserve
        );
        assert_eq!(
            c.classify("", StageKind::ExternalAttackSurface),
            IntentAxis::PassiveObserve
        );
        assert_eq!(
            c.classify("", StageKind::Enumeration),
            IntentAxis::ActiveProbe
        );
        // 其它 stage 默认 PassiveObserve (最保守)
        assert_eq!(
            c.classify("", StageKind::VulnTriage),
            IntentAxis::PassiveObserve
        );
        assert_eq!(
            c.classify("", StageKind::Cleanup),
            IntentAxis::PassiveObserve
        );
    }

    #[test]
    fn default_keywords_each_axis_non_empty() {
        let cfg = IntentClassifierConfig::default_keywords();
        assert!(cfg.passive_keywords.len() >= 10);
        assert!(cfg.active_probe_keywords.len() >= 10);
        assert!(cfg.vuln_validation_keywords.len() >= 8);
        assert!(cfg.exploit_keywords.len() >= 8);
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
