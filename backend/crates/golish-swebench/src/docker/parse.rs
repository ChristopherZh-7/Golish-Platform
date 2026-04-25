//! Pure parsing of pytest/Django test output (no Docker required).

use std::collections::HashMap;

use tracing::debug;

use crate::types::{SWEBenchInstance, TestResult};

use super::DockerExecutor;

impl DockerExecutor {
    /// Strip ANSI escape codes from a string.
    pub(super) fn strip_ansi_codes(s: &str) -> String {
        // ANSI escape codes start with ESC[ (0x1B 0x5B) and end with a letter
        // Common patterns: \x1b[0m, \x1b[31m, \x1b[32m, etc.
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                if chars.peek() == Some(&'[') {
                    chars.next(); // consume '['
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Parse test results from output.
    ///
    /// This is a pure parsing function that doesn't require Docker.
    pub(super) fn parse_test_results(
        instance: &SWEBenchInstance,
        stdout: &str,
        stderr: &str,
    ) -> (Vec<TestResult>, Vec<TestResult>) {
        let fail_to_pass = instance.fail_to_pass_tests();
        let pass_to_pass = instance.pass_to_pass_tests();

        let clean_stdout = Self::strip_ansi_codes(stdout);
        let clean_stderr = Self::strip_ansi_codes(stderr);

        // Parse test output for test results
        // Supports both pytest and Django test runner formats
        let mut results: HashMap<String, bool> = HashMap::new();

        let combined_output = format!("{}\n{}", clean_stdout, clean_stderr);

        debug!(
            "Parsing test results from {} lines of output",
            clean_stdout.lines().count()
        );

        let is_django = instance.test_runner() == crate::types::TestRunner::Django;

        for line in clean_stdout.lines() {
            let line = line.trim();

            // Django test runner format: "test_method (module.TestClass) ... ok/FAIL/ERROR"
            // Examples:
            //   test_override_file_upload_permissions (test_utils.tests.OverrideSettingsTests) ... ok
            //   test_something (admin_views.tests.AdminViewTest) ... FAIL
            //   module_name (unittest.loader._FailedTest) ... ERROR
            if is_django && line.contains(" ... ") {
                if let Some((test_part, status_part)) = line.rsplit_once(" ... ") {
                    let passed = status_part.trim().eq_ignore_ascii_case("ok");
                    let is_error = status_part.trim().eq_ignore_ascii_case("error");

                    // Extract test method and class: "test_method (module.Class)"
                    // Also handle _FailedTest: "module (unittest.loader._FailedTest)"
                    if let Some((method_name, class_part)) = test_part.rsplit_once(" (") {
                        let class_path = class_part.trim_end_matches(')');

                        if class_path.contains("_FailedTest") {
                            // The method_name here is actually the module that failed
                            // Mark any tests containing this module as failed
                            debug!(
                                "Django module load failure: {} ({})",
                                method_name, class_path
                            );
                            results.insert(format!("__module_fail__{}", method_name), false);
                            continue;
                        }

                        let full_test_name = format!("{}.{}", class_path, method_name);
                        debug!(
                            "Parsed Django test result: {} = {}",
                            full_test_name,
                            if passed {
                                "ok"
                            } else if is_error {
                                "ERROR"
                            } else {
                                "FAIL"
                            }
                        );
                        results.insert(full_test_name, passed);

                        // Also store just the class path for partial matching
                        // (in case FAIL_TO_PASS has module.Class without method)
                        if !results.contains_key(class_path) || passed {
                            results.insert(class_path.to_string(), passed);
                        }
                    }
                }
            }
            // pytest verbose output: "test_module.py::test_name PASSED"
            else if line.contains(" PASSED")
                || line.contains(" FAILED")
                || line.contains(" ERROR")
            {
                let passed = line.contains(" PASSED");
                let parts: Vec<&str> = line.split_whitespace().collect();
                if !parts.is_empty() {
                    let test_name = parts[0].to_string();
                    debug!(
                        "Parsed pytest result: {} = {}",
                        test_name,
                        if passed { "PASSED" } else { "FAILED" }
                    );
                    results.insert(test_name, passed);
                }
            }
        }

        debug!("Found {} test results in output", results.len());

        debug!("Looking for FAIL_TO_PASS tests: {:?}", fail_to_pass);
        debug!("Looking for PASS_TO_PASS tests: {:?}", pass_to_pass);
        debug!(
            "Parsed result keys: {:?}",
            results.keys().collect::<Vec<_>>()
        );

        let error_patterns = Self::extract_error_messages(&combined_output);

        let fail_to_pass_results: Vec<TestResult> = fail_to_pass
            .iter()
            .map(|test| {
                let passed = Self::find_test_result(&results, test);
                debug!(
                    "FAIL_TO_PASS test '{}': passed={} (looking in {} parsed results)",
                    test,
                    passed,
                    results.len()
                );
                let error = if passed {
                    None
                } else {
                    Self::find_error_for_test(&error_patterns, test, &combined_output)
                        .or_else(|| Some("Test did not pass".to_string()))
                };
                TestResult {
                    name: test.clone(),
                    passed,
                    error,
                    duration_ms: None,
                }
            })
            .collect();

        let pass_to_pass_results: Vec<TestResult> = pass_to_pass
            .iter()
            .map(|test| {
                let passed = Self::find_test_result(&results, test);
                let error = if passed {
                    None
                } else {
                    Self::find_error_for_test(&error_patterns, test, &combined_output)
                        .or_else(|| Some("Test regression".to_string()))
                };
                TestResult {
                    name: test.clone(),
                    passed,
                    error,
                    duration_ms: None,
                }
            })
            .collect();

        (fail_to_pass_results, pass_to_pass_results)
    }

    /// Extract error messages from pytest output.
    fn extract_error_messages(output: &str) -> HashMap<String, String> {
        let mut errors = HashMap::new();
        let lines: Vec<&str> = output.lines().collect();

        // Look for error blocks in pytest output
        // Format: "FAILED test_name - ErrorType: message"
        for line in &lines {
            if line.contains("FAILED") && line.contains(" - ") {
                if let Some(idx) = line.find(" - ") {
                    let test_part = line[..idx].trim();
                    let error_part = line[idx + 3..].trim();
                    if let Some(test_name) = test_part.split_whitespace().last() {
                        errors.insert(test_name.to_string(), error_part.to_string());
                    }
                }
            }
        }

        // Also look for common Python errors
        for (i, line) in lines.iter().enumerate() {
            if line.contains("ImportError:")
                || line.contains("ModuleNotFoundError:")
                || line.contains("SyntaxError:")
                || line.contains("NameError:")
                || line.contains("AttributeError:")
                || line.contains("TypeError:")
            {
                // Try to find which test this belongs to by looking backwards
                for j in (0..i).rev() {
                    if lines[j].contains("::")
                        && (lines[j].contains("test_") || lines[j].contains("Test"))
                    {
                        let test_name = lines[j].split_whitespace().next().unwrap_or("");
                        if !test_name.is_empty() && !errors.contains_key(test_name) {
                            errors.insert(test_name.to_string(), line.trim().to_string());
                        }
                        break;
                    }
                }
            }
        }

        errors
    }

    /// Find error message for a specific test.
    fn find_error_for_test(
        errors: &HashMap<String, String>,
        test_name: &str,
        output: &str,
    ) -> Option<String> {
        if let Some(error) = errors.get(test_name) {
            return Some(error.clone());
        }

        for (key, error) in errors {
            if key.contains(test_name) || test_name.contains(key.as_str()) {
                return Some(error.clone());
            }
        }

        // Look for collection errors (tests that couldn't even be collected)
        if output.contains("collected 0 items") {
            if output.contains("ImportError") {
                for line in output.lines() {
                    if line.contains("ImportError") {
                        return Some(format!("Collection failed: {}", line.trim()));
                    }
                }
            }
            if output.contains("SyntaxError") {
                for line in output.lines() {
                    if line.contains("SyntaxError") {
                        return Some(format!("Collection failed: {}", line.trim()));
                    }
                }
            }
            return Some("Test collection failed - check for import or syntax errors".to_string());
        }

        None
    }

    /// Find test result by name (handles various naming formats).
    fn find_test_result(results: &HashMap<String, bool>, test_name: &str) -> bool {
        if let Some(&passed) = results.get(test_name) {
            return passed;
        }

        // Check for module-level failures (Django _FailedTest)
        // Test name like "test_utils.tests.TestClass.test_method" - check if "test_utils" module failed
        for (key, &_passed) in results {
            if key.starts_with("__module_fail__") {
                let failed_module = key.trim_start_matches("__module_fail__");
                if test_name.starts_with(failed_module)
                    || test_name.starts_with(&format!("{}.", failed_module))
                {
                    debug!(
                        "Test {} matched module failure for {}",
                        test_name, failed_module
                    );
                    return false;
                }
            }
        }

        // Partial match (test name might be part of the key)
        for (key, &passed) in results {
            if key.starts_with("__module_fail__") {
                continue;
            }
            if key.contains(test_name) || test_name.contains(key.as_str()) {
                return passed;
            }
        }

        debug!(
            "Test {} not found in results, defaulting to failed",
            test_name
        );
        false
    }
}
