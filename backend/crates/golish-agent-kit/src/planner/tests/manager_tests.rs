use super::*;

#[tokio::test]
async fn test_plan_manager_new_is_empty() {
    let manager = PlanManager::new();
    assert!(manager.is_empty().await);
}

#[tokio::test]
async fn test_plan_manager_default_is_empty() {
    let manager = PlanManager::default();
    assert!(manager.is_empty().await);
}

#[tokio::test]
async fn test_plan_manager_update() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Test plan".to_string()),
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::Completed,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 3".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let plan = manager.update_plan(args).await.unwrap();

    assert_eq!(plan.version, 1);
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.summary.completed, 1);
    assert_eq!(plan.summary.in_progress, 1);
    assert_eq!(plan.summary.pending, 1);
    assert_eq!(plan.explanation, Some("Test plan".to_string()));
}

#[tokio::test]
async fn test_plan_manager_version_increments() {
    let manager = PlanManager::new();

    for i in 1..=5 {
        let args = UpdatePlanArgs {
            explanation: None,
            plan: vec![PlanStepInput {
                step: format!("Step version {}", i),
                status: StepStatus::Pending,
            }],
        };

        let plan = manager.update_plan(args).await.unwrap();
        assert_eq!(plan.version, i);
    }
}

#[tokio::test]
async fn test_plan_manager_snapshot() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Snapshot test".to_string()),
        plan: vec![PlanStepInput {
            step: "Test step".to_string(),
            status: StepStatus::Pending,
        }],
    };

    manager.update_plan(args).await.unwrap();

    let snapshot = manager.snapshot().await;
    assert_eq!(snapshot.explanation, Some("Snapshot test".to_string()));
    assert_eq!(snapshot.steps.len(), 1);
    assert_eq!(snapshot.version, 1);
}

#[tokio::test]
async fn test_plan_manager_clear() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Will be cleared".to_string()),
        plan: vec![PlanStepInput {
            step: "Step".to_string(),
            status: StepStatus::InProgress,
        }],
    };

    manager.update_plan(args).await.unwrap();
    assert!(!manager.is_empty().await);

    manager.clear().await;
    assert!(manager.is_empty().await);

    let snapshot = manager.snapshot().await;
    assert!(snapshot.explanation.is_none());
    assert!(snapshot.steps.is_empty());
    // Version is reset on clear
    assert_eq!(snapshot.version, 0);
}

#[tokio::test]
async fn test_plan_manager_trims_whitespace() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("  Trimmed explanation  ".to_string()),
        plan: vec![PlanStepInput {
            step: "  Trimmed step  ".to_string(),
            status: StepStatus::Pending,
        }],
    };

    let plan = manager.update_plan(args).await.unwrap();
    assert_eq!(plan.explanation, Some("Trimmed explanation".to_string()));
    assert_eq!(plan.steps[0].step, "Trimmed step");
}

#[tokio::test]
async fn test_plan_manager_rejects_empty_steps() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![PlanStepInput {
            step: "  ".to_string(), // Empty after trim
            status: StepStatus::Pending,
        }],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(1))));
}

#[tokio::test]
async fn test_plan_manager_rejects_multiple_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::InProgress,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::MultipleInProgress(2))));
}

#[tokio::test]
async fn test_plan_manager_allows_zero_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::Completed,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_plan_manager_allows_one_in_progress() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Step 1".to_string(),
                status: StepStatus::InProgress,
            },
            PlanStepInput {
                step: "Step 2".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };

    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().summary.in_progress, 1);
}

#[tokio::test]
async fn test_plan_manager_rejects_too_many_steps() {
    let manager = PlanManager::new();

    let steps: Vec<PlanStepInput> = (0..15)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(15))));
}

#[tokio::test]
async fn test_plan_manager_rejects_zero_steps() {
    let manager = PlanManager::new();

    let args = UpdatePlanArgs {
        explanation: Some("Empty plan".to_string()),
        plan: vec![],
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(0))));
}

#[tokio::test]
async fn test_plan_manager_accepts_boundary_step_counts() {
    let manager = PlanManager::new();

    // Test minimum (1 step)
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![PlanStepInput {
            step: "Single step".to_string(),
            status: StepStatus::Pending,
        }],
    };
    assert!(manager.update_plan(args).await.is_ok());

    // Test maximum (12 steps)
    let steps: Vec<PlanStepInput> = (0..12)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i + 1),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };
    let result = manager.update_plan(args).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().steps.len(), 12);
}

#[tokio::test]
async fn test_plan_manager_rejects_just_over_max() {
    let manager = PlanManager::new();

    // Test 13 steps (just over max)
    let steps: Vec<PlanStepInput> = (0..13)
        .map(|i| PlanStepInput {
            step: format!("Step {}", i + 1),
            status: StepStatus::Pending,
        })
        .collect();

    let args = UpdatePlanArgs {
        explanation: None,
        plan: steps,
    };

    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::InvalidStepCount(13))));
}

#[tokio::test]
async fn test_plan_manager_empty_description_at_various_positions() {
    let manager = PlanManager::new();

    // Empty at position 1
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "".to_string(),
                status: StepStatus::Pending,
            },
            PlanStepInput {
                step: "Valid".to_string(),
                status: StepStatus::Pending,
            },
        ],
    };
    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(1))));

    // Empty at position 2
    let args = UpdatePlanArgs {
        explanation: None,
        plan: vec![
            PlanStepInput {
                step: "Valid".to_string(),
                status: StepStatus::Pending,
            },
            PlanStepInput {
                step: "\t\n".to_string(), // Whitespace only
                status: StepStatus::Pending,
            },
        ],
    };
    let result = manager.update_plan(args).await;
    assert!(matches!(result, Err(PlanError::EmptyStepDescription(2))));
}
