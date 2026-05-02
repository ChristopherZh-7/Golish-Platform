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
