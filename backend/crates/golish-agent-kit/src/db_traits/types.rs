//! Local model enums and structs for database operations.
//!
//! These types mirror the DB schema without pulling in `sqlx` or `golish-db`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use golish_pentest_domain::tool_truth::ToolTruthContract;

// ── Status enums ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtaskStatus {
    Created,
    Running,
    Waiting,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolcallStatus {
    Received,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Primary,
    Pentester,
    Coder,
    Searcher,
    Memorist,
    Reporter,
    Adviser,
    Reflector,
    Enricher,
    Installer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planning,
    InProgress,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// Lifecycle status of a sub-agent dispatch.
///
/// Mirrors the Postgres `sub_agent_dispatch_status` ENUM defined in the
/// `20260517000001_sub_agent_dispatches` migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl DispatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Observation,
    Conclusion,
    Technique,
    Vulnerability,
    ToolUsage,
}

// ── Input structs ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug)]
pub struct NewExecutionPlan {
    pub session_id: Option<Uuid>,
    pub project_path: Option<String>,
    pub title: String,
    pub description: String,
    pub steps: serde_json::Value,
    pub stage_id: Option<String>,
}

#[derive(Debug)]
pub struct NewTask {
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
}

#[derive(Debug)]
pub struct NewWikiPage {
    pub path: String,
    pub title: String,
    pub category: String,
    pub tags: Vec<String>,
    pub status: String,
    pub content: String,
}

#[derive(Debug)]
pub struct NewWikiChangelog {
    pub page_path: String,
    pub action: String,
    pub title: String,
    pub category: String,
    pub actor: String,
    pub summary: String,
}

// ── View structs (read-side projections) ────────────────────────────────

/// Memory hit row returned by search/fetch operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryHit {
    pub id: Uuid,
    pub content: String,
    pub mem_type: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Scored memory hit with optional tool name attribution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredMemoryHit {
    pub hit: MemoryHit,
    pub tool_name: Option<String>,
    pub score: f32,
}

/// Execution plan summary used in briefings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BriefingPlan {
    pub title: String,
    pub description: Option<String>,
    pub steps: serde_json::Value,
    pub current_step: i32,
    pub status: String,
}

/// Minimal view of a DB subtask (only fields accessed by golish-ai).
#[derive(Debug, Clone)]
pub struct SubtaskView {
    pub id: Uuid,
    pub status: SubtaskStatus,
    pub title: Option<String>,
    pub description: Option<String>,
    pub agent: Option<AgentType>,
    pub result: Option<String>,
}

/// Minimal view of a DB task (only fields accessed by golish-ai).
#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: Uuid,
    pub input: String,
    pub status: TaskStatus,
    pub result: Option<String>,
}

/// Minimal view of a DB execution plan.
#[derive(Debug, Clone)]
pub struct ExecutionPlanView {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub steps: serde_json::Value,
    pub status: PlanStatus,
    pub current_step: i32,
    pub stage_id: Option<String>,
}

/// Message chain record.
#[derive(Debug, Clone)]
pub struct MessageChainView {
    pub id: Uuid,
}

/// Minimal view of an `operation_state` cursor row (harness stage cursor, Doc 1 §3.4).
///
/// Only the fields the agent runtime reads back: which profile + which stage the
/// operation is currently on.
#[derive(Debug, Clone)]
pub struct OperationStateView {
    pub operation_id: Uuid,
    pub profile: String,
    pub current_stage: String,
    /// Immutable runtime-memory rollout contract frozen when this operation was
    /// created. Unknown persisted values fail closed in the app bridge.
    pub runtime_memory_contract: crate::runtime_memory::RuntimeMemoryContract,
    /// Tool/evidence authority contract frozen with the operation.
    pub tool_truth_contract: ToolTruthContract,
    /// Immutable Application Understanding/Candidate topology.
    pub application_model_contract: golish_core::ApplicationModelContract,
    /// Candidate/Hypothesis Registry schema contract frozen with the operation.
    pub investigation_contract_version: golish_core::InvestigationContractVersion,
    /// Candidate/Hypothesis Registry rollout mode frozen with the operation.
    pub investigation_rollout_mode: golish_core::InvestigationRolloutMode,
    /// Exact operation-frozen stage topology plus its canonical/hash witness.
    /// Runtime graph/profile selection must consume this material; it must
    /// never rederive an existing operation from a mutable deployment default.
    pub stage_topology_contract: golish_core::FrozenStageTopologyContractMaterial,
    /// Stable project/workspace authorization identity frozen when the runtime
    /// operation is created. Legacy operations may remain unbound.
    pub project_scope_id: Option<Uuid>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root org this operation is bound to. `None` = not yet
    /// bound; consumers must not reinterpret it as permission to read all orgs.
    pub engagement_org_id: Option<uuid::Uuid>,
    /// Harness-private resume state (JSONB). Carries `HarnessResumeState`
    /// (current stage run id + queue titles + completed count) for kill→resume.
    pub state_blob: serde_json::Value,
    /// When the current stage-run started (`stage_started_at`, set to NOW() on each
    /// `advance_stage`). The per-dimension freshness window (design 2026-06-22) uses
    /// it as `run_start` to discount DB-truth org-intel rows collected before this run.
    pub stage_started_at: chrono::DateTime<chrono::Utc>,
}

/// Durable asset wave read model for wave-aware `stage_run` fan-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAssetWaveView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub wave_index: i32,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub parent_wave_id: Option<Uuid>,
    pub asset_hash: String,
    /// Durable membership identity. Kept index-aligned with `asset_values`.
    pub target_ids: Vec<Uuid>,
    pub asset_values: Vec<String>,
}

