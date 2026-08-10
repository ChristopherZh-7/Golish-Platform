//! Engine v2 · P2 方案 C（Executor-driven run）— orchestrator 侧基础件。
//!
//! 用户 2026-06-02 选定方案 C：把 `run()` 改成 metalcraft Executor 驱动、subtask 循环
//! 折进 stage 节点体。**最高风险**，故 **增量 + flag 闸 + 旧 `execute_subtask_loop`
//! 保留为 fallback/回滚**，新路径证明前绝不删旧路径（见
//! `docs/superpowers/plans/2026-06-02-engine-v2-p2-metalcraft-graph-executor.md` §方案 C）。
//!
//! **C-1（本文件 · additive · 零 live 变更）**：[`group_subtasks_by_stage`] —— 把扁平
//! subtask 队列按 stage 分组，是「一个 stage 节点跑该组 subtask」的前提。纯函数，可单测，
//! 不碰 `run()`。C-2（`OrchestratorStageRunner` 内部可变）/ C-3（flag 闸 run 分流）/
//! C-4（DB checkpointer）是后续较大、较高风险的步骤。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::db_traits::DbRepoProvider;
use crate::harness::graph_engine::{Checkpointer, GraphError, Result as GraphResult};
use crate::harness::operation_flow::OperationFlowState;
use crate::harness::StageKind;

use super::types::PlannedSubtask;

/// Durable lifecycle state for one exact stage execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageExecutionStatus {
    Started,
    Completed,
    Failed,
    PausedNeedsUser,
}

impl StageExecutionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::PausedNeedsUser => "paused_needs_user",
        }
    }

    pub fn try_parse(value: &str) -> Option<Self> {
        match value {
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "paused_needs_user" => Some(Self::PausedNeedsUser),
            _ => None,
        }
    }
}

/// SQLx-free identity of one durable stage execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageExecution {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage: StageKind,
    pub status: StageExecutionStatus,
}

/// Compare-and-swap command for moving an operation to its next stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionStageExecution {
    pub operation_id: Uuid,
    pub current_stage_execution_id: Uuid,
    pub next_stage_execution_id: Uuid,
    pub next_stage: StageKind,
}

/// Compare-and-set command for a deliberate headless stage boundary. The
/// repository performs the normal stage transition, records an exact
/// source/successor marker, and parks the same Task in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseAfterStageSlice {
    pub operation_id: Uuid,
    pub current_stage_execution_id: Uuid,
    pub next_stage_execution_id: Uuid,
    pub next_stage: StageKind,
}

/// Exact old/new identities returned by an atomic stage transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionedStageExecution {
    pub previous: StageExecution,
    pub current: StageExecution,
}

/// Compare-and-swap command for a projected DAG terminal. The task and its
/// exact active stage execution must become terminal in one repository commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteTerminalStageExecution {
    pub operation_id: Uuid,
    pub current_stage_execution_id: Uuid,
    pub terminal_stage: StageKind,
    pub task_result: String,
}

/// 把 subtask 队列按 harness stage 分组（按 stage **首次出现**的顺序）。
///
/// 返回 `(stage, 属于该 stage 的 subtask 在原队列中的下标)`，下标按队列顺序。无 stage
/// 标签的 subtask 不入任何组（Executor 驱动只跑挂在某 stage 上的活；untagged subtask
/// 走旧路径或不参与 stage 图）。同一 stage 的 subtask 即使在队列里不连续，也会归到一组。
pub fn group_subtasks_by_stage(queue: &[PlannedSubtask]) -> Vec<(StageKind, Vec<usize>)> {
    let mut order: Vec<StageKind> = Vec::new();
    let mut groups: HashMap<StageKind, Vec<usize>> = HashMap::new();
    for (i, subtask) in queue.iter().enumerate() {
        let Some(hint) = subtask.harness_stage.as_ref() else {
            continue;
        };
        let stage = hint.stage_kind;
        if !groups.contains_key(&stage) {
            order.push(stage);
        }
        groups.entry(stage).or_default().push(i);
    }
    order
        .into_iter()
        .map(|stage| {
            let indices = groups.remove(&stage).unwrap_or_default();
            (stage, indices)
        })
        .collect()
}

