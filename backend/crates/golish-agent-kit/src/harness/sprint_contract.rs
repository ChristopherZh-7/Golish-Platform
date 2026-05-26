//! Sprint Contract DTO + Generator stub (Doc 3 §7).
//!
//! Phase 1c.2 skeleton · DTO + skeleton parser. Task 1c.4 完整 Generator + cross-vendor
//! LLM 填变量 + repo 接入.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::types::StageKind;

/// Doc 3 §7.1 expected_finding · sprint skeleton 中的一项.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedFinding {
    pub kind: String,
    /// `[min, max]` 范围 · planner LLM 会按 scope 调整.
    pub expected_count_range: [u32; 2],
    pub required_evidence_kinds: Vec<String>,
}

/// Doc 3 §7.1 sprint skeleton · 一个 stage 的所有期望.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSkeleton {
    pub expected_findings: Vec<ExpectedFinding>,
    pub time_budget_minutes: u32,
    #[serde(default)]
    pub min_tool_invocations: HashMap<String, u32>,
}

/// Profile-level skeleton · per-stage map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintSkeleton {
    #[serde(flatten)]
    pub stages: HashMap<String, StageSkeleton>,
}

impl SprintSkeleton {
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        // 跳过 $schema / $comment
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let map = match value {
            serde_json::Value::Object(m) => m,
            other => {
                return serde_json::from_value::<Self>(other);
            }
        };
        let stages: HashMap<String, StageSkeleton> = map
            .into_iter()
            .filter(|(k, _)| !k.starts_with('$'))
            .map(|(k, v)| serde_json::from_value::<StageSkeleton>(v).map(|s| (k, s)))
            .collect::<Result<_, _>>()?;
        Ok(SprintSkeleton { stages })
    }

    pub fn for_stage(&self, stage_kind: StageKind) -> Option<&StageSkeleton> {
        self.stages.get(stage_kind.as_str())
    }
}

/// Doc 3 §7 SprintContract DTO · 写入 sprint_contracts 表.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintContract {
    pub id: Uuid,
    pub stage_run_id: Uuid,
    pub contract_text: String,
    pub locked_after: DateTime<Utc>,
    pub status: String,
    pub planner_llm_id: String,
}

impl SprintContract {
    pub fn new_active(stage_run_id: Uuid, contract_text: String, planner_llm_id: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            stage_run_id,
            contract_text,
            locked_after: Utc::now(),
            status: "active".to_string(),
            planner_llm_id,
        }
    }
}

/// Doc 3 §7.2 generator trait · planner LLM 填变量.
#[async_trait]
pub trait SprintContractGenerator: Send + Sync {
    async fn generate(
        &self,
        stage_run_id: Uuid,
        stage_kind: StageKind,
        skeleton: &StageSkeleton,
        scope_context: &str,
    ) -> Result<SprintContract>;
}

/// Default deterministic generator (Phase 1c.4 MVP) · 不调 LLM, 直接基于
/// skeleton 拼 contract_text. v1 在 Phase 2 替换为 cross-vendor LLM impl.
///
/// 设计理由:
///   - Phase 1 数据收集阶段, planner LLM 接入是质量风险点 (跨厂商 fallback /
///     temperature / token budget 都没数据收敛). 先用 deterministic 版本跑通
///     pipeline, 等真实使用数据回来后再决定 LLM impl 细节.
///   - planner_llm_id = "deterministic-default" 让 audit_log 上能区分两种版本.
pub struct DefaultSprintContractGenerator;

#[async_trait]
impl SprintContractGenerator for DefaultSprintContractGenerator {
    async fn generate(
        &self,
        stage_run_id: Uuid,
        stage_kind: StageKind,
        skeleton: &StageSkeleton,
        scope_context: &str,
    ) -> Result<SprintContract> {
        let contract_text = render_contract_text(stage_kind, skeleton, scope_context);
        Ok(SprintContract::new_active(
            stage_run_id,
            contract_text,
            "deterministic-default".to_string(),
        ))
    }
}

