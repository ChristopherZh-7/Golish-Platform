//! Cross-session operation continuity decisions.
//!
//! This module is deliberately IO-free: it takes a profile-projected DAG plus a
//! caller-built snapshot of durable DB truth and decides where a new operation
//! should start after the user confirms reuse. The DB/UI layers decide whether
//! reuse is allowed; this module only owns deterministic stage-cursor math.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::operation_flow::branch_target;
use super::operation_graph::AllowedDag;
use super::types::StageKind;

/// User-facing continuity strategy for a new operation when durable DB facts
/// already exist outside the current chat session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContinuityDecision {
    /// Stop before starting the harness and ask the user whether to reuse.
    #[default]
    AskBeforeReuse,
    /// Adopt reusable DB-backed stages and continue from the first gap.
    ReuseExisting,
    /// Ignore older DB-backed progress and start a fresh harness run.
    StartFresh,
}

impl ContinuityDecision {
    pub fn try_parse(value: &str) -> Option<Self> {
        match value.trim() {
            "ask_before_reuse" | "ask" => Some(Self::AskBeforeReuse),
            "reuse_existing" | "reuse" => Some(Self::ReuseExisting),
            "start_fresh" | "fresh" | "ignore_existing" => Some(Self::StartFresh),
            _ => None,
        }
    }
}

/// Deterministic reuse status for one stage in a continuity snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageReuseStatus {
    /// The stage passed and its durable facts are fresh enough to adopt.
    Reusable,
    /// The stage passed, but its outcome intentionally takes the no-progress
    /// branch (for example EAS found no live surface, so the DAG should bail to
    /// reporting rather than run enumeration).
    ReusableNoProgress,
    /// Some but not all required facts are reusable; this is the resume cursor.
    Partial,
    /// Prior facts exist but are outside the freshness window.
    Stale,
    /// Prior facts conflict with the current scope/profile and must not be
    /// silently adopted.
    Conflict,
    /// No durable proof exists for this stage.
    Missing,
}

impl StageReuseStatus {
    fn reusable_made_progress(self) -> Option<bool> {
        match self {
            Self::Reusable => Some(true),
            Self::ReusableNoProgress => Some(false),
            Self::Partial | Self::Stale | Self::Conflict | Self::Missing => None,
        }
    }

