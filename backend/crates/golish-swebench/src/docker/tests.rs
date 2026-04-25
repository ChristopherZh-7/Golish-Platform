//! Unit tests for `DockerExecutor`: live Docker check + pure parser cases.

use super::*;
use crate::types::SWEBenchInstance;

#[tokio::test]
async fn test_docker_connection() {
    // This test requires Docker to be running
    if let Ok(executor) = DockerExecutor::new() {
        let available = executor.is_available().await;
        println!("Docker available: {}", available);
    }
}

#[test]
fn test_strip_ansi_codes() {
    // Test stripping color codes from pytest output
    let input = "test_foo.py::test_bar \x1b[32mPASSED\x1b[0m";
    let expected = "test_foo.py::test_bar PASSED";
    assert_eq!(DockerExecutor::strip_ansi_codes(input), expected);

    let input = "test_foo.py::test_baz \x1b[31mFAILED\x1b[0m";
    let expected = "test_foo.py::test_baz FAILED";
    assert_eq!(DockerExecutor::strip_ansi_codes(input), expected);

    let input = "\x1b[1m\x1b[31mFAILED\x1b[0m test.py";
    let expected = "FAILED test.py";
    assert_eq!(DockerExecutor::strip_ansi_codes(input), expected);

    let input = "plain text";
    assert_eq!(DockerExecutor::strip_ansi_codes(input), input);
}

#[test]
fn test_parse_pytest_output_with_ansi() {
    // Simulate actual pytest output with ANSI codes
    let stdout = r#"
test_rst.py::test_read_normal [32mPASSED[0m
test_rst.py::test_with_header_rows [31mFAILED[0m
"#;
    let stdout = stdout
        .replace("[32m", "\x1b[32m")
        .replace("[31m", "\x1b[31m")
        .replace("[0m", "\x1b[0m");

    let cleaned = DockerExecutor::strip_ansi_codes(&stdout);
    assert!(cleaned.contains("test_read_normal PASSED"));
    assert!(cleaned.contains("test_with_header_rows FAILED"));
}

#[test]
fn test_parse_django_test_output() {
    let instance = SWEBenchInstance {
        instance_id: "django__django-11133".to_string(),
        repo: "django/django".to_string(),
        base_commit: "abc123".to_string(),
        problem_statement: "Test".to_string(),
        patch: "".to_string(),
        test_patch: "".to_string(),
        fail_to_pass: r#"["test_utils.tests.OverrideSettingsTests.test_override_file_upload_permissions"]"#.to_string(),
        pass_to_pass: "[]".to_string(),
        version: "3.0".to_string(),
        environment_setup_commit: "def456".to_string(),
        hints_text: None,
        created_at: None,
    };

    // Django test output format with passing test
    let stdout = r#"
Testing against Django installed in '/testbed/django'
System check identified no issues (0 silenced).
test_override_file_upload_permissions (test_utils.tests.OverrideSettingsTests) ... ok

----------------------------------------------------------------------
Ran 1 test in 0.001s

OK
"#;

    let (fail_to_pass_results, _) = DockerExecutor::parse_test_results(&instance, stdout, "");

    assert_eq!(fail_to_pass_results.len(), 1);
    assert!(
        fail_to_pass_results[0].passed,
        "Test should be marked as passed, got: {:?}",
        fail_to_pass_results[0]
    );
}

#[test]
fn test_parse_django_test_output_with_module_failure() {
    let instance = SWEBenchInstance {
        instance_id: "django__django-11133".to_string(),
        repo: "django/django".to_string(),
        base_commit: "abc123".to_string(),
        problem_statement: "Test".to_string(),
        patch: "".to_string(),
        test_patch: "".to_string(),
        fail_to_pass: r#"["test_utils.tests.OverrideSettingsTests.test_override_file_upload_permissions"]"#.to_string(),
        pass_to_pass: "[]".to_string(),
        version: "3.0".to_string(),
        environment_setup_commit: "def456".to_string(),
        hints_text: None,
        created_at: None,
    };

    // Django test output with module load failure
    let stdout = r#"
Testing against Django installed in '/testbed/django'
test_utils (unittest.loader._FailedTest) ... ERROR

======================================================================
ERROR: test_utils (unittest.loader._FailedTest)
----------------------------------------------------------------------
ImportError: Failed to import test module: test_utils
"#;

    let (fail_to_pass_results, _) = DockerExecutor::parse_test_results(&instance, stdout, "");

    assert_eq!(fail_to_pass_results.len(), 1);
    assert!(
        !fail_to_pass_results[0].passed,
        "Test should be marked as failed due to module load error"
    );
}

#[test]
fn test_parse_django_test_output_mixed() {
    let instance = SWEBenchInstance {
        instance_id: "django__django-11133".to_string(),
        repo: "django/django".to_string(),
        base_commit: "abc123".to_string(),
        problem_statement: "Test".to_string(),
        patch: "".to_string(),
        test_patch: "".to_string(),
        fail_to_pass: r#"["admin_views.tests.AdminViewBasicTest.test_login"]"#.to_string(),
        pass_to_pass: r#"["admin_views.tests.AdminViewBasicTest.test_logout"]"#.to_string(),
        version: "3.0".to_string(),
        environment_setup_commit: "def456".to_string(),
        hints_text: None,
        created_at: None,
    };

    // Django test output with mixed results
    let stdout = r#"
Testing against Django installed in '/testbed/django'
test_login (admin_views.tests.AdminViewBasicTest) ... ok
test_logout (admin_views.tests.AdminViewBasicTest) ... FAIL

----------------------------------------------------------------------
Ran 2 tests in 0.005s

FAILED (failures=1)
"#;

    let (fail_to_pass_results, pass_to_pass_results) =
        DockerExecutor::parse_test_results(&instance, stdout, "");

    assert_eq!(fail_to_pass_results.len(), 1);
    assert!(
        fail_to_pass_results[0].passed,
        "test_login should pass: {:?}",
        fail_to_pass_results[0]
    );

    assert_eq!(pass_to_pass_results.len(), 1);
    assert!(
        !pass_to_pass_results[0].passed,
        "test_logout should fail: {:?}",
        pass_to_pass_results[0]
    );
}
