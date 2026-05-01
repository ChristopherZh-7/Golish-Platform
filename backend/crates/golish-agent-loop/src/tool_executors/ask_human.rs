use serde_json::json;
use golish_core::events::AiEvent;

/// Execute the ask_human barrier tool.
///
/// Emits an AskHumanRequest event to the frontend, pauses the agentic loop,
/// and waits for the user to respond. Uses the same coordinator/oneshot
/// pattern as HITL tool approval.
pub async fn execute_ask_human_tool(
    args: &serde_json::Value,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    coordinator: Option<&crate::event_coordinator::CoordinatorHandle>,
    pending_approvals: &tokio::sync::RwLock<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<golish_core::hitl::ApprovalDecision>>,
    >,
) -> (serde_json::Value, bool) {
    let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("I need your input.");
    let input_type = args.get("input_type").and_then(|v| v.as_str()).unwrap_or("freetext");
    let options: Vec<String> = args.get("options")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let request_id = uuid::Uuid::new_v4().to_string();

    let rx = if let Some(coord) = coordinator {
        coord.register_approval(request_id.clone())
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = pending_approvals.write().await;
            pending.insert(request_id.clone(), tx);
        }
        rx
    };

    let _ = event_tx.send(AiEvent::AskHumanRequest {
        request_id: request_id.clone(),
        question: question.to_string(),
        input_type: input_type.to_string(),
        options,
        context,
    });

    tracing::info!("[ask_human] Waiting for user response: id={}, type={}", request_id, input_type);

    const ASK_HUMAN_TIMEOUT_SECS: u64 = 600;

    match tokio::time::timeout(
        std::time::Duration::from_secs(ASK_HUMAN_TIMEOUT_SECS),
        rx,
    ).await {
        Ok(Ok(decision)) => {
            let _ = event_tx.send(AiEvent::AskHumanResponse {
                request_id,
                response: decision.reason.clone().unwrap_or_default(),
                skipped: !decision.approved,
            });

            if decision.approved {
                (json!({
                    "response": decision.reason.unwrap_or_default(),
                    "skipped": false,
                }), true)
            } else {
                (json!({
                    "skipped": true,
                    "message": "User chose to skip this request. Adapt your approach accordingly.",
                }), true)
            }
        }
        Ok(Err(_)) => {
            (json!({
                "error": "Request was cancelled",
                "skipped": true,
            }), false)
        }
        Err(_) => {
            tracing::warn!("[ask_human] Timed out after {}s", ASK_HUMAN_TIMEOUT_SECS);
            if coordinator.is_none() {
                let mut pending = pending_approvals.write().await;
                pending.remove(&request_id);
            }
            (json!({
                "error": format!("No response within {} seconds", ASK_HUMAN_TIMEOUT_SECS),
                "timeout": true,
                "skipped": true,
            }), false)
        }
    }
}
