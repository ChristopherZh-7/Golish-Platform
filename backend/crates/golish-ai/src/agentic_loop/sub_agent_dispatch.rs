//! Sub-agent dispatch helpers.
//!
//! Extracted from the main agentic loop to reduce file size.
//! Contains the LlmClient → model type dispatch for sub-agent execution
//! and the orchestrator briefing builder.

use std::collections::HashMap;

use golish_sub_agents::{execute_sub_agent, SubAgentContext, SubAgentExecutorContext};

use crate::tool_provider_impl::DefaultToolProvider;

/// Execute a sub-agent with an LlmClient by dispatching to the correct model type.
///
/// This function matches on the LlmClient variant and calls execute_sub_agent
/// with the appropriate inner model type.
pub(crate) async fn execute_sub_agent_with_client(
    agent_def: &golish_sub_agents::SubAgentDefinition,
    args: &serde_json::Value,
    context: &SubAgentContext,
    client: &golish_llm_providers::LlmClient,
    ctx: SubAgentExecutorContext<'_>,
    tool_provider: &DefaultToolProvider,
    parent_request_id: &str,
) -> anyhow::Result<golish_sub_agents::SubAgentResult> {
    use golish_llm_providers::LlmClient;

    match client {
        LlmClient::VertexAnthropic(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigOpenRouter(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigOpenAi(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigOpenAiResponses(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::OpenAiReasoning(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigAnthropic(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigOllama(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigGemini(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigGroq(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigXai(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigZaiSdk(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::RigNvidia(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::VertexGemini(model) => {
            execute_sub_agent(agent_def, args, context, model, ctx, tool_provider, parent_request_id).await
        }
        LlmClient::Mock => Err(anyhow::anyhow!("Cannot execute sub-agent with Mock client")),
    }
}

/// Build an orchestrator briefing for a sub-agent by querying shared memories
/// and active execution plans from the database.
///
/// Returns `None` if no relevant context is found or DB is not available,
/// avoiding unnecessary prompt inflation.
pub(crate) async fn build_sub_agent_briefing(
    db_tracker: Option<&crate::db_tracking::DbTracker>,
    agent_id: &str,
    task_description: &str,
) -> Option<String> {
    let tracker = db_tracker?;

    let keywords: Vec<&str> = task_description
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .take(3)
        .collect();

    if keywords.is_empty() {
        return None;
    }

    let (memories, plans) = tokio::join!(
        tracker.fetch_memories_for_briefing(&keywords, 5),
        tracker.fetch_active_plans(),
    );

    let has_memories = !memories.is_empty();
    let has_plans = !plans.is_empty();

    if !has_memories && !has_plans {
        return None;
    }

    let mut briefing = String::from("## Briefing from Orchestrator\n");

    if has_plans {
        briefing.push_str("\n### Active Execution Plans\n");
        for plan in &plans {
            briefing.push_str(&format!(
                "- **{}** (status: {}, step {}):\n",
                plan.title, plan.status, plan.current_step
            ));
            if let Some(desc) = &plan.description {
                briefing.push_str(&format!("  {}\n", desc));
            }
            if let Some(steps) = plan.steps.as_array() {
                for (i, step) in steps.iter().enumerate() {
                    let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed");
                    let status = step.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
                    let marker = if i as i32 == plan.current_step {
                        ">>>"
                    } else {
                        "   "
                    };
                    briefing.push_str(&format!(
                        "  {} {}. {} [{}]\n",
                        marker,
                        i + 1,
                        name,
                        status
                    ));
                }
            }
        }
    }

    if has_memories {
        briefing.push_str("\n### Relevant Findings from Other Agents\n");
        for mem in &memories {
            let content_preview = if mem.content.len() > 300 {
                format!(
                    "{}...",
                    &mem.content[..mem
                        .content
                        .char_indices()
                        .take_while(|(i, _)| *i < 300)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(300)]
                )
            } else {
                mem.content.clone()
            };
            briefing.push_str(&format!("- [{}] {}\n", mem.mem_type, content_preview));
        }
    }

    tracing::info!(
        "[briefing] Built briefing for sub-agent '{}': {} memories, {} plans, {} chars",
        agent_id,
        memories.len(),
        plans.len(),
        briefing.len()
    );

    Some(briefing)
}

/// Check if a tool call is a sub-agent invocation.
pub(crate) fn is_sub_agent_tool(tool_name: &str) -> bool {
    tool_name.starts_with("sub_agent_")
}

/// Partition tool calls into sub-agent calls and non-sub-agent calls,
/// preserving original indices for result ordering.
#[allow(clippy::type_complexity)]
pub(crate) fn partition_tool_calls(
    tool_calls: Vec<rig::message::ToolCall>,
) -> (Vec<(usize, rig::message::ToolCall)>, Vec<(usize, rig::message::ToolCall)>) {
    let mut sub_agent_calls = Vec::new();
    let mut other_calls = Vec::new();

    for (idx, tc) in tool_calls.into_iter().enumerate() {
        if is_sub_agent_tool(&tc.function.name) {
            sub_agent_calls.push((idx, tc));
        } else {
            other_calls.push((idx, tc));
        }
    }

    (sub_agent_calls, other_calls)
}

/// Detect repetitive text patterns that indicate degenerate model generation.
///
/// Splits text into sentences by common terminators and checks for repeated
/// sentence prefixes. Returns true if 3+ sentences share the same opening.
pub(crate) fn detect_repetitive_text(text: &str) -> bool {
    let char_count = text.chars().count();
    if char_count < 100 {
        return false;
    }

    let mut sentences = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        current.push(c);
        if matches!(c, '。' | '！' | '？' | '\n') {
            let trimmed = current.trim().to_string();
            let trimmed_chars = trimmed.chars().count();
            if trimmed_chars >= 8 {
                sentences.push(trimmed);
            }
            current = String::new();
        }
    }

    if sentences.len() < 3 {
        return false;
    }

    let mut fingerprints: HashMap<String, usize> = HashMap::new();
    for sentence in &sentences {
        let fp: String = sentence.chars().take(12).collect();
        *fingerprints.entry(fp).or_default() += 1;
    }
    if fingerprints.values().any(|&count| count >= 3) {
        return true;
    }

    if char_count >= 200 {
        let chars: Vec<char> = text.chars().collect();
        let window_size = 40.min(chars.len() / 4);
        if window_size >= 20 {
            let tail: String = chars[chars.len() - window_size..].iter().collect();
            let head: String = chars[..chars.len() - window_size].iter().collect();
            if head.matches(&tail).count() >= 2 {
                return true;
            }
        }
    }

    false
}
