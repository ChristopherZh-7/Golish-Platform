//! Prompt construction for SWE-bench scenarios.

use crate::types::SWEBenchInstance;

use super::SWEBenchScenario;

impl SWEBenchScenario {
    /// Build the base prompt for the agent (without test environment info).
    ///
    /// Note: this version uses a placeholder path — prefer
    /// [`Self::build_prompt_with_workspace`] for actual runs.
    pub(super) fn build_prompt(instance: &SWEBenchInstance) -> String {
        Self::build_prompt_with_workspace(instance, None, None)
    }

    /// Build the prompt for the agent with workspace path and optional
    /// Docker container.
    ///
    /// # Arguments
    /// * `instance` — the SWE-bench instance.
    /// * `repo_path` — the actual host filesystem path to the repository
    ///   root (where the agent will work).
    /// * `container_name` — optional Docker container name for running
    ///   tests.
    pub(super) fn build_prompt_with_workspace(
        instance: &SWEBenchInstance,
        _repo_path: Option<&std::path::Path>,
        container_name: Option<&str>,
    ) -> String {
        let hints_section = instance
            .hints_text
            .as_ref()
            .filter(|h| !h.is_empty())
            .map(|hints| format!("## Hints\n\n{}\n\n", hints))
            .unwrap_or_default();

        let test_section = container_name
            .map(|_| Self::build_test_section(instance))
            .unwrap_or_default();

        // Get the fail_to_pass tests for the prompt.
        let fail_to_pass_tests = instance.fail_to_pass_tests();
        let tests_to_fix = if !fail_to_pass_tests.is_empty() {
            let tests_list = fail_to_pass_tests
                .iter()
                .map(|t| format!("- `{}`", t))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                r#"## Tests to Fix

The following tests currently fail and must pass after your fix:

{}

"#,
                tests_list
            )
        } else {
            String::new()
        };

        format!(
            r#"You are fixing a software engineering issue from the SWE-bench benchmark.

## Repository
- Repository: {repo}
- Version: {version}

## Problem Statement

{problem_statement}

{hints_section}{tests_to_fix}## Success Criteria

You are done when:
1. All tests listed above pass
2. No other tests regress (existing functionality preserved)
3. Only necessary files were modified

## Approach

1. **Run the failing test** - Use `run_swebench_test` to see the actual error and traceback
2. **Locate the bug** - The traceback shows the exact file, function, and line
3. **Make a minimal fix** - Change only what's necessary (typically 1-3 files)
4. **Verify** - Run the test again to confirm it passes
5. **Check for regressions** - Ensure you haven't broken other tests

## Constraints

- Do not modify test files (they are read-only)
- Do not refactor or "improve" unrelated code
- Preserve all existing functionality

{test_section}"#,
            repo = instance.repo,
            version = instance.version,
            problem_statement = instance.problem_statement,
            hints_section = hints_section,
            tests_to_fix = tests_to_fix,
            test_section = test_section,
        )
    }

    /// Build the test section of the prompt.
    ///
    /// Uses the `run_swebench_test` tool instead of `docker exec` to
    /// prevent agents from accessing git history which could leak fix
    /// commits.
    pub(super) fn build_test_section(instance: &SWEBenchInstance) -> String {
        let is_django = instance.repo == "django/django";

        let examples = if is_django {
            r#"
For Django, use dotted module paths:
- Module: `{"test_path": "admin_views.tests"}`
- Method: `{"test_path": "admin_views.tests.AdminViewBasicTest.test_login"}`"#
        } else {
            ""
        };

        format!(
            r#"## Running Tests

Use relative paths from the repository root for all file operations.

Test with `run_swebench_test`:
```json
{{"test_path": "tests/test_example.py::test_function"}}
```

Other patterns:
- Full file: `{{"test_path": "tests/test_example.py"}}`
- Pattern match: `{{"test_path": "-k test_pattern"}}`
{examples}
Your changes are automatically synced to the test environment.
"#,
            examples = examples,
        )
    }
}