    pub fn is_reusable(self) -> bool {
        self.reusable_made_progress().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReuseSummary {
    pub stage: StageKind,
    pub status: StageReuseStatus,
    /// Human-readable summary of why this stage is or is not reusable.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContinuitySnapshot {
    /// Number of in-scope organizations/assets used to build the snapshot.
    pub scope_units: usize,
    pub stages: Vec<StageReuseSummary>,
}

impl ContinuitySnapshot {
    pub fn has_reusable_progress(&self) -> bool {
        self.stages.iter().any(|s| s.status.is_reusable())
    }

    fn status_for(&self, stage: StageKind) -> StageReuseStatus {
        self.stages
            .iter()
            .find(|s| s.stage == stage)
            .map(|s| s.status)
            .unwrap_or(StageReuseStatus::Missing)
    }
}

/// The cursor seed a confirmed-reuse operation should apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuityAdoptionPlan {
    #[serde(default = "default_schema_v")]
    pub schema_v: u32,
    /// Stages that are adopted and therefore skipped in the new operation.
    pub adopted_stages: Vec<StageKind>,
    /// The first stage the new operation must execute.
    pub entry_stage: StageKind,
    /// Profile-DAG stages still allowed to run, in DAG order. The orchestrator
    /// feeds this into its stage allowlist so the executor really starts at
    /// `entry_stage` rather than the original profile entry.
    pub remaining_stages: Vec<StageKind>,
    /// True when all projected stages were reusable. The caller usually reruns
    /// the terminal/reporting stage to produce a fresh closeout for this chat.
    pub all_projected_stages_reusable: bool,
    pub snapshot: ContinuitySnapshot,
}

fn default_schema_v() -> u32 {
    1
}

/// Decide the first stage to run after adopting reusable prefix stages.
///
/// The function walks the projected DAG from its entry. For every reusable
/// stage, it follows the same branch rule as the live operation flow
/// (`made_progress=true` => main path, false => bail path). The first
/// non-reusable stage becomes the new entry stage.
pub fn plan_adoption_cursor(
    dag: &AllowedDag,
    snapshot: ContinuitySnapshot,
) -> Option<ContinuityAdoptionPlan> {
    if !snapshot.has_reusable_progress() {
        return None;
    }

    let mut current = *dag.entry_points().first()?;
    let mut adopted = Vec::new();
    let mut seen: HashMap<StageKind, ()> = HashMap::new();

    loop {
        if seen.insert(current, ()).is_some() {
            return None;
        }

        let status = snapshot.status_for(current);
        let Some(made_progress) = status.reusable_made_progress() else {
            return Some(plan_from_entry(dag, snapshot, adopted, current, false));
        };

        adopted.push(current);
        let nexts = dag.next_stages(current);
        if nexts.is_empty() {
            return Some(plan_from_entry(dag, snapshot, adopted, current, true));
        }
        current = if nexts.len() == 1 {
            nexts[0]
        } else {
            branch_target(&nexts, made_progress).unwrap_or(nexts[0])
        };
    }
}

fn plan_from_entry(
    dag: &AllowedDag,
    snapshot: ContinuitySnapshot,
    adopted_stages: Vec<StageKind>,
    entry_stage: StageKind,
    all_projected_stages_reusable: bool,
) -> ContinuityAdoptionPlan {
    let remaining_set = dag.descendants_inclusive(entry_stage);
    let remaining_stages = dag
        .nodes
        .iter()
        .copied()
        .filter(|stage| remaining_set.contains(stage))
        .collect();
    ContinuityAdoptionPlan {
        schema_v: 1,
        adopted_stages,
        entry_stage,
        remaining_stages,
        all_projected_stages_reusable,
        snapshot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::operation_graph::load_operation_graph_from_json;
    use crate::harness::profile::load_profile_from_json;

    const BASE_GRAPH_JSON: &str =
        include_str!("../../../../../resources/harness/graph/operation_graph.json");
    const ASSESSMENT_JSON: &str =
        include_str!("../../../../../resources/harness/profiles/assessment.json");

    fn assessment_dag() -> AllowedDag {
        let graph = load_operation_graph_from_json(BASE_GRAPH_JSON).expect("base graph");
        let profile = load_profile_from_json(ASSESSMENT_JSON).expect("assessment profile");
        graph.project(&profile.allowed_stage_set())
    }

    fn summary(stage: StageKind, status: StageReuseStatus) -> StageReuseSummary {
        StageReuseSummary {
            stage,
            status,
            detail: format!("{} => {:?}", stage.as_str(), status),
        }
    }

    #[test]
    fn reusable_prefix_jumps_to_first_gap() {
        let dag = assessment_dag();
        let snapshot = ContinuitySnapshot {
            scope_units: 3,
            stages: vec![
                summary(StageKind::Scoping, StageReuseStatus::Reusable),
                summary(StageKind::TargetIntel, StageReuseStatus::Reusable),
                summary(StageKind::ExternalAttackSurface, StageReuseStatus::Partial),
            ],
        };

        let plan = plan_adoption_cursor(&dag, snapshot).expect("adoption plan");

        assert_eq!(
            plan.adopted_stages,
            vec![StageKind::Scoping, StageKind::TargetIntel]
        );
        assert_eq!(plan.entry_stage, StageKind::ExternalAttackSurface);
        assert!(!plan.remaining_stages.contains(&StageKind::Scoping));
        assert!(plan.remaining_stages.contains(&StageKind::Enumeration));
    }

    #[test]
    fn no_progress_adoption_follows_bail_branch() {
        let dag = assessment_dag();
        let snapshot = ContinuitySnapshot {
            scope_units: 1,
            stages: vec![
                summary(StageKind::Scoping, StageReuseStatus::Reusable),
                summary(StageKind::TargetIntel, StageReuseStatus::Reusable),
                summary(
                    StageKind::ExternalAttackSurface,
                    StageReuseStatus::ReusableNoProgress,
                ),
                summary(StageKind::Reporting, StageReuseStatus::Missing),
            ],
        };

        let plan = plan_adoption_cursor(&dag, snapshot).expect("adoption plan");

        assert_eq!(plan.entry_stage, StageKind::Reporting);
        assert!(plan
            .adopted_stages
            .contains(&StageKind::ExternalAttackSurface));
        assert!(!plan.remaining_stages.contains(&StageKind::Enumeration));
    }

    #[test]
    fn no_reusable_progress_returns_none() {
        let dag = assessment_dag();
        let snapshot = ContinuitySnapshot {
            scope_units: 2,
            stages: vec![summary(StageKind::Scoping, StageReuseStatus::Missing)],
        };

        assert!(plan_adoption_cursor(&dag, snapshot).is_none());
    }
}
