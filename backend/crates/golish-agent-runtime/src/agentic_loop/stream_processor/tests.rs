use serde_json::json;

use super::{has_complete_tool_args, initial_tool_args_fragment};

#[test]
fn string_tool_args_are_treated_as_streaming_fragments() {
    let args = json!("{\"command\":\"dig example.com A +short\", \"cwd\":");

    assert!(!has_complete_tool_args(&args));
    assert_eq!(
        initial_tool_args_fragment(&args),
        "{\"command\":\"dig example.com A +short\", \"cwd\":"
    );
}

#[test]
fn object_tool_args_are_complete() {
    let args = json!({"command": "dig example.com A +short", "cwd": ""});

    assert!(has_complete_tool_args(&args));
    assert_eq!(
        initial_tool_args_fragment(&args),
        "{\"command\":\"dig example.com A +short\",\"cwd\":\"\"}"
    );
}
