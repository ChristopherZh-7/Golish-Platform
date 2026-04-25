use rig::completion::ToolDefinition;
use serde_json::json;

use super::config::ToolConfig;
use super::definitions::get_sub_agent_tool_definitions;
use super::preset::ToolPreset;
use super::sanitize::sanitize_schema;
use super::selection::{
    filter_tools_by_allowed, get_standard_tool_definitions, get_tool_definitions_for_preset,
    get_tool_definitions_with_config,
};

#[test]
fn test_sanitize_schema_removes_any_of() {
    let schema = json!({
        "type": "object",
        "anyOf": [{"type": "string"}, {"type": "number"}],
        "properties": {
            "name": {"type": "string"}
        }
    });

    let sanitized = sanitize_schema(schema);

    assert!(sanitized.get("anyOf").is_none());
    assert!(sanitized.get("properties").is_some());
}

#[test]
fn test_sanitize_schema_handles_one_of_in_properties() {
    let schema = json!({
        "type": "object",
        "properties": {
            "value": {
                "oneOf": [
                    {"type": "string"},
                    {"type": "number"}
                ]
            }
        },
        "required": ["value"] // Make it required so type stays as-is
    });

    let sanitized = sanitize_schema(schema);

    let value_prop = sanitized
        .get("properties")
        .and_then(|p| p.get("value"))
        .unwrap();
    assert!(value_prop.get("oneOf").is_none());
    // Should have type from first oneOf option
    assert_eq!(
        value_prop.get("type").and_then(|t| t.as_str()),
        Some("string")
    );
}

#[test]
fn test_sanitize_schema_strict_mode_compatibility() {
    let schema = json!({
        "type": "object",
        "properties": {
            "file_path": {"type": "string", "description": "Required file path"},
            "line_start": {"type": "integer", "description": "Optional start line"},
            "line_end": {"type": "integer", "description": "Optional end line"}
        },
        "required": ["file_path"]
    });

    let sanitized = sanitize_schema(schema);

    assert_eq!(
        sanitized.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false))
    );

    let required = sanitized
        .get("required")
        .and_then(|r| r.as_array())
        .unwrap();
    assert!(required.contains(&json!("file_path")));
    assert!(required.contains(&json!("line_start")));
    assert!(required.contains(&json!("line_end")));

    let file_path = sanitized
        .get("properties")
        .and_then(|p| p.get("file_path"))
        .unwrap();
    assert_eq!(file_path.get("type"), Some(&json!("string")));

    let line_start = sanitized
        .get("properties")
        .and_then(|p| p.get("line_start"))
        .unwrap();
    let line_start_type = line_start.get("type").unwrap();
    assert!(line_start_type.is_array());
    let type_arr = line_start_type.as_array().unwrap();
    assert!(type_arr.contains(&json!("integer")));
    assert!(type_arr.contains(&json!("null")));
}

#[test]
fn test_sanitize_schema_nested_objects() {
    let schema = json!({
        "type": "object",
        "properties": {
            "plan": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "status": {"type": "string"}
                    },
                    "required": ["task"]
                }
            }
        },
        "required": ["plan"]
    });

    let sanitized = sanitize_schema(schema);

    let items = sanitized
        .get("properties")
        .and_then(|p| p.get("plan"))
        .and_then(|p| p.get("items"))
        .unwrap();

    assert_eq!(
        items.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false))
    );

    let items_required = items.get("required").and_then(|r| r.as_array()).unwrap();
    assert!(items_required.contains(&json!("task")));
    assert!(items_required.contains(&json!("status")));

    let status = items
        .get("properties")
        .and_then(|p| p.get("status"))
        .unwrap();
    let status_type = status.get("type").unwrap();
    assert!(status_type.is_array());
    assert!(status_type.as_array().unwrap().contains(&json!("null")));
}

#[test]
fn test_get_standard_tool_definitions() {
    let tools = get_standard_tool_definitions();
    assert!(!tools.is_empty());
}

#[test]
fn test_filter_tools_by_allowed() {
    let tools = vec![
        ToolDefinition {
            name: "tool_a".to_string(),
            description: "A".to_string(),
            parameters: json!({}),
        },
        ToolDefinition {
            name: "tool_b".to_string(),
            description: "B".to_string(),
            parameters: json!({}),
        },
        ToolDefinition {
            name: "tool_c".to_string(),
            description: "C".to_string(),
            parameters: json!({}),
        },
    ];

    let allowed = vec!["tool_a".to_string(), "tool_c".to_string()];
    let filtered = filter_tools_by_allowed(tools, &allowed);

    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|t| t.name == "tool_a"));
    assert!(filtered.iter().any(|t| t.name == "tool_c"));
}

#[test]
fn test_filter_tools_empty_allowed() {
    let tools = vec![ToolDefinition {
        name: "tool_a".to_string(),
        description: "A".to_string(),
        parameters: json!({}),
    }];

    let filtered = filter_tools_by_allowed(tools.clone(), &[]);

    assert_eq!(filtered.len(), 1);
}

#[test]
fn test_tool_preset_minimal() {
    let preset = ToolPreset::Minimal;
    let names = preset.tool_names().unwrap();

    assert_eq!(names.len(), 4);
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"edit_file"));
    assert!(names.contains(&"write_file"));
    assert!(names.contains(&"run_pty_cmd"));
}

#[test]
fn test_tool_preset_standard() {
    let preset = ToolPreset::Standard;
    let names = preset.tool_names().unwrap();

    assert!(names.contains(&"grep_file"));
    assert!(names.contains(&"list_files"));
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"edit_file"));
    assert!(names.contains(&"run_pty_cmd"));
    assert!(names.contains(&"web_fetch"));

    assert!(!names.contains(&"save_skill"));
    assert!(!names.contains(&"create_pty_session"));
}