/// Narrow request for the server-owned Tool Truth denominator compound.
/// Members, counts and hashes are deliberately absent: the bridge locks the
/// durable source and derives all three inside the sealing transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealToolTruthDenominatorRequest {
    pub stable_seal_request_id: Uuid,
    pub stage_execution_id: Uuid,
    pub source: ToolTruthDenominatorSourceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolTruthDenominatorSourceRef {
    StageAssetWave { stage_asset_wave_id: Uuid },
    StageTeamUnit { stage_run_unit_id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTruthDenominatorView {
    pub id: Uuid,
    pub execution_authority_id: Uuid,
    pub input_manifest_hash: String,
    pub member_count: i64,
    pub denominator_hash: String,
}

/// Trusted host request issued only after the deterministic org Gate has
/// accepted the stage deliverable and terminal producer outcomes are durable.
/// No model-authored counts, hashes, observations or evidence ids cross this
/// seam; the app repository re-derives them from the exact denominator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeStageToolTruthRequest {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_kind: String,
    pub stage_started_at: chrono::DateTime<chrono::Utc>,
    pub outcome_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageToolTruthCloseoutView {
    pub denominator_id: Uuid,
    pub expected_input_count: i64,
    pub finalized_receipt_count: i64,
    pub receipt_ids: Vec<Uuid>,
}

/// One immutable Enumeration root cell read back from the exact
/// StageTeamUnit denominator. Runtime uses this census to prove that a mutable
/// coverage projection neither added nor dropped an exact-origin/axis shard.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EnumerationFrozenRootMemberView {
    pub target_id: Uuid,
    pub exact_origin: String,
    pub technique: String,
    pub expected_capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolTruthCoverageView {
    pub denominator_id: Uuid,
    pub expected: usize,
    pub terminal: usize,
    pub degraded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordToolTruthShadowAssessment {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub stage_asset_wave_id: Option<Uuid>,
    pub legacy_allowed: bool,
}

/// Provenance-preserving projection of one materialized technique outcome.
/// `source` must survive the DB/application boundary because security-sensitive
/// terminal states (currently Enumeration `blocked`) trust one backend producer
/// only; a four-column tuple would discard the fact needed to enforce that at
/// submit preview and final org-gate time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TechniqueOutcomeFact {
    pub asset: String,
    pub technique: String,
    pub outcome: String,
    pub evidence_id: i64,
    pub source: Option<String>,
}

impl TechniqueOutcomeFact {
    pub fn new(
        asset: impl Into<String>,
        technique: impl Into<String>,
        outcome: impl Into<String>,
        evidence_id: i64,
        source: Option<String>,
    ) -> Self {
        Self {
            asset: asset.into(),
            technique: technique.into(),
            outcome: outcome.into(),
            evidence_id,
            source,
        }
    }
}

impl StageAssetWaveView {
    /// A present running wave is never equivalent to `NoWave`: corrupt or empty
    /// membership must fail closed before any caller considers cutoff fallback.
    pub fn validate_membership(&self) -> Result<(), String> {
        if self.target_ids.is_empty() || self.asset_values.is_empty() {
            return Err(format!("running asset wave {} has no items", self.id));
        }
        if self.target_ids.len() != self.asset_values.len() {
            return Err(format!(
                "running asset wave {} has mismatched target_ids ({}) and asset_values ({})",
                self.id,
                self.target_ids.len(),
                self.asset_values.len()
            ));
        }
        if self.target_ids.iter().any(Uuid::is_nil) {
            return Err(format!(
                "running asset wave {} contains a nil target_id",
                self.id
            ));
        }
        if self
            .asset_values
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(format!(
                "running asset wave {} contains a blank asset_value",
                self.id
            ));
        }
        let unique_ids = self
            .target_ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        if unique_ids.len() != self.target_ids.len() {
            return Err(format!(
                "running asset wave {} contains duplicate target_ids",
                self.id
            ));
        }
        Ok(())
    }
}

/// Minimal view of a sub-agent dispatch row, exposed to higher layers
/// (Tauri command + frontend) for the "resume after restart" feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentDispatchView {
    pub id: Uuid,
    pub parent_dispatch_id: Option<Uuid>,
    pub agent_id: String,
    pub tool_call_id: Option<String>,
    pub depth: i32,
    pub args: serde_json::Value,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod stage_asset_wave_tests {
    use super::*;

    fn wave(target_ids: Vec<Uuid>, asset_values: Vec<String>) -> StageAssetWaveView {
        StageAssetWaveView {
            id: Uuid::from_u128(1),
            operation_id: Uuid::from_u128(2),
            organization_id: Uuid::from_u128(3),
            stage_kind: "enumeration".to_string(),
            wave_index: 0,
            started_at: chrono::Utc::now(),
            parent_wave_id: None,
            asset_hash: "test".to_string(),
            target_ids,
            asset_values,
        }
    }

    #[test]
    fn running_wave_empty_items_fail_closed() {
        assert!(wave(Vec::new(), Vec::new()).validate_membership().is_err());
    }

    #[test]
    fn running_wave_blank_values_fail_closed() {
        assert!(wave(vec![Uuid::from_u128(9)], vec!["  ".to_string()])
            .validate_membership()
            .is_err());
    }

    #[test]
    fn running_wave_requires_aligned_unique_target_ids() {
        let id = Uuid::from_u128(9);
        assert!(wave(vec![id], vec!["a".to_string(), "b".to_string()])
            .validate_membership()
            .is_err());
        assert!(wave(vec![id, id], vec!["a".to_string(), "a".to_string()])
            .validate_membership()
            .is_err());
    }
}
