//! Test path validation and test-runner availability detection.
//!
//! Pure helpers that contain no I/O — they exist as their own module so the
//! security-sensitive validation logic and runner-detection heuristics can be
//! unit-tested in isolation.

use anyhow::Result;

/// Validate a test path to prevent command injection.
///
/// Only allows:
/// - Alphanumeric characters
/// - Underscores, hyphens, dots
/// - Forward slashes (path separators)
/// - Colons (for pytest test selection)
/// - Brackets (for parameterized tests)
/// - Spaces (for `-k` patterns, but limited)
pub(super) fn validate_test_path(path: &str) -> Result<()> {
    let forbidden_chars = [
        '`', '$', ';', '&', '|', '>', '<', '!', '\\', '\n', '\r', '\'', '"',
    ];

    for c in forbidden_chars {
        if path.contains(c) {
            anyhow::bail!("Forbidden character '{}' in test path", c);
        }
    }

    if path.contains("$(") || path.contains("${") {
        anyhow::bail!("Command substitution not allowed");
    }

    if path.contains("..") {
        anyhow::bail!("Path traversal not allowed");
    }

    if path.len() > 1000 {
        anyhow::bail!("Test path too long (max 1000 characters)");
    }

    Ok(())
}

/// Check if the test runner is missing based on output and exit code.
pub(super) fn is_test_runner_missing(output: &str, exit_code: i64) -> bool {
    if exit_code == 127 {
        return true;
    }

    let missing_indicators = [
        "No module named pytest",
        "No module named 'pytest'",
        "pytest: not found",
        "pytest not found",
        "command not found: pytest",
        "/bin/bash: pytest: command not found",
        "ModuleNotFoundError: No module named 'pytest'",
        "No module named django",
        "No module named 'django'",
        "./tests/runtests.py: No such file or directory",
        "runtests.py: not found",
    ];

    missing_indicators
        .iter()
        .any(|indicator| output.contains(indicator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_test_path_valid() {
        assert!(validate_test_path("tests/test_example.py").is_ok());
        assert!(validate_test_path("tests/test_example.py::test_function").is_ok());
        assert!(validate_test_path("tests/test_example.py::TestClass::test_method").is_ok());
        assert!(validate_test_path("-k test_pattern").is_ok());
        assert!(validate_test_path("tests/test_foo.py[param1]").is_ok());
    }

    #[test]
    fn validate_test_path_invalid() {
        assert!(validate_test_path("tests/test.py; rm -rf /").is_err());
        assert!(validate_test_path("tests/test.py && cat /etc/passwd").is_err());
        assert!(validate_test_path("tests/test.py | grep secret").is_err());
        assert!(validate_test_path("$(whoami)").is_err());
        assert!(validate_test_path("tests/`id`/test.py").is_err());
        assert!(validate_test_path("tests/../../../etc/passwd").is_err());
    }

    #[test]
    fn detects_missing_pytest() {
        assert!(is_test_runner_missing("No module named pytest", 1));
        assert!(is_test_runner_missing(
            "ModuleNotFoundError: No module named 'pytest'",
            1
        ));
        assert!(is_test_runner_missing(
            "/bin/bash: pytest: command not found",
            127
        ));
    }

    #[test]
    fn detects_missing_django_runner() {
        assert!(is_test_runner_missing(
            "./tests/runtests.py: No such file or directory",
            1
        ));
    }

    #[test]
    fn exit_127_always_missing() {
        assert!(is_test_runner_missing("", 127));
    }

    #[test]
    fn normal_failures_do_not_trigger_fallback() {
        assert!(!is_test_runner_missing("FAILED test_foo.py::test_bar", 1));
        assert!(!is_test_runner_missing("AssertionError: expected True", 1));
        assert!(!is_test_runner_missing("1 passed, 2 failed", 1));
    }
}
