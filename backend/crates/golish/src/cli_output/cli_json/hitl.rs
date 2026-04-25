//! Human-in-the-loop events: ask-human request / response.

pub(super) fn ask_human_request(
    request_id: &str,
    question: &str,
    input_type: &str,
    options: &[String],
    context: &str,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "question": question,
        "input_type": input_type,
        "options": options,
        "context": context
    })
}

pub(super) fn ask_human_response(
    request_id: &str,
    response: &str,
    skipped: bool,
) -> serde_json::Value {
    serde_json::json!({
        "request_id": request_id,
        "response": response,
        "skipped": skipped
    })
}