/// C-4 · DB-backed [`Checkpointer`] for the Executor-driven flow.
///
/// Legacy/dual contracts persist [`OperationFlowState`] (+ next node) into
/// `operation_state.state_blob` under a `"graph_flow"` key. V2-only operations
/// never write that legacy authority: save is a no-op and load reconstructs a
/// fresh flow state at the relational `operation_state.current_stage` cursor.
/// Errors map to [`GraphError::Checkpoint`].
pub struct DbFlowCheckpointer {
    repo: Arc<dyn DbRepoProvider>,
    operation_id: Uuid,
    selected_resume_source: Option<crate::db_traits::RuntimeMemoryRecordSource>,
}

impl DbFlowCheckpointer {
    pub fn new(repo: Arc<dyn DbRepoProvider>, operation_id: Uuid) -> Self {
        Self {
            repo,
            operation_id,
            selected_resume_source: None,
        }
    }

    /// Pin a resume to the complete source already selected by the trusted
    /// caller. This is intentionally distinct from the operation's rollout
    /// contract: `DualWriteV2Preferred` may select either V2 or one complete
    /// legacy fallback, and the checkpointer must not make that decision again.
    pub fn with_selected_resume_source(
        mut self,
        source: crate::db_traits::RuntimeMemoryRecordSource,
    ) -> Self {
        self.selected_resume_source = Some(source);
        self
    }
}

fn resume_uses_relational_cursor(
    contract: crate::runtime_memory::RuntimeMemoryContract,
    selected: Option<crate::db_traits::RuntimeMemoryRecordSource>,
) -> GraphResult<bool> {
    use crate::db_traits::RuntimeMemoryRecordSource as Source;
    use crate::runtime_memory::RuntimeMemoryContract as Contract;

    match (contract, selected) {
        (Contract::V2Only, None | Some(Source::V2))
        | (Contract::DualWriteV2Preferred, Some(Source::V2)) => Ok(true),
        (Contract::LegacyV1 | Contract::DualWriteLegacyRead, None | Some(Source::Legacy))
        | (Contract::DualWriteV2Preferred, None | Some(Source::LegacyFallback)) => Ok(false),
        (contract, Some(source)) => Err(GraphError::Checkpoint(format!(
            "selected runtime-memory source {source:?} is invalid for frozen contract {contract}"
        ))),
    }
}

#[async_trait]
impl Checkpointer<OperationFlowState> for DbFlowCheckpointer {
    async fn save(
        &self,
        _thread_id: &str,
        state: &OperationFlowState,
        next_node: &str,
    ) -> GraphResult<()> {
        let view = crate::db_shim::operation_state::get(&*self.repo, self.operation_id)
            .await
            .map_err(|e| GraphError::Checkpoint(e.to_string()))?;
        if let Some(view) = view.as_ref() {
            if resume_uses_relational_cursor(
                view.runtime_memory_contract,
                self.selected_resume_source,
            )? {
                return Ok(());
            }
        } else if self.selected_resume_source.is_some() {
            return Err(GraphError::Checkpoint(
                "selected resume operation_state is missing".to_string(),
            ));
        }
        let existing = view.map(|view| view.state_blob).unwrap_or_default();
        let blob = state_blob_with_graph_flow(existing, state, next_node);
        crate::db_shim::operation_state::write_state_blob(&*self.repo, self.operation_id, blob)
            .await
            .map_err(|e| GraphError::Checkpoint(e.to_string()))?;
        Ok(())
    }

    async fn load(&self, _thread_id: &str) -> GraphResult<Option<(OperationFlowState, String)>> {
        let view = crate::db_shim::operation_state::get(&*self.repo, self.operation_id)
            .await
            .map_err(|e| GraphError::Checkpoint(e.to_string()))?;
        let Some(view) = view else {
            return Ok(None);
        };
        if resume_uses_relational_cursor(view.runtime_memory_contract, self.selected_resume_source)?
        {
            return Ok(Some((OperationFlowState::default(), view.current_stage)));
        }
        let Some(gf) = view.state_blob.get("graph_flow") else {
            return Ok(None);
        };
        let state: OperationFlowState =
            serde_json::from_value(gf.get("state").cloned().unwrap_or_default())
                .map_err(|e| GraphError::Checkpoint(e.to_string()))?;
        let next_node = gf
            .get("next_node")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Some((state, next_node)))
    }
}

