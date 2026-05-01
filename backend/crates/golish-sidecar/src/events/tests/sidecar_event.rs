use super::super::SidecarEvent;

#[test]
fn session_started_serializes_correctly() {
    let event = SidecarEvent::SessionStarted {
        session_id: "abc-123".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "session_started");
    assert_eq!(parsed["session_id"], "abc-123");
}

#[test]
fn session_ended_serializes_correctly() {
    let event = SidecarEvent::SessionEnded {
        session_id: "abc-123".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "session_ended");
    assert_eq!(parsed["session_id"], "abc-123");
}

#[test]
fn patch_created_serializes_correctly() {
    let event = SidecarEvent::PatchCreated {
        session_id: "abc-123".to_string(),
        patch_id: 1,
        subject: "feat: add authentication".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "patch_created");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["patch_id"], 1);
    assert_eq!(parsed["subject"], "feat: add authentication");
}

#[test]
fn patch_applied_serializes_correctly() {
    let event = SidecarEvent::PatchApplied {
        session_id: "abc-123".to_string(),
        patch_id: 1,
        commit_sha: "a1b2c3d".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "patch_applied");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["patch_id"], 1);
    assert_eq!(parsed["commit_sha"], "a1b2c3d");
}

#[test]
fn patch_discarded_serializes_correctly() {
    let event = SidecarEvent::PatchDiscarded {
        session_id: "abc-123".to_string(),
        patch_id: 1,
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "patch_discarded");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["patch_id"], 1);
}

#[test]
fn patch_message_updated_serializes_correctly() {
    let event = SidecarEvent::PatchMessageUpdated {
        session_id: "abc-123".to_string(),
        patch_id: 1,
        new_subject: "fix: correct the bug".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "patch_message_updated");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["patch_id"], 1);
    assert_eq!(parsed["new_subject"], "fix: correct the bug");
}

#[test]
fn artifact_created_serializes_correctly() {
    let event = SidecarEvent::ArtifactCreated {
        session_id: "abc-123".to_string(),
        filename: "README.md".to_string(),
        target: "/project/README.md".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "artifact_created");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["filename"], "README.md");
    assert_eq!(parsed["target"], "/project/README.md");
}

#[test]
fn artifact_applied_serializes_correctly() {
    let event = SidecarEvent::ArtifactApplied {
        session_id: "abc-123".to_string(),
        filename: "README.md".to_string(),
        target: "/project/README.md".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "artifact_applied");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["filename"], "README.md");
    assert_eq!(parsed["target"], "/project/README.md");
}

#[test]
fn artifact_discarded_serializes_correctly() {
    let event = SidecarEvent::ArtifactDiscarded {
        session_id: "abc-123".to_string(),
        filename: "README.md".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "artifact_discarded");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["filename"], "README.md");
}

#[test]
fn state_updated_serializes_correctly() {
    let event = SidecarEvent::StateUpdated {
        session_id: "abc-123".to_string(),
        backend: "VertexAnthropic".to_string(),
    };

    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["event_type"], "state_updated");
    assert_eq!(parsed["session_id"], "abc-123");
    assert_eq!(parsed["backend"], "VertexAnthropic");
}

#[test]
fn all_events_have_event_type_field() {
    // Verify that every variant serializes with an event_type field
    let events = vec![
        SidecarEvent::SessionStarted {
            session_id: "s".to_string(),
        },
        SidecarEvent::SessionEnded {
            session_id: "s".to_string(),
        },
        SidecarEvent::PatchCreated {
            session_id: "s".to_string(),
            patch_id: 1,
            subject: "sub".to_string(),
        },
        SidecarEvent::PatchApplied {
            session_id: "s".to_string(),
            patch_id: 1,
            commit_sha: "sha".to_string(),
        },
        SidecarEvent::PatchDiscarded {
            session_id: "s".to_string(),
            patch_id: 1,
        },
        SidecarEvent::PatchMessageUpdated {
            session_id: "s".to_string(),
            patch_id: 1,
            new_subject: "sub".to_string(),
        },
        SidecarEvent::ArtifactCreated {
            session_id: "s".to_string(),
            filename: "f".to_string(),
            target: "t".to_string(),
        },
        SidecarEvent::ArtifactApplied {
            session_id: "s".to_string(),
            filename: "f".to_string(),
            target: "t".to_string(),
        },
        SidecarEvent::ArtifactDiscarded {
            session_id: "s".to_string(),
            filename: "f".to_string(),
        },
        SidecarEvent::StateUpdated {
            session_id: "s".to_string(),
            backend: "VertexAnthropic".to_string(),
        },
    ];

    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("event_type").is_some(),
            "Event {:?} missing event_type field",
            event
        );
    }
}
