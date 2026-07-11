//! Tool function declarations for LLM consumption.
//!
//! This module provides the `build_function_declarations()` function that returns
//! tool schemas in the format expected by LLM providers.

mod core_tools;
mod file_ops;
mod graph_tools;
mod knowledge_base;
mod memory_tools;
mod security_tools;
mod sploitus_tools;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Function declaration format for LLM tool calling.
///
/// This struct matches the format expected by vtcode_core::tools::registry::FunctionDeclaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    /// Tool name (must match what the tool registry expects)
    pub name: String,
    /// Human-readable description for the LLM
    pub description: String,
    /// JSON Schema for the tool's parameters
    pub parameters: Value,
}

/// Build all tool declarations for LLM consumption.
///
/// This is a drop-in replacement for vtcode_core::tools::registry::build_function_declarations().
///
/// Returns a vector of function declarations that describe all available tools
/// and their parameter schemas.
pub fn build_function_declarations() -> Vec<FunctionDeclaration> {
    let mut decls = Vec::new();
    decls.extend(file_ops::file_declarations());
    decls.extend(file_ops::directory_declarations());
    decls.extend(core_tools::plan_declarations());
    decls.extend(core_tools::shell_declarations());
    decls.extend(core_tools::ast_declarations());
    decls.extend(memory_tools::memory_declarations());
    decls.extend(memory_tools::code_store_declarations());
    decls.extend(memory_tools::guide_store_declarations());
    decls.extend(knowledge_base::knowledge_base_declarations());
    decls.extend(security_tools::security_analysis_declarations());
    decls.extend(graph_tools::graph_declarations());
    decls.extend(sploitus_tools::sploitus_declarations());
    decls
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_function_declarations_returns_all_tools() {
        let declarations = build_function_declarations();

        assert_eq!(declarations.len(), 48);

        let names: Vec<&str> = declarations.iter().map(|d| d.name.as_str()).collect();

        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"create_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"delete_file"));

        assert!(names.contains(&"list_files"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"grep_file"));

        assert!(names.contains(&"ast_grep"));
        assert!(names.contains(&"ast_grep_replace"));

        assert!(names.contains(&"run_pty_cmd"));

        assert!(names.contains(&"update_plan"));
        assert!(names.contains(&"update_plan_patch"));

        assert!(names.contains(&"search_memories"));
        assert!(names.contains(&"store_memory"));
        assert!(names.contains(&"list_memories"));

        assert!(names.contains(&"search_knowledge_base"));
        assert!(names.contains(&"write_knowledge"));
        assert!(names.contains(&"read_knowledge"));
        assert!(names.contains(&"ingest_cve"));
        assert!(names.contains(&"save_poc"));
        assert!(names.contains(&"list_cves_with_pocs"));
        assert!(names.contains(&"list_unresearched_cves"));
        assert!(names.contains(&"poc_stats"));

        assert!(names.contains(&"log_operation"));
        assert!(names.contains(&"discover_apis"));
        assert!(names.contains(&"save_js_analysis"));
        assert!(names.contains(&"fingerprint_target"));
        assert!(names.contains(&"log_scan_result"));
        assert!(names.contains(&"query_target_data"));
        assert!(names.contains(&"list_in_scope_targets"));
        assert!(names.contains(&"list_attack_surface_seeds"));
        assert!(names.contains(&"list_enumeration_web_roots"));
        assert!(names.contains(&"enum_preflight_web_origins"));
        assert!(names.contains(&"check_stage_asset_coverage"));
        assert!(names.contains(&"stage_worklist_status"));
        assert!(names.contains(&"stage_worklist_next"));
        assert!(names.contains(&"list_recent_evidence"));

        assert!(names.contains(&"graph_add_entity"));
        assert!(names.contains(&"graph_add_relation"));
        assert!(names.contains(&"graph_search"));
        assert!(names.contains(&"graph_neighbors"));
        assert!(names.contains(&"graph_attack_paths"));

        assert!(names.contains(&"search_exploits"));
    }

    #[test]
    fn test_declarations_have_valid_schemas() {
        let declarations = build_function_declarations();

        for decl in declarations {
            assert!(!decl.name.is_empty(), "Declaration should have a name");
            assert!(
                !decl.description.is_empty(),
                "Declaration should have a description"
            );
            assert_eq!(
                decl.parameters.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "Parameters should be an object type for {}",
                decl.name
            );
            assert!(
                decl.parameters.get("properties").is_some(),
                "Parameters should have properties for {}",
                decl.name
            );
        }
    }

    #[test]
    fn enumeration_preflight_tools_share_terminal_exception_preview_schema() {
        let declarations = build_function_declarations();

        for name in [
            "check_stage_asset_coverage",
            "stage_worklist_status",
            "stage_worklist_next",
        ] {
            let declaration = declarations
                .iter()
                .find(|declaration| declaration.name == name)
                .unwrap_or_else(|| panic!("missing declaration for {name}"));
            let schema = &declaration.parameters["properties"]["terminal_exceptions"];
            assert_eq!(schema["type"], "array");
            assert!(schema.get("maxItems").is_none());
            assert_eq!(schema["items"]["additionalProperties"], false);
            assert_eq!(schema["items"]["properties"], json!({}));
            assert!(schema["description"]
                .as_str()
                .unwrap()
                .contains("non-empty array is rejected"));
        }

        let next = declarations
            .iter()
            .find(|declaration| declaration.name == "stage_worklist_next")
            .unwrap();
        assert!(next.parameters["properties"]["prefer"]["items"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("partial")));
        assert!(next
            .description
            .contains("at most 50 distinct exact-origin roots"));
        assert!(next.parameters["properties"]["limit"]["description"]
            .as_str()
            .unwrap()
            .contains("max 200"));

        let roots = declarations
            .iter()
            .find(|declaration| declaration.name == "list_enumeration_web_roots")
            .unwrap();
        assert!(roots.parameters["properties"]["limit"]["description"]
            .as_str()
            .unwrap()
            .contains("Default 25, max 50"));

        let transport = declarations
            .iter()
            .find(|declaration| declaration.name == "enum_preflight_web_origins")
            .unwrap();
        let origins = &transport.parameters["properties"]["origins"];
        assert!(origins.get("minItems").is_none());
        assert!(origins.get("maxItems").is_none());
        assert!(origins["items"]["properties"]["target_id"]
            .get("format")
            .is_none());
        assert_eq!(origins["items"]["additionalProperties"], false);
    }

    #[test]
    fn test_read_file_declaration() {
        let declarations = build_function_declarations();
        let read_file = declarations
            .iter()
            .find(|d| d.name == "read_file")
            .expect("read_file should exist");

        let required = read_file.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));

        let props = read_file.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("line_start"));
        assert!(props.contains_key("line_end"));
    }

    #[test]
    fn test_edit_file_declaration() {
        let declarations = build_function_declarations();
        let edit_file = declarations
            .iter()
            .find(|d| d.name == "edit_file")
            .expect("edit_file should exist");

        let required = edit_file.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("old_text")));
        assert!(required.contains(&json!("new_text")));
    }

    #[test]
    fn test_run_pty_cmd_declaration() {
        let declarations = build_function_declarations();
        let run_pty_cmd = declarations
            .iter()
            .find(|d| d.name == "run_pty_cmd")
            .expect("run_pty_cmd should exist");

        let required = run_pty_cmd.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("command")));

        let props = run_pty_cmd.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("command"));
        assert!(props.contains_key("cwd"));
        assert!(props.contains_key("timeout"));
    }

    #[test]
    fn test_function_declaration_serialization() {
        let decl = FunctionDeclaration {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "arg": {"type": "string"}
                },
                "required": ["arg"]
            }),
        };

        let json_str = serde_json::to_string(&decl).unwrap();
        assert!(json_str.contains("test_tool"));

        let parsed: FunctionDeclaration = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.name, "test_tool");
    }
}
