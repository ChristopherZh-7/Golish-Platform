//! [`PlanManager`] / [`TaskPlan`] tests, including proptests in a nested module.

use super::*;

// ========================================================================
// StepStatus Tests
// ========================================================================

#[test]
fn test_step_status_default() {
    let status = StepStatus::default();
    assert_eq!(status, StepStatus::Pending);
}

#[test]
fn test_step_status_display() {
    assert_eq!(format!("{}", StepStatus::Pending), "pending");
    assert_eq!(format!("{}", StepStatus::InProgress), "in_progress");
    assert_eq!(format!("{}", StepStatus::Completed), "completed");
}

#[test]
fn test_step_status_serialization() {
    assert_eq!(
        serde_json::to_string(&StepStatus::Pending).unwrap(),
        "\"pending\""
    );
    assert_eq!(
        serde_json::to_string(&StepStatus::InProgress).unwrap(),
        "\"in_progress\""
    );
    assert_eq!(
        serde_json::to_string(&StepStatus::Completed).unwrap(),
        "\"completed\""
    );
}

#[test]
fn test_step_status_deserialization() {
    assert_eq!(
        serde_json::from_str::<StepStatus>("\"pending\"").unwrap(),
        StepStatus::Pending
    );
    assert_eq!(
        serde_json::from_str::<StepStatus>("\"in_progress\"").unwrap(),
        StepStatus::InProgress
    );
    assert_eq!(
        serde_json::from_str::<StepStatus>("\"completed\"").unwrap(),
        StepStatus::Completed
    );
}

// ========================================================================
// PlanSummary Tests
// ========================================================================

#[test]
fn test_plan_summary_default() {
    let summary = PlanSummary::default();
    assert_eq!(summary.total, 0);
    assert_eq!(summary.completed, 0);
    assert_eq!(summary.in_progress, 0);
    assert_eq!(summary.pending, 0);
}

#[test]
fn test_plan_summary_from_empty_steps() {
    let summary = PlanSummary::from_steps(&[]);
    assert_eq!(summary.total, 0);
    assert_eq!(summary.completed, 0);
    assert_eq!(summary.in_progress, 0);
    assert_eq!(summary.pending, 0);
}

#[test]
fn test_plan_summary_from_mixed_steps() {
    let steps = vec![
        PlanStep {
            id: None,
            step: "Step 1".to_string(),
            status: StepStatus::Completed,
            failure_kind: None,
        },
        PlanStep {
            id: None,
            step: "Step 2".to_string(),
            status: StepStatus::Completed,
            failure_kind: None,
        },
        PlanStep {
            id: None,
            step: "Step 3".to_string(),
            status: StepStatus::InProgress,
            failure_kind: None,
        },
        PlanStep {
            id: None,
            step: "Step 4".to_string(),
            status: StepStatus::Pending,
            failure_kind: None,
        },
        PlanStep {
            id: None,
            step: "Step 5".to_string(),
            status: StepStatus::Pending,
            failure_kind: None,
        },
    ];

    let summary = PlanSummary::from_steps(&steps);
    assert_eq!(summary.total, 5);
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.in_progress, 1);
    assert_eq!(summary.pending, 2);
}

#[test]
fn test_plan_summary_all_completed() {
    let steps = vec![
        PlanStep {
            id: None,
            step: "Done 1".to_string(),
            status: StepStatus::Completed,
            failure_kind: None,
        },
        PlanStep {
            id: None,
            step: "Done 2".to_string(),
            status: StepStatus::Completed,
            failure_kind: None,
        },
    ];

    let summary = PlanSummary::from_steps(&steps);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.in_progress, 0);
    assert_eq!(summary.pending, 0);
}

// ========================================================================
// TaskPlan Tests
// ========================================================================

#[test]
fn test_task_plan_default() {
    let plan = TaskPlan::default();
    assert!(plan.explanation.is_none());
    assert!(plan.steps.is_empty());
    assert_eq!(plan.version, 0);
    assert!(plan.is_empty());
}

#[test]
fn test_task_plan_is_empty() {
    let mut plan = TaskPlan::default();
    assert!(plan.is_empty());

    plan.steps.push(PlanStep {
        id: None,
        step: "Test".to_string(),
        status: StepStatus::Pending,
        failure_kind: None,
    });
    assert!(!plan.is_empty());
}

// ========================================================================
// PlanStep Serialization Tests
// ========================================================================

#[test]
fn test_plan_step_serialization() {
    let step = PlanStep {
        id: Some("abc-123".into()),
        step: "Read the file".to_string(),
        status: StepStatus::InProgress,
        failure_kind: None,
    };

    let json = serde_json::to_string(&step).unwrap();
    assert!(json.contains("\"step\":\"Read the file\""));
    assert!(json.contains("\"status\":\"in_progress\""));
    assert!(json.contains("\"id\":\"abc-123\""));
}

#[test]
fn test_plan_step_input_deserialization_with_status() {
    let json = r#"{"step": "Do something", "status": "completed"}"#;
    let input: PlanStepInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.step, "Do something");
    assert_eq!(input.status, StepStatus::Completed);
}

#[test]
fn test_plan_step_input_deserialization_without_status() {
    let json = r#"{"step": "Do something"}"#;
    let input: PlanStepInput = serde_json::from_str(json).unwrap();
    assert_eq!(input.step, "Do something");
    assert_eq!(input.status, StepStatus::Pending); // Default
}

// ========================================================================
// UpdatePlanArgs Deserialization Tests
// ========================================================================

#[test]
fn test_update_plan_args_full() {
    let json = r#"{
        "explanation": "My plan",
        "plan": [
            {"step": "Step 1", "status": "completed"},
            {"step": "Step 2", "status": "in_progress"},
            {"step": "Step 3"}
        ]
    }"#;

    let args: UpdatePlanArgs = serde_json::from_str(json).unwrap();
    assert_eq!(args.explanation, Some("My plan".to_string()));
    assert_eq!(args.plan.len(), 3);
    assert_eq!(args.plan[0].status, StepStatus::Completed);
    assert_eq!(args.plan[1].status, StepStatus::InProgress);
    assert_eq!(args.plan[2].status, StepStatus::Pending);
}

#[test]
fn test_update_plan_args_minimal() {
    let json = r#"{"plan": [{"step": "Only step"}]}"#;

    let args: UpdatePlanArgs = serde_json::from_str(json).unwrap();
    assert!(args.explanation.is_none());
    assert_eq!(args.plan.len(), 1);
}

// ========================================================================
// PlanError Tests
// ========================================================================

#[test]
fn test_plan_error_display() {
    let err = PlanError::InvalidStepCount(15);
    assert!(err.to_string().contains("15"));
    assert!(err.to_string().contains("1"));
    assert!(err.to_string().contains("12"));

    let err = PlanError::EmptyStepDescription(3);
    assert!(err.to_string().contains("Step 3"));
    assert!(err.to_string().contains("empty"));

    let err = PlanError::MultipleInProgress(2);
    assert!(err.to_string().contains("2"));
    assert!(err.to_string().contains("one"));
}

// ========================================================================
// PlanManager Unit Tests
// ========================================================================

mod manager_tests;

// ========================================================================
// Patch Ops Tests (P0-2 stage 2)
// ========================================================================

mod patch_tests;

// ========================================================================
// Property-Based Tests
// ========================================================================

mod property_tests;
