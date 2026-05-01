use super::*;

#[test]
fn test_loop_capture_context_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LoopCaptureContext>();
}

#[test]
fn test_loop_capture_context_shared_ref_process() {
    let ctx = LoopCaptureContext::new(None);
    let event = AiEvent::ToolRequest {
        request_id: "test".to_string(),
        tool_name: "read_file".to_string(),
        args: json!({}),
        source: golish_core::events::ToolSource::Main,
    };
    ctx.process(&event);
    ctx.process(&event);
}

#[tokio::test]
async fn test_loop_capture_context_concurrent_access() {
    let ctx = Arc::new(LoopCaptureContext::new(None));
    let mut handles = vec![];
    for i in 0..5 {
        let ctx = Arc::clone(&ctx);
        handles.push(tokio::spawn(async move {
            let event = AiEvent::ToolRequest {
                request_id: format!("req-{}", i),
                tool_name: "read_file".to_string(),
                args: json!({}),
                source: golish_core::events::ToolSource::Main,
            };
            ctx.process(&event);
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
}
