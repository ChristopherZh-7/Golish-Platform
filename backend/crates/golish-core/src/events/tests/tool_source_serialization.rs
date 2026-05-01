use super::*;

#[test]
fn main_source_json_format() {
    let source = ToolSource::Main;
    let json = serde_json::to_value(&source).unwrap();

    assert_eq!(json["type"], "main");
}

#[test]
fn sub_agent_source_json_format() {
    let source = ToolSource::SubAgent {
        agent_id: "agent-001".to_string(),
        agent_name: "analyzer".to_string(),
    };
    let json = serde_json::to_value(&source).unwrap();

    assert_eq!(json["type"], "sub_agent");
    assert_eq!(json["agent_id"], "agent-001");
    assert_eq!(json["agent_name"], "analyzer");
}

#[test]
fn workflow_source_json_format() {
    let source = ToolSource::Workflow {
        workflow_id: "wf-001".to_string(),
        workflow_name: "git_commit".to_string(),
        step_name: Some("analyze".to_string()),
        step_index: Some(0),
    };
    let json = serde_json::to_value(&source).unwrap();

    assert_eq!(json["type"], "workflow");
    assert_eq!(json["workflow_id"], "wf-001");
    assert_eq!(json["workflow_name"], "git_commit");
    assert_eq!(json["step_name"], "analyze");
    assert_eq!(json["step_index"], 0);
}

#[test]
fn workflow_source_without_step_json_format() {
    let source = ToolSource::Workflow {
        workflow_id: "wf-001".to_string(),
        workflow_name: "git_commit".to_string(),
        step_name: None,
        step_index: None,
    };
    let json = serde_json::to_value(&source).unwrap();

    assert_eq!(json["type"], "workflow");
    assert_eq!(json["workflow_id"], "wf-001");
    // step_name and step_index should be absent (skip_serializing_if)
    assert!(json.get("step_name").is_none());
    assert!(json.get("step_index").is_none());
}