#[test]
fn test_tool_preset_full() {
    let preset = ToolPreset::Full;
    assert!(preset.tool_names().is_none());
}

#[test]
fn test_tool_config_default_is_standard() {
    let config = ToolConfig::default();
    assert_eq!(config.preset, ToolPreset::Standard);
}

#[test]
fn test_tool_config_is_tool_enabled() {
    let config = ToolConfig::with_preset(ToolPreset::Standard);

    assert!(config.is_tool_enabled("read_file"));
    assert!(config.is_tool_enabled("grep_file"));

    assert!(!config.is_tool_enabled("save_skill"));
    assert!(!config.is_tool_enabled("create_pty_session"));
}

#[test]
fn test_tool_config_additional_tools() {
    let config = ToolConfig {
        preset: ToolPreset::Minimal,
        additional: vec!["grep_file".to_string()],
        disabled: vec![],
    };

    assert!(config.is_tool_enabled("read_file"));
    assert!(config.is_tool_enabled("grep_file"));
    assert!(!config.is_tool_enabled("web_fetch"));
}

#[test]
fn test_tool_config_disabled_tools() {
    let config = ToolConfig {
        preset: ToolPreset::Standard,
        additional: vec![],
        disabled: vec!["delete_file".to_string()],
    };

    assert!(config.is_tool_enabled("read_file"));
    assert!(!config.is_tool_enabled("delete_file"));
}

#[test]
fn test_tool_config_disabled_overrides_additional() {
    let config = ToolConfig {
        preset: ToolPreset::Minimal,
        additional: vec!["grep_file".to_string()],
        disabled: vec!["grep_file".to_string()],
    };

    assert!(!config.is_tool_enabled("grep_file"));
}

#[test]
fn test_get_tool_definitions_for_preset_minimal() {
    let tools = get_tool_definitions_for_preset(ToolPreset::Minimal);
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    assert_eq!(tools.len(), 4);
    assert!(tool_names.contains(&"read_file"));
    assert!(tool_names.contains(&"edit_file"));
    assert!(tool_names.contains(&"write_file"));
    assert!(tool_names.contains(&"run_pty_cmd"));
}

#[test]
fn test_get_tool_definitions_for_preset_full() {
    let full_tools = get_tool_definitions_for_preset(ToolPreset::Full);
    let standard_tools = get_tool_definitions_for_preset(ToolPreset::Standard);

    assert!(full_tools.len() > standard_tools.len());
}

#[test]
fn test_tool_config_with_config() {
    let config = ToolConfig {
        preset: ToolPreset::Minimal,
        additional: vec!["grep_file".to_string(), "list_files".to_string()],
        disabled: vec![],
    };

    let tools = get_tool_definitions_with_config(&config);
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    assert_eq!(tools.len(), 6);
    assert!(tool_names.contains(&"read_file"));
    assert!(tool_names.contains(&"grep_file"));
    assert!(tool_names.contains(&"list_files"));
}

#[test]
fn test_tool_config_main_agent() {
    let config = ToolConfig::main_agent();

    assert_eq!(config.preset, ToolPreset::Standard);

    assert!(config.additional.contains(&"execute_code".to_string()));
    assert!(config.additional.contains(&"apply_patch".to_string()));

    assert!(config.is_tool_enabled("read_file"));
    assert!(config.is_tool_enabled("grep_file"));
    assert!(config.is_tool_enabled("execute_code"));
    assert!(config.is_tool_enabled("apply_patch"));

    assert!(config.is_tool_enabled("web_fetch"));
    assert!(config.is_tool_enabled("tavily_search"));
    assert!(config.is_tool_enabled("tavily_search_answer"));
    assert!(config.is_tool_enabled("tavily_extract"));
    assert!(config.is_tool_enabled("tavily_crawl"));
    assert!(config.is_tool_enabled("tavily_map"));

    assert!(!config.is_tool_enabled("run_pty_cmd"));

    assert!(!config.is_tool_enabled("save_skill"));
    assert!(!config.is_tool_enabled("create_pty_session"));
}

#[test]
fn test_main_agent_tool_definitions() {
    let config = ToolConfig::main_agent();
    let tools = get_tool_definitions_with_config(&config);
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();

    assert!(tool_names.contains(&"grep_file"));
    assert!(tool_names.contains(&"read_file"));
    assert!(tool_names.contains(&"edit_file"));
    assert!(tool_names.contains(&"write_file"));
    assert!(tool_names.contains(&"list_files"));
    assert!(tool_names.contains(&"create_file"));
    assert!(tool_names.contains(&"delete_file"));

    assert!(!tool_names.contains(&"run_pty_cmd"));
}

#[tokio::test]
async fn test_sub_agent_tool_definitions_no_system_prompt_param() {
    use golish_sub_agents::SubAgentRegistry;

    let mut registry = SubAgentRegistry::new();

    let worker = golish_sub_agents::SubAgentDefinition::new(
        "worker",
        "Worker",
        "A worker agent",
        "default prompt",
    )
    .with_prompt_template("Generate prompt for: {task}");
    registry.register(worker);

    let fixed = golish_sub_agents::SubAgentDefinition::new(
        "fixed",
        "Fixed",
        "A fixed agent",
        "fixed prompt",
    );
    registry.register(fixed);

    let defs = get_sub_agent_tool_definitions(&registry).await;

    for def in &defs {
        let props = def.parameters.get("properties").unwrap();
        assert!(
            props.get("system_prompt").is_none(),
            "Agent {} should NOT have system_prompt parameter",
            def.name
        );
        assert!(
            props.get("task").is_some(),
            "Agent {} should have task parameter",
            def.name
        );
    }
}
