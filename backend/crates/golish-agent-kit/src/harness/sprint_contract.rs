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
///
/// Phase 1c.2 skeleton · Task 1c.4 加 default impl + cross-vendor LLM 接入.
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
}
