//! Top-level orchestration for the `run_swebench_test` agent tool.
//!
//! This module is intentionally thin — heavy lifting lives in sibling modules:
//! - [`super::commands`] builds the bash payload run inside the container
//! - [`super::validate`] enforces command-injection guards
//! - [`super::docker_exec`] talks to bollard and truncates output
//!
//! Keeping `execute.rs` focused on the tool-call protocol (extracting args,
//! deciding fallback, formatting JSON results) keeps each file well under the
//! 500-line target.

use serde_json::json;

use super::commands::{build_fallback_test_command, build_test_command};
use super::docker_exec::{run_in_container, truncate_output};
use super::get_active_context;
use super::validate::{is_test_runner_missing, validate_test_path};

/// Execute the `run_swebench_test` tool against the current SWE-bench context.
///
/// Returns `(json_result, success_flag)` where `success_flag` is the boolean
/// the agent loop uses to mark the call as successful.
pub async fn execute_swebench_test_tool(args: &serde_json::Value) -> (serde_json::Value, bool) {
    let ctx = match get_active_context() {
        Some(ctx) => ctx,
        None => {
            return (
                json!({
                    "error": "No active SWE-bench container. This tool is only available during SWE-bench evaluations."
                }),
                false,
            );
        }
    };

    let container_name = ctx.container_name.clone();

    let test_path = match args.get("test_path").and_then(|v| v.as_str()) {
        Some(path) => path,
        None => {
            return (
                json!({
                    "error": "Missing required argument: test_path"
                }),
                false,
            );
        }
    };

    let verbose = args
        .get("verbose")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if let Err(e) = validate_test_path(test_path) {
        return (
            json!({
                "error": format!("Invalid test path: {}", e)
            }),
            false,
        );
    }

    let test_cmd = build_test_command(&ctx, test_path, verbose);

    let (stdout, stderr, exit_code) = match run_in_container(&container_name, &test_cmd).await {
        Ok(result) => result,
        Err(e) => {
            return (
                json!({
                    "error": format!("Failed to run tests: {}", e)
                }),
                false,
            );
        }
    };

    let combined_output = format!("{}\n{}", stdout, stderr);
    let needs_fallback = is_test_runner_missing(&combined_output, exit_code);

    if needs_fallback {
        let fallback_cmd = build_fallback_test_command(&ctx, test_path, verbose);

        match run_in_container(&container_name, &fallback_cmd).await {
            Ok((fb_stdout, fb_stderr, fb_exit_code)) => {
                let success = fb_exit_code == 0;
                let output = if fb_stderr.is_empty() {
                    format!(
                        "[Primary test runner unavailable, using fallback]\n\n{}",
                        fb_stdout
                    )
                } else {
                    format!(
                        "[Primary test runner unavailable, using fallback]\n\n{}\n\nSTDERR:\n{}",
                        fb_stdout, fb_stderr
                    )
                };

                return (
                    json!({
                        "output": truncate_output(&output, 50000),
                        "exit_code": fb_exit_code,
                        "success": success,
                        "used_fallback": true
                    }),
                    success,
                );
            }
            Err(e) => {
                return (
                    json!({
                        "error": format!("Both primary and fallback test runners failed: {}", e),
                        "primary_output": truncate_output(&combined_output, 10000)
                    }),
                    false,
                );
            }
        }
    }

    let success = exit_code == 0;
    let output = if stderr.is_empty() {
        stdout
    } else {
        format!("{}\n\nSTDERR:\n{}", stdout, stderr)
    };

    (
        json!({
            "output": truncate_output(&output, 50000),
            "exit_code": exit_code,
            "success": success
        }),
        success,
    )
}

/// Sanitize a JSON schema for LLM compatibility (force `additionalProperties: false`).
pub(super) fn sanitize_schema(mut schema: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = schema.as_object_mut() {
        obj.insert(
            "additionalProperties".to_string(),
            serde_json::Value::Bool(false),
        );
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_without_active_context_returns_error() {
        super::super::clear_active_container();
        let (result, success) = execute_swebench_test_tool(&json!({"test_path": "x"})).await;
        assert!(!success);
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn execute_missing_test_path_returns_error() {
        super::super::set_active_container(Some("dummy-container".to_string()));
        let (result, success) = execute_swebench_test_tool(&json!({})).await;
        assert!(!success);
        let err = result.get("error").and_then(|v| v.as_str()).unwrap_or("");
        assert!(err.contains("Missing required argument"));
        super::super::clear_active_container();
    }

    #[test]
    fn sanitize_schema_adds_additional_properties_false() {
        let schema = sanitize_schema(json!({"type": "object", "properties": {}}));
        assert_eq!(schema["additionalProperties"], json!(false));
    }
}
