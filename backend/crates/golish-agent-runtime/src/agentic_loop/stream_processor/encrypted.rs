//! Extract `reasoning_encrypted_content` from the OpenAI Responses `Final`
//! payload.
//!
//! This field is required for stateless multi-turn conversations with
//! reasoning models, but it isn't exposed on the typed response struct — we
//! go through JSON serialization to read it.


/// Extract `reasoning_encrypted_content` from the `Final` payload of OpenAI's
/// Responses API. This field is required for stateless multi-turn conversations
/// with reasoning models, but it isn't surfaced on the typed response struct —
/// we go through JSON to read it.
pub(super) fn extract_openai_reasoning_encrypted_content<R: serde::Serialize>(
    resp: &R,
    thinking_id: &mut Option<String>,
    thinking_signature: &mut Option<String>,
) {
    let json_value = match serde_json::to_value(resp) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!("[OpenAI Debug] Failed to serialize Final response to JSON");
            return;
        }
    };

    let has_encrypted_field = json_value.get("reasoning_encrypted_content").is_some();
    tracing::info!(
        "[OpenAI Debug] Final response: has_reasoning_encrypted_content={}, thinking_id={:?}, thinking_signature_before={:?}",
        has_encrypted_field,
        thinking_id,
        thinking_signature.as_ref().map(|s| s.len())
    );

    let Some(encrypted_map) = json_value
        .get("reasoning_encrypted_content")
        .and_then(|v| v.as_object())
    else {
        return;
    };

    tracing::info!(
        "[OpenAI Debug] encrypted_map has {} entries: {:?}",
        encrypted_map.len(),
        encrypted_map.keys().collect::<Vec<_>>()
    );

    // If we already captured a thinking_id, look up its encrypted_content.
    if let Some(ref tid) = *thinking_id {
        if let Some(encrypted) = encrypted_map.get(tid).and_then(|v| v.as_str()) {
            tracing::info!(
                "[OpenAI Debug] Found encrypted_content for reasoning item {}: {} bytes",
                tid,
                encrypted.len()
            );
            *thinking_signature = Some(encrypted.to_string());
        } else {
            tracing::warn!(
                "[OpenAI Debug] thinking_id {} NOT FOUND in encrypted_map!",
                tid
            );
        }
    }

    // If we don't have a thinking_id but only one reasoning item is present,
    // adopt that one (common case: single reasoning block in a response).
    if thinking_signature.is_none() && encrypted_map.len() == 1 {
        if let Some((id, encrypted)) = encrypted_map.iter().next() {
            if let Some(encrypted_str) = encrypted.as_str() {
                tracing::info!(
                    "[OpenAI Debug] Using single encrypted_content for reasoning item {}: {} bytes",
                    id,
                    encrypted_str.len()
                );
                *thinking_signature = Some(encrypted_str.to_string());
                if thinking_id.is_none() {
                    *thinking_id = Some(id.clone());
                }
            }
        }
    }
}
