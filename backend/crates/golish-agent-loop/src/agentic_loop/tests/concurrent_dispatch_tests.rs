use super::*;
use crate::agentic_loop::sub_agent_dispatch::is_sub_agent_tool;

fn make_tool_call(name: &str, id: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        call_id: Some(id.to_string()),
        function: rig::message::ToolFunction {
            name: name.to_string(),
            arguments: json!({}),
        },
        signature: None,
        additional_params: None,
    }
}

#[test]
fn test_is_sub_agent_tool() {
    assert!(is_sub_agent_tool("sub_agent_coder"));
    assert!(is_sub_agent_tool("sub_agent_explorer"));
    assert!(!is_sub_agent_tool("read_file"));
    assert!(!is_sub_agent_tool("run_pty_cmd"));
}

#[test]
fn test_partition_tool_calls_mixed() {
    let calls = vec![
        make_tool_call("read_file", "tc1"),
        make_tool_call("sub_agent_coder", "tc2"),
        make_tool_call("write_file", "tc3"),
        make_tool_call("sub_agent_explorer", "tc4"),
    ];
    let (sub_agents, others) = partition_tool_calls(calls);
    assert_eq!(sub_agents.len(), 2);
    assert_eq!(others.len(), 2);
    assert_eq!(sub_agents[0].0, 1);
    assert_eq!(sub_agents[1].0, 3);
    assert_eq!(others[0].0, 0);
    assert_eq!(others[1].0, 2);
}

#[test]
fn test_partition_tool_calls_empty() {
    let (sub_agents, others) = partition_tool_calls(vec![]);
    assert_eq!(sub_agents.len(), 0);
    assert_eq!(others.len(), 0);
}
