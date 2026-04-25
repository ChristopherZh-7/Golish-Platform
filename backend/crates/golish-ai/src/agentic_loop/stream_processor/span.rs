//! Helpers that decorate the LLM span with the model's text/reasoning
//! output, so traces in Langfuse show what the model produced.

use rig::message::ToolCall;


/// Record a (truncated) version of the model's text output (or just the tool
/// names if it produced only tool calls) on the LLM span for Langfuse.
pub(super) fn record_completion_for_span(
    llm_span: &tracing::Span,
    text_content: &str,
    tool_calls_to_execute: &[ToolCall],
) {
    let completion_for_span = if !text_content.is_empty() {
        // Walk back to a char boundary before truncating
        let mut end = text_content.len().min(2000);
        while end > 0 && !text_content.is_char_boundary(end) {
            end -= 1;
        }
        if text_content.len() > 2000 {
            format!("{}... [truncated]", &text_content[..end])
        } else {
            text_content.to_string()
        }
    } else if !tool_calls_to_execute.is_empty() {
        // Tool-call only turns (common for GPT-5.2 / Codex): record tool names so
        // the span isn't empty and traces show what the model decided to do.
        let names: Vec<&str> = tool_calls_to_execute
            .iter()
            .map(|tc| tc.function.name.as_str())
            .collect();
        format!("[tool_calls: {}]", names.join(", "))
    } else {
        String::new()
    };
    if !completion_for_span.is_empty() {
        llm_span.record("gen_ai.completion", completion_for_span.as_str());
        llm_span.record("langfuse.observation.output", completion_for_span.as_str());
    }
}

/// Record the (truncated) reasoning trace on the span so Langfuse shows what
/// the model was thinking — the same content the UI displays in its
/// ThinkingBlock.
pub(super) fn record_reasoning_for_span(llm_span: &tracing::Span, thinking_content: &str) {
    if thinking_content.is_empty() {
        return;
    }

    let mut end = thinking_content.len().min(2000);
    while end > 0 && !thinking_content.is_char_boundary(end) {
        end -= 1;
    }
    let reasoning_for_span = if thinking_content.len() > 2000 {
        format!("{}... [truncated]", &thinking_content[..end])
    } else {
        thinking_content.to_string()
    };
    llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
}
