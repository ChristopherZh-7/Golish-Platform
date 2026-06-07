//! LLM stream processing for sub-agent execution.
//!
//! Extracts the streaming response processing loop from the main orchestrator,
//! handling text deltas, reasoning content, tool call accumulation, and idle
//! timeout detection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rig::message::{ToolCall, ToolFunction};
use rig::streaming::StreamedAssistantContent;
use uuid::Uuid;

use crate::executor_helpers::epoch_secs;
use golish_core::events::AiEvent;
use golish_llm_providers::{ProviderStreamQuirks, ReasoningHandling};

/// Accumulated result from processing an LLM streaming response.
pub(super) struct StreamResult {
    pub text_content: String,
    pub thinking_text: String,
    pub thinking_id: Option<String>,
    pub thinking_signature: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub has_tool_calls: bool,
    pub idle_timeout_hit: bool,
}

/// Process a streaming LLM response, accumulating text, thinking, and tool calls.
///
/// Handles text content deltas (emitted as `SubAgentTextDelta` events),
/// reasoning content (streamed and non-streamed), tool call accumulation
/// (complete and delta-based), idle timeout detection, encrypted reasoning
/// content from OpenAI Responses API, and pending tool call finalization.
///
/// Records reasoning content on `llm_span` before returning.
#[allow(clippy::too_many_arguments)]
pub(super) async fn process_llm_stream<S, R, E>(
    stream: &mut S,
    agent_id: &str,
    parent_request_id: &str,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AiEvent>,
    last_activity: &Arc<AtomicU64>,
    idle_timeout: Option<Duration>,
    llm_span: &tracing::Span,
    quirks: &ProviderStreamQuirks,
) -> StreamResult
where
    S: futures::Stream<Item = Result<StreamedAssistantContent<R>, E>> + Unpin,
    R: serde::Serialize + rig::completion::GetTokenUsage,
    E: std::fmt::Display,
{
    let mut text_content = String::new();
    let mut thinking_text = String::new();
    let mut thinking_signature: Option<String> = None;
    let mut thinking_id: Option<String> = None;
    let mut has_tool_calls = false;
    let mut tool_calls_to_execute: Vec<ToolCall> = vec![];

    let mut current_tool_id: Option<String> = None;
    let mut current_tool_call_id: Option<String> = None;
    let mut current_tool_name: Option<String> = None;
    let mut current_tool_args = String::new();

    last_activity.store(epoch_secs(), Ordering::Relaxed);

    let mut idle_timeout_hit = false;
    loop {
        let chunk_opt = if let Some(idle_dur) = idle_timeout {
            let last = last_activity.load(Ordering::Relaxed);
            let now = epoch_secs();
            let remaining = idle_dur.as_secs().saturating_sub(now.saturating_sub(last));

            if remaining == 0 {
                idle_timeout_hit = true;
                break;
            }

            match tokio::time::timeout(Duration::from_secs(remaining), stream.next()).await {
                Ok(v) => v,
                Err(_) => {
                    idle_timeout_hit = true;
                    break;
                }
            }
        } else {
            stream.next().await
        };

        let Some(chunk_result) = chunk_opt else {
            break;
        };

        last_activity.store(epoch_secs(), Ordering::Relaxed);

        match chunk_result {
            Ok(chunk) => match chunk {
                StreamedAssistantContent::Text(text_msg) => {
                    text_content.push_str(&text_msg.text);
                    let _ = event_tx.send(AiEvent::SubAgentTextDelta {
                        agent_id: agent_id.to_string(),
                        delta: text_msg.text,
                        accumulated: text_content.clone(),
                        parent_request_id: parent_request_id.to_string(),
                    });
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
                    let reasoning_collected = reasoning
                        .content
                        .iter()
                        .filter_map(|item| {
                            if let rig::message::ReasoningContent::Text { text, .. } = item {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    match quirks.reasoning_handling {
                        ReasoningHandling::AlwaysContent => {
                            if !reasoning_collected.is_empty() {
                                tracing::debug!(
                                    "[sub-agent] quirks=AlwaysContent: rerouting reasoning ({} chars) to text",
                                    reasoning_collected.len()
                                );
                                text_content.push_str(&reasoning_collected);
                                let _ = event_tx.send(AiEvent::SubAgentTextDelta {
                                    agent_id: agent_id.to_string(),
                                    delta: reasoning_collected,
                                    accumulated: text_content.clone(),
                                    parent_request_id: parent_request_id.to_string(),
                                });
                            }
                        }
                        ReasoningHandling::Standard | ReasoningHandling::FallbackToContent => {
                            for item in &reasoning.content {
                                if let rig::message::ReasoningContent::Text { text, signature } =
                                    item
                                {
                                    if !text.is_empty() {
                                        tracing::debug!(
                                            "[sub-agent] Thinking: {} chars",
                                            text.len()
                                        );
                                        thinking_text.push_str(text);
                                        let _ = event_tx.send(AiEvent::SubAgentReasoning {
                                            agent_id: agent_id.to_string(),
                                            delta: text.clone(),
                                            accumulated: thinking_text.clone(),
                                            parent_request_id: parent_request_id.to_string(),
                                        });
                                    }
                                    if signature.is_some() && thinking_signature.is_none() {
                                        thinking_signature = signature.clone();
                                    }
                                }
                            }
                            if reasoning.id.is_some() && thinking_id.is_none() {
                                thinking_id = reasoning.id.clone();
                            }
                        }
                    }
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                    match quirks.reasoning_handling {
                        ReasoningHandling::AlwaysContent => {
                            if !reasoning.is_empty() {
                                tracing::debug!(
                                    "[sub-agent] quirks=AlwaysContent: rerouting reasoning delta ({} chars) to text",
                                    reasoning.len()
                                );
                                text_content.push_str(&reasoning);
                                let _ = event_tx.send(AiEvent::SubAgentTextDelta {
                                    agent_id: agent_id.to_string(),
                                    delta: reasoning,
                                    accumulated: text_content.clone(),
                                    parent_request_id: parent_request_id.to_string(),
                                });
                            }
                        }
                        ReasoningHandling::Standard | ReasoningHandling::FallbackToContent => {
                            if !reasoning.is_empty() {
                                thinking_text.push_str(&reasoning);
                                let _ = event_tx.send(AiEvent::SubAgentReasoning {
                                    agent_id: agent_id.to_string(),
                                    delta: reasoning.clone(),
                                    accumulated: thinking_text.clone(),
                                    parent_request_id: parent_request_id.to_string(),
                                });
                            }
                            if id.is_some() && thinking_id.is_none() {
                                thinking_id = id;
                            }
                        }
                    }
                }
                StreamedAssistantContent::ToolCall { tool_call, .. } => {
                    tracing::debug!(
                        "[sub-agent] Received tool call: {} (id: {})",
                        tool_call.function.name,
                        tool_call.id
                    );

                    if let (Some(prev_id), Some(prev_name)) =
                        (current_tool_id.take(), current_tool_name.take())
                    {
                        let args = golish_json_repair::parse_tool_args(&current_tool_args);
                        tracing::debug!(
                            "[sub-agent] Finalizing previous tool call: {} with args: {}",
                            prev_name,
                            current_tool_args
                        );
                        has_tool_calls = true;
                        let prev_call_id = current_tool_call_id
                            .take()
                            .unwrap_or_else(|| prev_id.clone());
                        tool_calls_to_execute.push(ToolCall {
                            id: prev_id,
                            call_id: Some(prev_call_id),
                            function: ToolFunction {
                                name: prev_name,
                                arguments: args,
                            },
                            signature: None,
                            additional_params: None,
                        });
                        current_tool_args.clear();
                    }

                    // Treat string and empty args as incomplete streaming
                    // fragments. Some providers (e.g. Xiaomi MiMo) send the first
                    // partial JSON fragment as a string; dispatching it as-is
                    // hands malformed args to the tool handler (e.g.
                    // search_memories receiving `{"category": ` and failing with
                    // "requires a non-empty 'query'"). Seed the buffer and let
                    // deltas complete it, then parse on close.
                    let has_complete_args =
                        golish_core::has_complete_tool_args(&tool_call.function.arguments);

                    if has_complete_args {
                        tracing::debug!(
                            "[sub-agent] Tool call has complete args: {:?}",
                            tool_call.function.arguments
                        );
                        has_tool_calls = true;
                        let mut tc = tool_call;
                        if tc.call_id.is_none() {
                            tc.call_id = Some(tc.id.clone());
                        }
                        tool_calls_to_execute.push(tc);
                    } else {
                        tracing::debug!(
                            "[sub-agent] Tool call args incomplete, tracking for delta accumulation"
                        );
                        current_tool_id = Some(tool_call.id.clone());
                        current_tool_call_id = tool_call.call_id.clone();
                        current_tool_name = Some(tool_call.function.name.clone());
                        current_tool_args =
                            golish_core::initial_tool_args_fragment(&tool_call.function.arguments);
                    }
                }
                StreamedAssistantContent::ToolCallDelta { id, content, .. } => {
                    if current_tool_id.is_none() && !id.is_empty() {
                        current_tool_id = Some(id);
                    }
                    if let rig::streaming::ToolCallDeltaContent::Delta(delta) = content {
                        current_tool_args.push_str(&delta);
                    }
                }
                StreamedAssistantContent::Final(ref resp) => {
                    if let Some(usage) = resp.token_usage() {
                        llm_span.record("gen_ai.usage.prompt_tokens", usage.input_tokens as i64);
                        llm_span
                            .record("gen_ai.usage.completion_tokens", usage.output_tokens as i64);
                    }

                    if let Ok(json_value) = serde_json::to_value(resp) {
                        if let Some(encrypted_map) = json_value
                            .get("reasoning_encrypted_content")
                            .and_then(|v| v.as_object())
                        {
                            if let Some(ref tid) = thinking_id {
                                if let Some(encrypted) =
                                    encrypted_map.get(tid).and_then(|v| v.as_str())
                                {
                                    tracing::debug!(
                                        "[sub-agent] Found encrypted_content for reasoning item {}: {} bytes",
                                        tid,
                                        encrypted.len()
                                    );
                                    thinking_signature = Some(encrypted.to_string());
                                }
                            }
                            if thinking_signature.is_none() && encrypted_map.len() == 1 {
                                if let Some((id, encrypted)) = encrypted_map.iter().next() {
                                    if let Some(encrypted_str) = encrypted.as_str() {
                                        tracing::debug!(
                                            "[sub-agent] Using single encrypted_content for reasoning item {}: {} bytes",
                                            id,
                                            encrypted_str.len()
                                        );
                                        thinking_signature = Some(encrypted_str.to_string());
                                        if thinking_id.is_none() {
                                            thinking_id = Some(id.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
            Err(e) => {
                tracing::warn!("[sub-agent] Stream error: {}", e);
            }
        }
    }

    // Finalize any remaining pending tool call after stream ends
    if let (Some(prev_id), Some(prev_name)) = (current_tool_id.take(), current_tool_name.take()) {
        let args = golish_json_repair::parse_tool_args(&current_tool_args);
        tracing::debug!(
            "[sub-agent] Finalizing final tool call: {} with args: {}",
            prev_name,
            current_tool_args
        );
        has_tool_calls = true;
        let prev_call_id = current_tool_call_id
            .take()
            .unwrap_or_else(|| prev_id.clone());
        tool_calls_to_execute.push(ToolCall {
            id: prev_id,
            call_id: Some(prev_call_id),
            function: ToolFunction {
                name: prev_name,
                arguments: args,
            },
            signature: None,
            additional_params: None,
        });
    }

    // Textual tool-call finalization (compatibility adapter).
    //
    // Some model families (e.g. Xiaomi MiMo) emit tool calls as textual
    // `<tool_call><function=...>` markup instead of native structured calls, and
    // frequently BATCH several blocks in one turn. Stripping the markup is
    // unconditional so it never leaks into the response; recovering call(s) to
    // execute only happens when the provider produced no native tool calls (so
    // the same intent is not executed twice). Recover ALL of them — not just the
    // first — each with a distinct id so the loop balances one tool_result per
    // call; dropped calls otherwise force the model to re-issue them on later
    // turns. The `ask_human` barrier still collapses to a single call. Mirrors
    // the main agentic loop's `extract_textual_tool_calls` recovery.
    let recovered = if tool_calls_to_execute.is_empty() {
        golish_core::select_textual_tool_calls(&text_content)
    } else {
        Vec::new()
    };
    text_content = golish_core::strip_textual_tool_call_markup(&text_content);
    if !recovered.is_empty() {
        let tool_names: Vec<&str> = recovered.iter().map(|c| c.name.as_str()).collect();
        tracing::warn!(
            agent_id,
            recovered = recovered.len(),
            tools = ?tool_names,
            text_len = text_content.len(),
            "[sub-agent][tool-adapter] Recovered textual XML-style tool call(s) into structured tool call(s)"
        );
        // Index the synthetic id per call so a batched multi-call turn produces
        // distinct ids (the loop balances one tool_result per id).
        let batch = Uuid::new_v4();
        for (idx, call) in recovered.into_iter().enumerate() {
            let id = format!("textual-tool-call-{batch}-{idx}");
            tool_calls_to_execute.push(ToolCall {
                id: id.clone(),
                call_id: Some(id),
                function: ToolFunction {
                    name: call.name,
                    arguments: call.arguments,
                },
                signature: None,
                additional_params: None,
            });
        }
        has_tool_calls = true;
    }

    // Record reasoning/thinking content on the llm_completion span if present.
    if !thinking_text.is_empty() {
        let mut end = thinking_text.len().min(2000);
        while end > 0 && !thinking_text.is_char_boundary(end) {
            end -= 1;
        }
        let reasoning_for_span = if thinking_text.len() > 2000 {
            format!("{}... [truncated]", &thinking_text[..end])
        } else {
            thinking_text.clone()
        };
        llm_span.record("gen_ai.reasoning", reasoning_for_span.as_str());
    }

    StreamResult {
        text_content,
        thinking_text,
        thinking_id,
        thinking_signature,
        tool_calls: tool_calls_to_execute,
        has_tool_calls,
        idle_timeout_hit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct DummyResp;

    impl rig::completion::GetTokenUsage for DummyResp {
        fn token_usage(&self) -> Option<rig::completion::Usage> {
            None
        }
    }

    fn text_stream(
        text: &str,
    ) -> impl futures::Stream<Item = Result<StreamedAssistantContent<DummyResp>, String>> + Unpin
    {
        let chunks = vec![Ok(StreamedAssistantContent::Text(rig::message::Text {
            text: text.to_string(),
        }))];
        futures::stream::iter(chunks)
    }

    async fn run(text: &str) -> StreamResult {
        let mut stream = text_stream(text);
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let last_activity = Arc::new(AtomicU64::new(epoch_secs()));
        let quirks = golish_llm_providers::resolve_stream_quirks("xiaomi", "mimo-v2.5-pro", None);
        let span = tracing::Span::none();
        process_llm_stream(
            &mut stream,
            "pentester",
            "req-1",
            &event_tx,
            &last_activity,
            None,
            &span,
            &quirks,
        )
        .await
    }

    #[tokio::test]
    async fn recovers_mimo_textual_tool_call_in_sub_agent_stream() {
        let markup = "我先查一下 DNS。\n\
<tool_call>\n\
<function=run_command>\n\
<parameter=command>dig example.com</parameter>\n\
</function>\n\
</tool_call>";

        let sr = run(markup).await;

        assert!(sr.has_tool_calls, "expected recovered textual tool call");
        assert_eq!(sr.tool_calls.len(), 1);
        assert_eq!(sr.tool_calls[0].function.name, "run_command");
        assert_eq!(
            sr.tool_calls[0].function.arguments["command"],
            "dig example.com"
        );
        assert!(
            !sr.text_content.contains("<tool_call>") && !sr.text_content.contains("<function="),
            "markup should be stripped from text_content, got: {}",
            sr.text_content
        );
        assert!(sr.text_content.contains("我先查一下 DNS"));
    }

    #[tokio::test]
    async fn plain_prose_yields_no_recovered_tool_call() {
        let sr = run("just a normal answer, no tool markup").await;

        assert!(!sr.has_tool_calls);
        assert!(sr.tool_calls.is_empty());
        assert_eq!(sr.text_content, "just a normal answer, no tool markup");
    }

    #[tokio::test]
    async fn recovers_all_batched_mimo_textual_tool_calls_in_sub_agent_stream() {
        // MiMo batches several `<function=...>` blocks in one turn; the sub-agent
        // adapter must recover ALL of them (not just the first), each with a
        // distinct id so the loop balances one tool_result per call (the gap the
        // main loop already closed via select_textual_tool_calls).
        let markup = "先跑两步。\n\
<tool_call>\n\
<function=run_command>\n\
<parameter=command>dig example.com</parameter>\n\
</function>\n\
</tool_call>\n\
<tool_call>\n\
<function=update_plan>\n\
<parameter=plan>[{\"status\":\"in_progress\",\"step\":\"scan\"}]</parameter>\n\
</function>\n\
</tool_call>";

        let sr = run(markup).await;

        assert!(sr.has_tool_calls, "expected recovered textual tool calls");
        assert_eq!(
            sr.tool_calls.len(),
            2,
            "both batched calls must be recovered"
        );
        assert_eq!(sr.tool_calls[0].function.name, "run_command");
        assert_eq!(
            sr.tool_calls[0].function.arguments["command"],
            "dig example.com"
        );
        assert_eq!(sr.tool_calls[1].function.name, "update_plan");
        assert_ne!(
            sr.tool_calls[0].id, sr.tool_calls[1].id,
            "ids must be distinct per call so tool_results balance"
        );
        assert!(
            !sr.text_content.contains("<tool_call>") && !sr.text_content.contains("<function="),
            "markup should be stripped, got: {}",
            sr.text_content
        );
    }

    #[tokio::test]
    async fn ask_human_barrier_drops_follow_up_in_sub_agent_stream() {
        // An ask_human batched with a follow-up side effect: the barrier must
        // still win so a self-answered create cannot bypass the human gate.
        let markup = "需要确认。\n\
<tool_call>\n\
<function=ask_human>\n\
<parameter=question>确认候选单元?</parameter>\n\
<parameter=input_type>unit_review</parameter>\n\
</function>\n\
</tool_call>\n\
<tool_call>\n\
<function=manage_organizations>\n\
<parameter=action>create</parameter>\n\
<parameter=name>ACME Corp</parameter>\n\
</function>\n\
</tool_call>";

        let sr = run(markup).await;

        assert!(sr.has_tool_calls);
        assert_eq!(
            sr.tool_calls.len(),
            1,
            "ask_human barrier must drop the follow-up"
        );
        assert_eq!(sr.tool_calls[0].function.name, "ask_human");
        assert_eq!(
            sr.tool_calls[0].function.arguments["input_type"],
            "unit_review"
        );
    }
}
