//! SWE-bench specific tools for agent execution.
//!
//! These tools are only available during SWE-bench evaluations and provide
//! restricted access to the Docker test environment. This prevents agents from
//! accessing git history or other information that could leak answers.
//!
//! # Module layout
//!
//! - [`execute`] — orchestrates a `run_swebench_test` call (this is the
//!   public entry point exported via [`execute_swebench_test_tool`])
//! - [`commands`] — builds the bash payload run inside the container
//! - [`validate`] — command-injection guards and runner-availability heuristics
//! - [`docker_exec`] — bollard exec wrapper and output truncation
//!
//! `mod.rs` itself owns the thread-local [`SWEBenchContext`] state and the
//! tool definition consumed by the agent.

use std::cell::RefCell;

use rig::completion::ToolDefinition;
use serde_json::json;

mod commands;
mod docker_exec;
mod execute;
mod validate;

pub use execute::execute_swebench_test_tool;
use execute::sanitize_schema;

/// Context for the active SWE-bench container.
#[derive(Clone)]
pub struct SWEBenchContext {
    /// Container name for `docker exec`.
    pub container_name: String,
    /// Test command prefix (e.g. `python -m pytest -xvs` or `./tests/runtests.py`).
    pub test_command: String,
    /// Repository name (e.g. `django/django`).
    pub repo: String,
}

thread_local! {
    /// Thread-local storage for the active SWE-bench context.
    /// Set by `SWEBenchScenario::run()` before agent execution.
    static ACTIVE_CONTEXT: RefCell<Option<SWEBenchContext>> = const { RefCell::new(None) };
}

/// Set the active SWE-bench context for the current thread.
///
/// Called by `SWEBenchScenario` before running the agent.
pub fn set_active_context(ctx: Option<SWEBenchContext>) {
    ACTIVE_CONTEXT.with(|cell| {
        *cell.borrow_mut() = ctx;
    });
}

/// Get the active SWE-bench context for the current thread.
pub fn get_active_context() -> Option<SWEBenchContext> {
    ACTIVE_CONTEXT.with(|cell| cell.borrow().clone())
}

/// Set the active container (convenience wrapper for backward compatibility).
pub fn set_active_container(name: Option<String>) {
    if let Some(name) = name {
        set_active_context(Some(SWEBenchContext {
            container_name: name,
            test_command: "python -m pytest -xvs".to_string(),
            repo: "unknown".to_string(),
        }));
    } else {
        set_active_context(None);
    }
}

/// Get the active container name for the current thread.
pub fn get_active_container() -> Option<String> {
    get_active_context().map(|ctx| ctx.container_name)
}

/// Clear the active container/context.
///
/// Called by `SWEBenchScenario` after agent execution.
pub fn clear_active_container() {
    set_active_context(None);
}

/// Get the tool definition for the SWE-bench test runner.
///
/// This tool allows the agent to run tests in the Docker container without
/// giving it direct access to `docker exec` (which would allow accessing git
/// history containing the fix commits).
pub fn get_swebench_test_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "run_swebench_test".to_string(),
        description: "Run tests in the SWE-bench Docker test environment. \
            Use this to verify your code changes. \
            The appropriate test runner for the repository is used automatically \
            (pytest for most repos, Django's test runner for Django, etc.). \
            You can run specific test files, test functions, or test patterns."
            .to_string(),
        parameters: sanitize_schema(json!({
            "type": "object",
            "properties": {
                "test_path": {
                    "type": "string",
                    "description": "The test to run. Can be:\n\
                        - A test file path (e.g., 'tests/test_example.py')\n\
                        - A specific test (e.g., 'tests/test_example.py::test_function')\n\
                        - A test class (e.g., 'tests/test_example.py::TestClass')\n\
                        - A pattern with -k (e.g., '-k test_memoryview')"
                },
                "verbose": {
                    "type": "boolean",
                    "description": "Whether to use verbose output (-xvs flags). Defaults to true."
                }
            },
            "required": ["test_path"]
        })),
    }
}

/// Whether `tool_name` is a SWE-bench specific tool.
pub fn is_swebench_tool(tool_name: &str) -> bool {
    tool_name == "run_swebench_test"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_thread_local_set_and_clear() {
        set_active_container(Some("test-container".to_string()));
        assert_eq!(get_active_container(), Some("test-container".to_string()));

        clear_active_container();
        assert_eq!(get_active_container(), None);
    }

    #[test]
    fn tool_definition_metadata() {
        let def = get_swebench_test_tool_definition();
        assert_eq!(def.name, "run_swebench_test");
        assert!(def.description.contains("pytest"));
    }

    #[test]
    fn is_swebench_tool_recognises_only_run_swebench_test() {
        assert!(is_swebench_tool("run_swebench_test"));
        assert!(!is_swebench_tool("run_shell_command"));
        assert!(!is_swebench_tool(""));
    }
}
