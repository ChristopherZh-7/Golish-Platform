use golish_evals::scenarios::Scenario;

use crate::types::SWEBenchInstance;

use super::SWEBenchScenario;

fn mock_instance() -> SWEBenchInstance {
    SWEBenchInstance {
        instance_id: "django__django-11133".to_string(),
        repo: "django/django".to_string(),
        base_commit: "abc123def456".to_string(),
        problem_statement: "HttpResponse doesn't handle memoryview objects".to_string(),
        patch: "".to_string(),
        test_patch: "".to_string(),
        fail_to_pass: "[\"test_memoryview\"]".to_string(),
        pass_to_pass: "[\"test_existing\"]".to_string(),
        version: "3.0".to_string(),
        environment_setup_commit: "def456".to_string(),
        hints_text: None,
        created_at: None,
    }
}

#[test]
fn test_scenario_creation() {
    let instance = mock_instance();
    let scenario = SWEBenchScenario::new(instance);

    assert_eq!(scenario.name(), "django__django-11133");
    assert_eq!(
        scenario.description(),
        "SWE-bench software engineering task"
    );
    assert_eq!(scenario.testbed(), "swebench");
    assert!(scenario.prompt().contains("django/django"));
    assert!(scenario.prompt().contains("HttpResponse"));
}

#[test]
fn test_prompt_formatting() {
    let mut instance = mock_instance();
    instance.hints_text = Some("Try looking at the make_bytes method".to_string());

    let scenario = SWEBenchScenario::new(instance);
    let prompt = scenario.prompt();

    assert!(prompt.contains("## Repository"));
    assert!(prompt.contains("## Problem Statement"));
    assert!(prompt.contains("## Hints"));
    assert!(prompt.contains("## Approach"));
    assert!(prompt.contains("## Success Criteria"));
}
