use super::*;

/// Test serialization of PlanUpdated event with all fields
#[test]
fn test_plan_updated_with_explanation_serialization() {
    let event = AiEvent::PlanUpdated {
        version: 2,
        summary: PlanSummary {
            total: 4,
            completed: 1,
            in_progress: 1,
            pending: 2,
        },
        steps: vec![
            PlanStep {
                id: Some("step-1".into()),
                step: "Analyze the codebase".to_string(),
                status: StepStatus::Completed,
                failure_kind: None,
            },
            PlanStep {
                id: Some("step-2".into()),
                step: "Implement the feature".to_string(),
                status: StepStatus::InProgress,
                failure_kind: None,
            },
            PlanStep {
                id: Some("step-3".into()),
                step: "Write tests".to_string(),
                status: StepStatus::Pending,
                failure_kind: None,
            },
            PlanStep {
                id: Some("step-4".into()),
                step: "Update documentation".to_string(),
                status: StepStatus::Pending,
                failure_kind: None,
            },
        ],
        explanation: Some("Updated plan based on code analysis results".to_string()),
        stage_id: None,
    };
    let json = serde_json::to_value(&event).unwrap();
    insta::assert_json_snapshot!(json);
}

/// Test serialization of PlanUpdated event without explanation
#[test]
fn test_plan_updated_without_explanation_serialization() {
    let event = AiEvent::PlanUpdated {
        version: 1,
        summary: PlanSummary {
            total: 2,
            completed: 0,
            in_progress: 0,
            pending: 2,
        },
        steps: vec![
            PlanStep {
                id: Some("step-a".into()),
                step: "Research the problem".to_string(),
                status: StepStatus::Pending,
                failure_kind: None,
            },
            PlanStep {
                id: Some("step-b".into()),
                step: "Implement solution".to_string(),
                status: StepStatus::Pending,
                failure_kind: None,
            },
        ],
        explanation: None,
        stage_id: None,
    };
    let json = serde_json::to_value(&event).unwrap();
    insta::assert_json_snapshot!(json);
}

// ============================================================================
// ToolSource Serialization Tests
// ============================================================================

/// Test serialization of ToolSource::Main
#[test]
fn test_tool_source_main_serialization() {
    let source = ToolSource::Main;
    let json = serde_json::to_value(&source).unwrap();
    insta::assert_json_snapshot!(json);
}

/// Test serialization of ToolSource::SubAgent
#[test]
fn test_tool_source_sub_agent_serialization() {
    let source = ToolSource::SubAgent {
        agent_id: "agent-001".to_string(),
        agent_name: "analyzer".to_string(),
    };
    let json = serde_json::to_value(&source).unwrap();
    insta::assert_json_snapshot!(json);
}

/// Test serialization of ToolSource::Workflow with step info
#[test]
fn test_tool_source_workflow_with_step_serialization() {
    let source = ToolSource::Workflow {
        workflow_id: "wf-001".to_string(),
        workflow_name: "git_commit".to_string(),
        step_name: Some("analyze".to_string()),
        step_index: Some(0),
    };
    let json = serde_json::to_value(&source).unwrap();
    insta::assert_json_snapshot!(json);
}

/// Test serialization of ToolSource::Workflow without step info
#[test]
fn test_tool_source_workflow_without_step_serialization() {
    let source = ToolSource::Workflow {
        workflow_id: "wf-001".to_string(),
        workflow_name: "git_commit".to_string(),
        step_name: None,
        step_index: None,
    };
    let json = serde_json::to_value(&source).unwrap();
    insta::assert_json_snapshot!(json);
}