/// 把 skeleton + scope_context 拼成人话 contract 文本 (Doc 3 §7.2 deterministic
/// fallback path; LLM 接入是 v1 之后).
///
/// 文本结构:
///   - Header: stage_kind + scope_context
///   - Expected findings 列表 (kind + count range + required_evidence_kinds)
///   - Tool invocation budget
///   - Time budget
fn render_contract_text(
    stage_kind: StageKind,
    skeleton: &StageSkeleton,
    scope_context: &str,
) -> String {
    use std::fmt::Write as _;

    let mut s = String::new();
    let _ = writeln!(s, "# Sprint Contract · {}", stage_kind.as_str());
    let _ = writeln!(s, "scope_context: {}", scope_context);
    let _ = writeln!(s, "time_budget_minutes: {}", skeleton.time_budget_minutes);
    let _ = writeln!(s, "expected_findings:");
    for ef in &skeleton.expected_findings {
        let _ = writeln!(
            s,
            "  - kind: {} | count_range: [{}, {}] | required_evidence_kinds: {}",
            ef.kind,
            ef.expected_count_range[0],
            ef.expected_count_range[1],
            ef.required_evidence_kinds.join(", "),
        );
    }
    if !skeleton.min_tool_invocations.is_empty() {
        let _ = writeln!(s, "min_tool_invocations:");
        // 按 key 排序输出 (deterministic test diffability)
        let mut keys: Vec<_> = skeleton.min_tool_invocations.keys().collect();
        keys.sort();
        for k in keys {
            let _ = writeln!(s, "  - {}: {}", k, skeleton.min_tool_invocations[k]);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSESSMENT_SKELETON_JSON: &str = include_str!(
        "../../../../../resources/harness/profiles/assessment.sprint_skeleton.json"
    );

    #[test]
    fn parse_assessment_skeleton_has_external_attack_surface() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        assert!(s
            .for_stage(StageKind::ExternalAttackSurface)
            .is_some());
    }

    #[test]
    fn external_attack_surface_expected_findings() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        assert_eq!(stage.expected_findings.len(), 2);
        let subdomain = &stage.expected_findings[0];
        assert_eq!(subdomain.kind, "subdomain");
        assert_eq!(subdomain.expected_count_range, [1, 200]);
        assert!(subdomain.required_evidence_kinds.contains(&"dns_a".to_string()));
        assert!(subdomain.required_evidence_kinds.contains(&"ct_log".to_string()));
    }

    #[test]
    fn external_attack_surface_time_budget_30_min() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        assert_eq!(stage.time_budget_minutes, 30);
    }

    #[test]
    fn external_attack_surface_min_tool_invocations() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        assert_eq!(stage.min_tool_invocations.get("dns_resolve"), Some(&1));
        assert_eq!(stage.min_tool_invocations.get("http_probe"), Some(&1));
        assert_eq!(stage.min_tool_invocations.get("subdomain_enum_passive"), Some(&1));
    }

    #[test]
    fn sprint_contract_new_active_locks_time_and_status() {
        let c = SprintContract::new_active(
            Uuid::new_v4(),
            "expected 1-200 subdomains".to_string(),
            "openai:gpt-4o".to_string(),
        );
        assert_eq!(c.status, "active");
        assert_eq!(c.planner_llm_id, "openai:gpt-4o");
        assert!(c.locked_after <= Utc::now());
    }

    #[tokio::test]
    async fn default_generator_uses_deterministic_marker() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        let gen = DefaultSprintContractGenerator;
        let stage_run_id = Uuid::new_v4();
        let contract = gen
            .generate(
                stage_run_id,
                StageKind::ExternalAttackSurface,
                stage,
                "example.com",
            )
            .await
            .expect("generate");
        assert_eq!(contract.planner_llm_id, "deterministic-default");
        assert_eq!(contract.stage_run_id, stage_run_id);
        assert_eq!(contract.status, "active");
    }

    #[tokio::test]
    async fn default_generator_renders_skeleton_into_text() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        let gen = DefaultSprintContractGenerator;
        let contract = gen
            .generate(
                Uuid::new_v4(),
                StageKind::ExternalAttackSurface,
                stage,
                "example.com",
            )
            .await
            .expect("generate");

        // contract_text 必须含关键字段
        assert!(contract.contract_text.contains("external_attack_surface"));
        assert!(contract.contract_text.contains("example.com"));
        assert!(contract.contract_text.contains("subdomain"));
        assert!(contract.contract_text.contains("http_service"));
        assert!(contract.contract_text.contains("time_budget_minutes: 30"));
        assert!(contract.contract_text.contains("dns_resolve"));
        assert!(contract.contract_text.contains("http_probe"));
    }

    #[tokio::test]
    async fn default_generator_is_deterministic_across_calls() {
        let s = SprintSkeleton::from_json(ASSESSMENT_SKELETON_JSON).expect("parse");
        let stage = s.for_stage(StageKind::ExternalAttackSurface).unwrap();
        let gen = DefaultSprintContractGenerator;
        let scope = "example.com scope";
        // 同一 stage_run_id 调两次应该有相同 contract_text (deterministic)
        let stage_run_id = Uuid::new_v4();
        let c1 = gen
            .generate(stage_run_id, StageKind::ExternalAttackSurface, stage, scope)
            .await
            .unwrap();
        let c2 = gen
            .generate(stage_run_id, StageKind::ExternalAttackSurface, stage, scope)
            .await
            .unwrap();
        assert_eq!(c1.contract_text, c2.contract_text);
        // id 仍不同 (每次 new uuid v4)
        assert_ne!(c1.id, c2.id);
    }

    #[test]
    fn render_text_handles_empty_min_invocations() {
        let skeleton = StageSkeleton {
            expected_findings: vec![],
            time_budget_minutes: 15,
            min_tool_invocations: std::collections::HashMap::new(),
        };
        let text = render_contract_text(StageKind::Reporting, &skeleton, "");
        assert!(text.contains("reporting"));
        // 没 min_invocations 时不输出该段
        assert!(!text.contains("min_tool_invocations:"));
    }
}