fn state_blob_with_graph_flow(
    mut existing: serde_json::Value,
    state: &OperationFlowState,
    next_node: &str,
) -> serde_json::Value {
    if !existing.is_object() {
        existing = serde_json::json!({});
    }
    existing.as_object_mut().unwrap().insert(
        "graph_flow".to_string(),
        serde_json::json!({ "state": state, "next_node": next_node }),
    );
    existing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_traits::RuntimeMemoryRecordSource as Source;
    use crate::harness::HarnessStageHint;
    use crate::runtime_memory::RuntimeMemoryContract as Contract;

    fn sub(stage: Option<StageKind>) -> PlannedSubtask {
        PlannedSubtask {
            title: "t".to_string(),
            description: "d".to_string(),
            agent: None,
            harness_stage: stage.map(HarnessStageHint::new),
            nl_slice: None,
            acceptance_criteria: Vec::new(),
        }
    }

    #[test]
    fn groups_by_stage_in_first_appearance_order_incl_noncontiguous() {
        let q = vec![
            sub(Some(StageKind::ExternalAttackSurface)),
            sub(Some(StageKind::ExternalAttackSurface)),
            sub(Some(StageKind::Enumeration)),
            sub(Some(StageKind::ExternalAttackSurface)), // non-contiguous, same stage
        ];
        assert_eq!(
            group_subtasks_by_stage(&q),
            vec![
                (StageKind::ExternalAttackSurface, vec![0, 1, 3]),
                (StageKind::Enumeration, vec![2]),
            ]
        );
    }

    #[test]
    fn untagged_subtasks_are_excluded() {
        let q = vec![sub(None), sub(Some(StageKind::Scoping)), sub(None)];
        assert_eq!(
            group_subtasks_by_stage(&q),
            vec![(StageKind::Scoping, vec![1])]
        );
    }

    #[test]
    fn empty_or_all_untagged_yields_no_groups() {
        assert!(group_subtasks_by_stage(&[]).is_empty());
        assert!(group_subtasks_by_stage(&[sub(None), sub(None)]).is_empty());
    }

    #[test]
    fn graph_flow_checkpoint_preserves_other_state_blob_keys() {
        let mut flow = OperationFlowState::default();
        flow.visited.push(StageKind::Scoping);
        let existing = serde_json::json!({
            "stage_run_workers": {
                "target_intel": {
                    "11111111-1111-1111-1111-111111111111": {
                        "chain_id": "22222222-2222-2222-2222-222222222222"
                    }
                }
            }
        });

        let merged = state_blob_with_graph_flow(existing, &flow, StageKind::TargetIntel.as_str());

        assert_eq!(
            merged["stage_run_workers"]["target_intel"]["11111111-1111-1111-1111-111111111111"]
                ["chain_id"],
            "22222222-2222-2222-2222-222222222222"
        );
        assert_eq!(merged["graph_flow"]["next_node"], "target_intel");
    }

    #[test]
    fn typed_stage_execution_preserves_exact_operation_stage_and_status() {
        let execution = StageExecution {
            id: Uuid::from_u128(0xa01),
            operation_id: Uuid::from_u128(0xa02),
            stage: StageKind::TargetIntel,
            status: StageExecutionStatus::Started,
        };

        assert_eq!(execution.id, Uuid::from_u128(0xa01));
        assert_eq!(execution.operation_id, Uuid::from_u128(0xa02));
        assert_eq!(execution.stage, StageKind::TargetIntel);
        assert_eq!(execution.status, StageExecutionStatus::Started);
    }

    #[test]
    fn preferred_resume_obeys_the_selected_whole_record_source() {
        assert!(
            resume_uses_relational_cursor(Contract::DualWriteV2Preferred, Some(Source::V2))
                .expect("complete V2 source")
        );
        assert!(!resume_uses_relational_cursor(
            Contract::DualWriteV2Preferred,
            Some(Source::LegacyFallback)
        )
        .expect("complete legacy fallback"));
        assert!(resume_uses_relational_cursor(
            Contract::DualWriteV2Preferred,
            Some(Source::Legacy)
        )
        .is_err());
        assert!(
            resume_uses_relational_cursor(Contract::V2Only, Some(Source::LegacyFallback)).is_err()
        );
    }
}
