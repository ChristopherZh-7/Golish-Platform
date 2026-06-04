//! Unit tests for `PlanManager::apply_patch_ops` (P0-2 stage 2).
//!
//! See docs/design/2026-05-17-refiner-patch-protocol.md.

use super::*;
use crate::planner::PlanPatchOp;

async fn make_plan_with_steps(titles: &[&str]) -> PlanManager {
    let manager = PlanManager::new();
    let args = UpdatePlanArgs {
        explanation: Some("seed".to_string()),
        plan: titles
            .iter()
            .map(|t| PlanStepInput {
                step: t.to_string(),
                status: StepStatus::Pending,
            })
            .collect(),
    };
    manager
        .update_plan(args, None)
        .await
        .expect("seed update_plan");
    manager
}

#[tokio::test]
async fn add_at_beginning_when_after_id_is_none() {
    let manager = make_plan_with_steps(&["A", "B"]).await;
    let ops = vec![PlanPatchOp::Add {
        after_id: None,
        title: "Z".to_string(),
        status: None,
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 3);
    assert_eq!(updated.steps[0].step, "Z");
    assert_eq!(updated.steps[1].step, "A");
}

#[tokio::test]
async fn add_after_existing_step_inserts_in_order() {
    let manager = make_plan_with_steps(&["A", "B", "C"]).await;
    let snapshot = manager.snapshot().await;
    let b_id = snapshot.steps[1].id.clone().unwrap();

    let ops = vec![PlanPatchOp::Add {
        after_id: Some(b_id),
        title: "B+".to_string(),
        status: Some(StepStatus::InProgress),
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 4);
    assert_eq!(updated.steps[0].step, "A");
    assert_eq!(updated.steps[1].step, "B");
    assert_eq!(updated.steps[2].step, "B+");
    assert_eq!(updated.steps[2].status, StepStatus::InProgress);
    assert_eq!(updated.steps[3].step, "C");
}

#[tokio::test]
async fn add_with_unknown_after_id_appends_to_end() {
    let manager = make_plan_with_steps(&["A", "B"]).await;
    let ops = vec![PlanPatchOp::Add {
        after_id: Some("does-not-exist".to_string()),
        title: "Tail".to_string(),
        status: None,
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 3);
    assert_eq!(updated.steps.last().unwrap().step, "Tail");
}

#[tokio::test]
async fn add_with_whitespace_only_title_is_noop() {
    let manager = make_plan_with_steps(&["A"]).await;
    let ops = vec![PlanPatchOp::Add {
        after_id: None,
        title: "   \n\t  ".to_string(),
        status: None,
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 1);
    assert_eq!(updated.steps[0].step, "A");
}

#[tokio::test]
async fn remove_existing_step() {
    let manager = make_plan_with_steps(&["A", "B", "C"]).await;
    let snapshot = manager.snapshot().await;
    let b_id = snapshot.steps[1].id.clone().unwrap();
    let ops = vec![PlanPatchOp::Remove { id: b_id }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 2);
    assert_eq!(updated.steps[0].step, "A");
    assert_eq!(updated.steps[1].step, "C");
}

#[tokio::test]
async fn remove_nonexistent_step_is_noop() {
    let manager = make_plan_with_steps(&["A"]).await;
    let ops = vec![PlanPatchOp::Remove {
        id: "ghost".to_string(),
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps.len(), 1);
    assert_eq!(updated.steps[0].step, "A");
}

#[tokio::test]
async fn modify_updates_title_status_and_failure_kind() {
    let manager = make_plan_with_steps(&["Old"]).await;
    let snapshot = manager.snapshot().await;
    let id = snapshot.steps[0].id.clone().unwrap();

    let ops = vec![PlanPatchOp::Modify {
        id,
        title: Some("New title".to_string()),
        status: Some(StepStatus::Failed),
        failure_kind: Some(FailureKind::Conceptual),
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps[0].step, "New title");
    assert_eq!(updated.steps[0].status, StepStatus::Failed);
    assert_eq!(updated.steps[0].failure_kind, Some(FailureKind::Conceptual));
}

#[tokio::test]
async fn modify_with_unknown_id_is_noop() {
    let manager = make_plan_with_steps(&["A"]).await;
    let ops = vec![PlanPatchOp::Modify {
        id: "ghost".to_string(),
        title: Some("X".to_string()),
        status: None,
        failure_kind: None,
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps[0].step, "A");
}

#[tokio::test]
async fn reorder_moves_step_to_head_when_after_id_is_none() {
    let manager = make_plan_with_steps(&["A", "B", "C"]).await;
    let snapshot = manager.snapshot().await;
    let c_id = snapshot.steps[2].id.clone().unwrap();
    let ops = vec![PlanPatchOp::Reorder {
        id: c_id,
        after_id: None,
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps[0].step, "C");
    assert_eq!(updated.steps[1].step, "A");
    assert_eq!(updated.steps[2].step, "B");
}

#[tokio::test]
async fn reorder_after_existing_step() {
    let manager = make_plan_with_steps(&["A", "B", "C"]).await;
    let snapshot = manager.snapshot().await;
    let a_id = snapshot.steps[0].id.clone().unwrap();
    let c_id = snapshot.steps[2].id.clone().unwrap();
    let ops = vec![PlanPatchOp::Reorder {
        id: a_id,
        after_id: Some(c_id),
    }];
    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    assert_eq!(updated.steps[0].step, "B");
    assert_eq!(updated.steps[1].step, "C");
    assert_eq!(updated.steps[2].step, "A");
}

#[tokio::test]
async fn over_max_steps_returns_invalid_step_count() {
    let manager = make_plan_with_steps(&["A"]).await;
    let ops: Vec<PlanPatchOp> = (0..MAX_PLAN_STEPS)
        .map(|i| PlanPatchOp::Add {
            after_id: None,
            title: format!("Extra {}", i),
            status: None,
        })
        .collect();
    let res = manager.apply_patch_ops(ops, None).await;
    assert!(matches!(res, Err(PlanError::InvalidStepCount(_))));
}

#[tokio::test]
async fn multiple_in_progress_returns_error() {
    let manager = make_plan_with_steps(&["A", "B"]).await;
    let snapshot = manager.snapshot().await;
    let a = snapshot.steps[0].id.clone().unwrap();
    let b = snapshot.steps[1].id.clone().unwrap();
    let ops = vec![
        PlanPatchOp::Modify {
            id: a,
            title: None,
            status: Some(StepStatus::InProgress),
            failure_kind: None,
        },
        PlanPatchOp::Modify {
            id: b,
            title: None,
            status: Some(StepStatus::InProgress),
            failure_kind: None,
        },
    ];
    let res = manager.apply_patch_ops(ops, None).await;
    assert!(matches!(res, Err(PlanError::MultipleInProgress(2))));
}

#[tokio::test]
async fn composite_ops_remove_add_modify_applied_in_order() {
    let manager = make_plan_with_steps(&["A", "B", "C"]).await;
    let snapshot = manager.snapshot().await;
    let a = snapshot.steps[0].id.clone().unwrap();
    let b = snapshot.steps[1].id.clone().unwrap();

    let ops = vec![
        PlanPatchOp::Remove { id: b }, // drop middle
        PlanPatchOp::Add {
            after_id: Some(a.clone()),
            title: "B-prime".to_string(),
            status: None,
        },
        PlanPatchOp::Modify {
            id: a,
            title: None,
            status: Some(StepStatus::Completed),
            failure_kind: None,
        },
    ];

    let updated = manager.apply_patch_ops(ops, None).await.unwrap();
    let titles: Vec<&str> = updated.steps.iter().map(|s| s.step.as_str()).collect();
    assert_eq!(titles, vec!["A", "B-prime", "C"]);
    assert_eq!(updated.steps[0].status, StepStatus::Completed);
}

#[tokio::test]
async fn version_increments_on_each_apply() {
    let manager = make_plan_with_steps(&["A"]).await;
    let initial = manager.snapshot().await.version;
    let _ = manager
        .apply_patch_ops(
            vec![PlanPatchOp::Add {
                after_id: None,
                title: "Z".to_string(),
                status: None,
            }],
            None,
        )
        .await
        .unwrap();
    let after = manager.snapshot().await.version;
    assert_eq!(after, initial + 1);
}
