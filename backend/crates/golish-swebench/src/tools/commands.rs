//! Builders for the test commands run inside the SWE-bench container.
//!
//! Two builders live here:
//! - [`build_test_command`] — primary command using the repository-specific
//!   runner from `SWEBenchContext::test_command`.
//! - [`build_fallback_test_command`] — generic pytest fallback used when the
//!   primary runner is missing.
//!
//! Both builders sync the agent's edits from `/workspace/repo` into
//! `/testbed` before running, while excluding test files from the sync to
//! prevent the agent from tampering with grading tests.

use super::SWEBenchContext;

/// Build the primary test command based on repository context.
///
/// Test paths are passed as-is without conversion — the agent should use the
/// test format appropriate for the repository (which is the format used in
/// `FAIL_TO_PASS`/`PASS_TO_PASS` lists).
///
/// The command syncs agent changes from `/workspace/repo` to `/testbed` before
/// running tests. This is necessary because:
/// - The agent's working directory is `/workspace/repo` (mounted from host)
/// - The conda environment expects tests to run from `/testbed`
/// - Running from `/workspace/repo` causes pytest `ImportPathMismatchError`
pub(super) fn build_test_command(ctx: &SWEBenchContext, test_path: &str, _verbose: bool) -> String {
    format!(
        r#"
cd /workspace/repo

# Function to check if a file is a test file
is_test_file() {{
    local file="$1"
    case "$file" in
        tests/*|test/*|*/tests/*|*/test/*|test_*.py|*_test.py)
            return 0  # true - is a test file
            ;;
        *)
            return 1  # false - not a test file
            ;;
    esac
}}

echo "=== Syncing changes to /testbed ==="
if [ -d .git ]; then
    for file in $(git diff --name-only HEAD 2>/dev/null || git status --porcelain | awk '{{print $2}}'); do
        if [ -f "$file" ]; then
            # Skip test files
            if is_test_file "$file"; then
                continue
            fi
            mkdir -p "/testbed/$(dirname "$file")"
            cp "$file" "/testbed/$file"
            echo "  Synced: $file"
        fi
    done
fi
cd /testbed
{} {}
"#,
        ctx.test_command, test_path
    )
}

/// Build a fallback test command when the primary runner is missing.
///
/// Always uses `python -m pytest`; still syncs to `/testbed` and excludes test
/// files from the sync.
pub(super) fn build_fallback_test_command(
    _ctx: &SWEBenchContext,
    test_path: &str,
    verbose: bool,
) -> String {
    let verbose_flags = if verbose { "-xvs" } else { "-x" };
    format!(
        r#"
cd /workspace/repo

# Function to check if a file is a test file
is_test_file() {{
    local file="$1"
    case "$file" in
        tests/*|test/*|*/tests/*|*/test/*|test_*.py|*_test.py)
            return 0  # true - is a test file
            ;;
        *)
            return 1  # false - not a test file
            ;;
    esac
}}

if [ -d .git ]; then
    for file in $(git diff --name-only HEAD 2>/dev/null || git status --porcelain | awk '{{print $2}}'); do
        if [ -f "$file" ]; then
            # Skip test files
            if is_test_file "$file"; then
                continue
            fi
            mkdir -p "/testbed/$(dirname "$file")"
            cp "$file" "/testbed/$file"
        fi
    done
fi
cd /testbed
python -m pytest {} {}
"#,
        verbose_flags, test_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_pytest_command_for_astropy() {
        let ctx = SWEBenchContext {
            container_name: "test".to_string(),
            test_command: "pytest -rA -vv -o console_output_style=classic --tb=no".to_string(),
            repo: "astropy/astropy".to_string(),
        };

        let cmd = build_test_command(
            &ctx,
            "astropy/io/ascii/tests/test_rst.py::test_rst_with_header_rows",
            true,
        );
        assert!(cmd.contains("Syncing changes to /testbed"));
        assert!(cmd.contains("cd /testbed"));
        assert!(cmd.contains("pytest -rA"));
        assert!(cmd.contains("astropy/io/ascii/tests/test_rst.py::test_rst_with_header_rows"));
    }

    #[test]
    fn build_django_runtests_command() {
        let ctx = SWEBenchContext {
            container_name: "test".to_string(),
            test_command: "./tests/runtests.py --verbosity 2 --settings=test_sqlite --parallel 1"
                .to_string(),
            repo: "django/django".to_string(),
        };

        let cmd = build_test_command(
            &ctx,
            "admin_views.tests.AdminViewBasicTest.test_login",
            true,
        );
        assert!(cmd.contains("Syncing changes to /testbed"));
        assert!(cmd.contains("cd /testbed"));
        assert!(cmd.contains("./tests/runtests.py"));
        assert!(cmd.contains("--settings=test_sqlite"));
        assert!(cmd.contains("admin_views.tests.AdminViewBasicTest.test_login"));
    }

    #[test]
    fn fallback_uses_pytest_and_syncs_testbed() {
        let ctx = SWEBenchContext {
            container_name: "test".to_string(),
            test_command: "./tests/runtests.py --verbosity 2".to_string(),
            repo: "django/django".to_string(),
        };

        let fallback = build_fallback_test_command(&ctx, "admin_views.tests", true);
        assert!(fallback.contains("cd /testbed"));
        assert!(fallback.contains("python -m pytest"));
        assert!(fallback.contains("-xvs"));
    }
}
