use super::*;
use serde_json::json;

#[test]
fn test_default_config() {
    let config = LoopProtectionConfig::default();
    assert_eq!(config.max_tool_loops, 100);
    assert_eq!(config.max_repeated_tool_calls, 5);
    assert!(config.enabled);
}

#[test]
fn test_allowed_calls() {
    let mut detector = LoopDetector::with_defaults();

    for i in 0..3 {
        let result =
            detector.record_tool_call("read_file", &json!({"path": format!("file{}.txt", i)}));
        assert!(result.is_allowed());
        assert!(!result.is_blocked());
    }
}

#[test]
fn test_warning_threshold() {
    let mut detector = LoopDetector::with_defaults();
    let args = json!({"path": "same_file.txt"});

    for i in 0..3 {
        let result = detector.record_tool_call("read_file", &args);
        if i < 2 {
            assert_eq!(result, LoopDetectionResult::Allowed);
        } else {
            assert!(matches!(result, LoopDetectionResult::Warning { .. }));
        }
    }
}

#[test]
fn test_blocked_at_threshold() {
    let config = LoopProtectionConfig {
        max_repeated_tool_calls: 3,
        ..Default::default()
    };
    let mut detector = LoopDetector::new(config);
    let args = json!({"command": "ls -la"});

    for _ in 0..3 {
        let result = detector.record_tool_call("run_pty_cmd", &args);
        assert!(result.is_allowed());
    }

    let result = detector.record_tool_call("run_pty_cmd", &args);
    assert!(result.is_blocked());
    assert!(matches!(
        result,
        LoopDetectionResult::Blocked {
            repeat_count: 4,
            ..
        }
    ));
}

#[test]
fn test_max_iterations_reached() {
    let config = LoopProtectionConfig {
        max_tool_loops: 5,
        ..Default::default()
    };
    let mut detector = LoopDetector::new(config);

    for i in 0..5 {
        let result = detector.record_tool_call("tool", &json!({"i": i}));
        assert!(result.is_allowed());
    }

    let result = detector.record_tool_call("tool", &json!({"i": 5}));
    assert!(matches!(
        result,
        LoopDetectionResult::MaxIterationsReached { .. }
    ));
}

#[test]
fn test_reset_clears_counts() {
    let mut detector = LoopDetector::with_defaults();
    let args = json!({"path": "file.txt"});

    detector.record_tool_call("read_file", &args);
    detector.record_tool_call("read_file", &args);
    assert_eq!(detector.iteration_count(), 2);

    detector.reset();
    assert_eq!(detector.iteration_count(), 0);

    let result = detector.record_tool_call("read_file", &args);
    assert_eq!(result, LoopDetectionResult::Allowed);
}

#[test]
fn test_disabled_for_session() {
    let config = LoopProtectionConfig {
        max_repeated_tool_calls: 2,
        ..Default::default()
    };
    let mut detector = LoopDetector::new(config);
    let args = json!({"x": 1});

    detector.record_tool_call("tool", &args);
    detector.record_tool_call("tool", &args);

    detector.disable_for_session();
    assert!(!detector.is_enabled());

    let result = detector.record_tool_call("tool", &args);
    assert!(result.is_allowed());
}

#[test]
fn test_different_args_not_counted() {
    let config = LoopProtectionConfig {
        max_repeated_tool_calls: 2,
        ..Default::default()
    };
    let mut detector = LoopDetector::new(config);

    for i in 0..10 {
        let result =
            detector.record_tool_call("read_file", &json!({"path": format!("file{}.txt", i)}));
        assert!(
            result.is_allowed()
                || matches!(result, LoopDetectionResult::MaxIterationsReached { .. })
        );
    }
}

#[test]
fn test_stats() {
    let mut detector = LoopDetector::with_defaults();

    detector.record_tool_call("read_file", &json!({"path": "a.txt"}));
    detector.record_tool_call("read_file", &json!({"path": "a.txt"}));
    detector.record_tool_call("write_file", &json!({"path": "b.txt"}));

    let stats = detector.stats();
    assert_eq!(stats.iteration_count, 3);
    assert_eq!(stats.unique_signatures, 2);
    assert_eq!(stats.most_repeated_tool, Some("read_file".to_string()));
    assert_eq!(stats.most_repeated_count, 2);
}

#[test]
fn execution_monitor_modes_are_explicit() {
    assert_eq!(
        ExecutionMonitor::shadow().mode(),
        ExecutionMonitorMode::Shadow
    );
    assert_eq!(
        ExecutionMonitor::soft_inject().mode(),
        ExecutionMonitorMode::SoftInject
    );
    assert_eq!(
        ExecutionMonitor::hard_inject().mode(),
        ExecutionMonitorMode::HardInject
    );
    assert!(!ExecutionMonitorMode::Shadow.injects());
    assert!(ExecutionMonitorMode::SoftInject.injects());
    assert!(ExecutionMonitorMode::HardInject.injects());
    // Preserve the pre-existing constructor semantics for callers that
    // explicitly instantiate a monitor: a monitor means soft mentor injection.
    assert_eq!(
        ExecutionMonitor::new().mode(),
        ExecutionMonitorMode::SoftInject
    );
}

#[test]
fn execution_monitor_does_not_trigger_on_successful_batch_scans() {
    let mut monitor = ExecutionMonitor::shadow();
    assert!(!monitor.record_result_and_check("whatweb", "https://a.test", true, "{}"));
    assert!(!monitor.record_result_and_check("whatweb", "https://b.test", true, "{}"));
    assert!(!monitor.record_result_and_check("whatweb", "https://c.test", true, "{}"));
    assert_eq!(monitor.repeated_tool_name(), "unknown");
    assert_eq!(monitor.same_tool_count(), 0);
    assert!(monitor.recent_calls_summary().contains("https://c.test"));
}

#[test]
fn execution_monitor_triggers_on_repeated_failed_pattern() {
    let mut monitor = ExecutionMonitor::shadow();
    let args = "nmap -bad-flag same-target";
    let result = r#"{"error":"invalid option -bad-flag"}"#;
    assert!(!monitor.record_result_and_check("pentest_run", args, false, result));
    assert!(!monitor.record_result_and_check("pentest_run", args, false, result));
    assert!(monitor.record_result_and_check("pentest_run", args, false, result));
    assert_eq!(monitor.repeated_tool_name(), "pentest_run");
    assert_eq!(monitor.same_tool_count(), 3);
    assert!(monitor.recent_calls_summary().contains("invalid option"));
}
