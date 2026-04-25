use crate::client::Client;
use crate::types;

use super::CompletionModel;

#[test]
fn test_temperature_clamping() {
    let clamp = |t: f64| -> f32 {
        let t = t as f32;
        if t <= 0.0 {
            0.01
        } else if t >= 1.0 {
            0.99
        } else {
            t
        }
    };

    assert_eq!(clamp(0.0), 0.01);
    assert_eq!(clamp(1.0), 0.99);
    assert_eq!(clamp(0.7), 0.7);
    assert_eq!(clamp(-0.5), 0.01);
    assert_eq!(clamp(1.5), 0.99);
}

#[test]
fn test_zai_request_defaults() {
    let req = types::CompletionRequest::default();
    assert!(req.thinking.is_some());
    assert_eq!(req.thinking.as_ref().unwrap().thinking_type, "enabled");
    assert_eq!(req.stream, None);
    assert_eq!(req.tool_stream, None);
}

#[test]
fn test_client_creation() {
    let client = Client::new("test-key");
    assert_eq!(client.api_key(), "test-key");
}

#[test]
fn test_completion_model_creation() {
    let client = Client::new("test-key");
    let model = CompletionModel::new(client, "glm-4".to_string());
    assert_eq!(model.model(), "glm-4");
}

#[test]
fn test_message_conversion() {
    let user_msg = types::Message::user("Hello");
    assert_eq!(user_msg.role, types::Role::User);
    match user_msg.content {
        types::MessageContent::Text(s) => assert_eq!(s, "Hello"),
        _ => panic!("Expected text content"),
    }

    let asst_msg = types::Message::assistant("Hi there");
    assert_eq!(asst_msg.role, types::Role::Assistant);
    match asst_msg.content {
        types::MessageContent::Text(s) => assert_eq!(s, "Hi there"),
        _ => panic!("Expected text content"),
    }

    let tool_msg = types::Message::tool_result("call_123", "Result data");
    assert_eq!(tool_msg.role, types::Role::Tool);
    assert_eq!(tool_msg.tool_call_id, Some("call_123".to_string()));
}
