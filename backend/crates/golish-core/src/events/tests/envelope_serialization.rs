use super::*;

#[test]
fn envelope_serializes_with_flattened_event() {
    let envelope = AiEventEnvelope {
        seq: 42,
        ts: "2024-01-15T10:30:00Z".to_string(),
        event: AiEvent::Started {
            turn_id: "turn-1".to_string(),
        },
    };
    let json = serde_json::to_string(&envelope).unwrap();

    // Verify seq and type are both present at the top level
    assert!(json.contains("\"seq\":42"));
    assert!(json.contains("\"type\":\"started\""));
    assert!(json.contains("\"turn_id\":\"turn-1\""));
    assert!(json.contains("\"ts\":\"2024-01-15T10:30:00Z\""));
}

#[test]
fn envelope_deserializes_correctly() {
    let json =
        r#"{"seq":42,"ts":"2024-01-15T10:30:00Z","type":"started","turn_id":"turn-1"}"#;
    let envelope: AiEventEnvelope = serde_json::from_str(json).unwrap();

    assert_eq!(envelope.seq, 42);
    assert_eq!(envelope.ts, "2024-01-15T10:30:00Z");
    if let AiEvent::Started { turn_id } = envelope.event {
        assert_eq!(turn_id, "turn-1");
    } else {
        panic!("Expected Started event");
    }
}

#[test]
fn envelope_with_complex_event() {
    let envelope = AiEventEnvelope {
        seq: 100,
        ts: "2024-01-15T10:30:00Z".to_string(),
        event: AiEvent::ToolResult {
            tool_name: "read_file".to_string(),
            result: json!({"content": "file contents"}),
            success: true,
            request_id: "req-123".to_string(),
            source: ToolSource::Main,
        },
    };

    let json = serde_json::to_value(&envelope).unwrap();

    // Verify envelope fields
    assert_eq!(json["seq"], 100);
    assert_eq!(json["ts"], "2024-01-15T10:30:00Z");

    // Verify flattened event fields
    assert_eq!(json["type"], "tool_result");
    assert_eq!(json["tool_name"], "read_file");
    assert_eq!(json["success"], true);
    assert_eq!(json["request_id"], "req-123");
    assert_eq!(json["result"]["content"], "file contents");
}

#[test]
fn envelope_roundtrip() {
    let original = AiEventEnvelope {
        seq: 999,
        ts: "2024-01-15T10:30:00Z".to_string(),
        event: AiEvent::Completed {
            response: "Done".to_string(),
            reasoning: Some("Thought process".to_string()),
            input_tokens: Some(100),
            output_tokens: Some(50),
            duration_ms: Some(1500),
        },
    };

    let json_str = serde_json::to_string(&original).unwrap();
    let roundtrip: AiEventEnvelope = serde_json::from_str(&json_str).unwrap();

    assert_eq!(roundtrip.seq, original.seq);
    assert_eq!(roundtrip.ts, original.ts);

    // Verify the roundtrip produces identical JSON
    let original_json = serde_json::to_value(&original).unwrap();
    let roundtrip_json = serde_json::to_value(&roundtrip).unwrap();
    assert_eq!(original_json, roundtrip_json);
}
