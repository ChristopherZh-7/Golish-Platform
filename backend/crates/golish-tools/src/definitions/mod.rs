//! Tool function declarations for LLM consumption.
//!
//! This module provides the `build_function_declarations()` function that returns
//! tool schemas in the format expected by LLM providers.

mod file_ops;
mod core_tools;
mod memory_tools;
mod knowledge_base;
mod security_tools;
mod graph_tools;
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

        assert_eq!(declarations.len(), 39);

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
