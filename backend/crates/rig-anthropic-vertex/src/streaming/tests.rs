use crate::types::{MessageDeltaContent, StopReason, StreamEvent, StreamMessageStart, Usage};

use super::{StreamChunk, StreamingResponse};

/// Helper to create a mock `StreamingResponse` for testing
/// `event_to_chunk`. We can't easily create a real one without a
/// `reqwest::Response`, so we test the token-tracking logic directly.
fn create_test_response() -> StreamingResponse {
    StreamingResponse {
        inner: Box::pin(futures::stream::empty()),
        buffer: String::new(),
        accumulated_text: String::new(),
        accumulated_signature: String::new(),
        done: false,
        input_tokens: None,
    }
}

#[test]
fn test_message_start_captures_input_tokens() {
    let mut response = create_test_response();

    let message_start = StreamEvent::MessageStart {
        message: StreamMessageStart {
            id: "msg_123".to_string(),
            message_type: "message".to_string(),
            role: "assistant".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            usage: Usage {
                input_tokens: 15000,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
    };

    let chunk = response.event_to_chunk(message_start);

    // MessageStart should not produce a chunk (returns None).
    assert!(chunk.is_none());
    // But it should capture the input_tokens.
    assert_eq!(response.input_tokens, Some(15000));
}

#[test]
fn test_message_delta_combines_tokens() {
    let mut response = create_test_response();

    response.input_tokens = Some(12500);

    let message_delta = StreamEvent::MessageDelta {
        delta: MessageDeltaContent {
            stop_reason: Some(StopReason::EndTurn),
            stop_sequence: None,
        },
        usage: Usage {
            input_tokens: 0,
            output_tokens: 450,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };

    let chunk = response.event_to_chunk(message_delta);

    assert!(chunk.is_some());
    if let Some(StreamChunk::Done { usage, .. }) = chunk {
        let usage = usage.expect("Usage should be present");
        assert_eq!(usage.input_tokens, 12500);
        assert_eq!(usage.output_tokens, 450);
    } else {
        panic!("Expected StreamChunk::Done");
    }
}

#[test]
fn test_message_delta_without_message_start() {
    let mut response = create_test_response();

    let message_delta = StreamEvent::MessageDelta {
        delta: MessageDeltaContent {
            stop_reason: Some(StopReason::EndTurn),
            stop_sequence: None,
        },
        usage: Usage {
            input_tokens: 0,
            output_tokens: 300,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };

    let chunk = response.event_to_chunk(message_delta);

    if let Some(StreamChunk::Done { usage, .. }) = chunk {
        let usage = usage.expect("Usage should be present");
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 300);
    } else {
        panic!("Expected StreamChunk::Done");
    }
}

#[test]
fn test_full_streaming_sequence_token_tracking() {
    let mut response = create_test_response();

    // 1. MessageStart.
    let _ = response.event_to_chunk(StreamEvent::MessageStart {
        message: StreamMessageStart {
            id: "msg_test".to_string(),
            message_type: "message".to_string(),
            role: "assistant".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            usage: Usage {
                input_tokens: 8500,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
    });
    assert_eq!(response.input_tokens, Some(8500));

    // 5. MessageDelta (final).
    let chunk = response.event_to_chunk(StreamEvent::MessageDelta {
        delta: MessageDeltaContent {
            stop_reason: Some(StopReason::EndTurn),
            stop_sequence: None,
        },
        usage: Usage {
            input_tokens: 0,
            output_tokens: 275,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    });

    if let Some(StreamChunk::Done { usage, .. }) = chunk {
        let usage = usage.expect("Usage should be present");
        assert_eq!(usage.input_tokens, 8500, "input_tokens from MessageStart");
        assert_eq!(usage.output_tokens, 275, "output_tokens from MessageDelta");
    } else {
        panic!("Expected StreamChunk::Done");
    }
}

#[test]
fn test_message_delta_with_input_tokens() {
    let mut response = create_test_response();

    response.input_tokens = Some(5000);

    // Newer API behavior: MessageDelta includes input_tokens. This
    // should take precedence over the MessageStart value.
    let message_delta = StreamEvent::MessageDelta {
        delta: MessageDeltaContent {
            stop_reason: Some(StopReason::EndTurn),
            stop_sequence: None,
        },
        usage: Usage {
            input_tokens: 15672,
            output_tokens: 408,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };

    let chunk = response.event_to_chunk(message_delta);

    if let Some(StreamChunk::Done { usage, .. }) = chunk {
        let usage = usage.expect("Usage should be present");
        assert_eq!(usage.input_tokens, 15672);
        assert_eq!(usage.output_tokens, 408);
    } else {
        panic!("Expected StreamChunk::Done");
    }
}

#[test]
fn test_usage_struct_serialization() {
    let usage = Usage {
        input_tokens: 50000,
        output_tokens: 1500,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    };

    let json = serde_json::to_string(&usage).unwrap();
    assert!(json.contains("\"input_tokens\":50000"));
    assert!(json.contains("\"output_tokens\":1500"));

    let parsed: Usage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.input_tokens, 50000);
    assert_eq!(parsed.output_tokens, 1500);
}

#[test]
fn test_usage_default_for_missing_fields() {
    // Anthropic sometimes omits input_tokens in message_delta —
    // verify `serde(default)` works.
    let json = r#"{"output_tokens": 200}"#;
    let usage: Usage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.input_tokens, 0);
    assert_eq!(usage.output_tokens, 200);
}
