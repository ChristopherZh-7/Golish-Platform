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
/// Persists the [`OperationFlowState`] (+ next node) into
/// `operation_state.state_blob` under a `"graph_flow"` key, so a crashed/killed
/// run can resume the metalcraft Executor mid-flow (finer than the legacy
/// stage-cursor resume). Errors map to [`GraphError::Checkpoint`].
pub struct DbFlowCheckpointer {
    repo: Arc<dyn DbRepoProvider>,
    operation_id: Uuid,
}

impl DbFlowCheckpointer {
    pub fn new(repo: Arc<dyn DbRepoProvider>, operation_id: Uuid) -> Self {
        Self { repo, operation_id }
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
        let existing = crate::db_shim::operation_state::get(&*self.repo, self.operation_id)
            .await
            .map_err(|e| GraphError::Checkpoint(e.to_string()))?
            .map(|view| view.state_blob)
            .unwrap_or_default();
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
    use crate::harness::HarnessStageHint;

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
}
