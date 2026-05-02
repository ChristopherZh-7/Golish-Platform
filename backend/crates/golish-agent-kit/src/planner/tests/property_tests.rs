use super::*;
use proptest::prelude::*;

/// Strategy for generating a valid step status
fn status_strategy() -> impl Strategy<Value = StepStatus> {
    prop_oneof![
        Just(StepStatus::Pending),
        Just(StepStatus::InProgress),
        Just(StepStatus::Completed),
    ]
}

/// Strategy for generating a non-empty step description
fn step_description_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ]{1,50}"
        .prop_filter("must not be empty after trim", |s| !s.trim().is_empty())
}

/// Strategy for generating a valid plan step input
fn plan_step_input_strategy() -> impl Strategy<Value = PlanStepInput> {
    (step_description_strategy(), status_strategy())
        .prop_map(|(step, status)| PlanStepInput { step, status })
}

/// Strategy for generating a plan with valid step count (1-12)
fn valid_plan_strategy() -> impl Strategy<Value = Vec<PlanStepInput>> {
    prop::collection::vec(plan_step_input_strategy(), 1..=12)
}

proptest! {
    /// Property: Summary counts always sum to total
    #[test]
    fn summary_counts_sum_to_total(steps in valid_plan_strategy()) {
        let plan_steps: Vec<PlanStep> = steps
            .into_iter()
            .map(|input| PlanStep {
                id: None,
                step: input.step,
                status: input.status,
            })
            .collect();

        let summary = PlanSummary::from_steps(&plan_steps);

        prop_assert_eq!(
            summary.completed + summary.in_progress + summary.pending,
            summary.total,
            "Summary counts don't sum to total"
        );
    }

    /// Property: Summary total equals step count
    #[test]
    fn summary_total_equals_step_count(steps in valid_plan_strategy()) {
        let plan_steps: Vec<PlanStep> = steps
            .into_iter()
            .map(|input| PlanStep {
                id: None,
                step: input.step,
                status: input.status,
            })
            .collect();

        let summary = PlanSummary::from_steps(&plan_steps);

        prop_assert_eq!(
            summary.total,
            plan_steps.len(),
            "Summary total doesn't equal step count"
        );
    }

    /// Property: Step status serialization round-trips correctly
    #[test]
    fn step_status_serialization_roundtrip(status in status_strategy()) {
        let json = serde_json::to_string(&status).unwrap();
        let parsed: StepStatus = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(status, parsed);
    }

    /// Property: PlanStep serialization round-trips correctly
    #[test]
    fn plan_step_serialization_roundtrip(
        description in step_description_strategy(),
        status in status_strategy()
    ) {
        let step = PlanStep {
            id: None,
            step: description,
            status,
        };

        let json = serde_json::to_string(&step).unwrap();
        let parsed: PlanStep = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(step.step, parsed.step);
        prop_assert_eq!(step.status, parsed.status);
        prop_assert_eq!(step.id, parsed.id);
    }

    /// Property: Valid plans always succeed
    #[test]
    fn valid_plans_succeed(
        steps in prop::collection::vec(plan_step_input_strategy(), 1..=12)
            .prop_filter("at most one in_progress", |steps| {
                steps.iter().filter(|s| s.status == StepStatus::InProgress).count() <= 1
            })
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let args = UpdatePlanArgs {
                explanation: None,
                plan: steps,
            };

            let result = manager.update_plan(args).await;
            prop_assert!(result.is_ok(), "Valid plan should succeed: {:?}", result);
            Ok(())
        })?;
    }

    /// Property: Invalid step counts always fail
    #[test]
    fn invalid_step_count_fails(count in (13usize..100)) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let steps: Vec<PlanStepInput> = (0..count)
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
            prop_assert!(matches!(result, Err(PlanError::InvalidStepCount(_))));
            Ok(())
        })?;
    }

    /// Property: Multiple in_progress always fails
    #[test]
    fn multiple_in_progress_fails(extra_in_progress in 2usize..5) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let steps: Vec<PlanStepInput> = (0..extra_in_progress)
                .map(|i| PlanStepInput {
                    step: format!("In progress step {}", i),
                    status: StepStatus::InProgress,
                })
                .collect();

            let args = UpdatePlanArgs {
                explanation: None,
                plan: steps,
            };

            let result = manager.update_plan(args).await;
            prop_assert!(matches!(result, Err(PlanError::MultipleInProgress(_))));
            Ok(())
        })?;
    }

    /// Property: Version always increments on successful update
    #[test]
    fn version_increments(num_updates in 1usize..10) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();

            for expected_version in 1..=num_updates {
                let args = UpdatePlanArgs {
                    explanation: None,
                    plan: vec![PlanStepInput {
                        step: format!("Step for update {}", expected_version),
                        status: StepStatus::Pending,
                    }],
                };

                let plan = manager.update_plan(args).await.unwrap();
                prop_assert_eq!(
                    plan.version as usize,
                    expected_version,
                    "Version should be {} but was {}",
                    expected_version,
                    plan.version
                );
            }
            Ok(())
        })?;
    }

    /// Property: Whitespace is trimmed from step descriptions
    #[test]
    fn whitespace_is_trimmed(
        prefix_spaces in "[ \\t]{0,5}",
        content in "[a-zA-Z0-9]{1,20}",
        suffix_spaces in "[ \\t]{0,5}"
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let step_with_whitespace = format!("{}{}{}", prefix_spaces, content, suffix_spaces);

            let args = UpdatePlanArgs {
                explanation: None,
                plan: vec![PlanStepInput {
                    step: step_with_whitespace,
                    status: StepStatus::Pending,
                }],
            };

            let plan = manager.update_plan(args).await.unwrap();
            prop_assert_eq!(
                &plan.steps[0].step,
                &content,
                "Step description should be trimmed"
            );
            Ok(())
        })?;
    }

    /// Property: Explanation whitespace is trimmed
    #[test]
    fn explanation_whitespace_is_trimmed(
        prefix_spaces in "[ \\t]{0,5}",
        content in "[a-zA-Z0-9]{1,20}",
        suffix_spaces in "[ \\t]{0,5}"
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let explanation_with_whitespace = format!("{}{}{}", prefix_spaces, content, suffix_spaces);

            let args = UpdatePlanArgs {
                explanation: Some(explanation_with_whitespace),
                plan: vec![PlanStepInput {
                    step: "Test step".to_string(),
                    status: StepStatus::Pending,
                }],
            };

            let plan = manager.update_plan(args).await.unwrap();
            prop_assert_eq!(
                plan.explanation,
                Some(content),
                "Explanation should be trimmed"
            );
            Ok(())
        })?;
    }

    /// Property: Clear resets plan to default state
    #[test]
    fn clear_resets_to_default(steps in valid_plan_strategy()) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();

            // First update with some data
            let in_progress_count = steps.iter().filter(|s| s.status == StepStatus::InProgress).count();
            if in_progress_count <= 1 {
                let args = UpdatePlanArgs {
                    explanation: Some("Will be cleared".to_string()),
                    plan: steps,
                };
                let _ = manager.update_plan(args).await;
            }

            // Clear
            manager.clear().await;

            // Verify default state
            let snapshot = manager.snapshot().await;
            prop_assert!(snapshot.is_empty());
            prop_assert!(snapshot.explanation.is_none());
            prop_assert_eq!(snapshot.version, 0);
            Ok(())
        })?;
    }

    /// Property: Snapshot returns consistent data
    #[test]
    fn snapshot_is_consistent(
        steps in prop::collection::vec(plan_step_input_strategy(), 1..=12)
            .prop_filter("at most one in_progress", |steps| {
                steps.iter().filter(|s| s.status == StepStatus::InProgress).count() <= 1
            }),
        explanation in prop::option::of("[a-zA-Z0-9 ]{1,30}")
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let manager = PlanManager::new();
            let step_count = steps.len();

            let args = UpdatePlanArgs {
                explanation: explanation.clone(),
                plan: steps,
            };

            manager.update_plan(args).await.unwrap();

            let snapshot1 = manager.snapshot().await;
            let snapshot2 = manager.snapshot().await;

            // Snapshots should be equal
            prop_assert_eq!(snapshot1.steps.len(), snapshot2.steps.len());
            prop_assert_eq!(snapshot1.version, snapshot2.version);
            prop_assert_eq!(snapshot1.explanation, snapshot2.explanation);
            prop_assert_eq!(snapshot1.steps.len(), step_count);

            Ok(())
        })?;
    }
}
