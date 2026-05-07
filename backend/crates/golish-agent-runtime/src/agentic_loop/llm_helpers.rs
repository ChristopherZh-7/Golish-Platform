use std::sync::Arc;
use tokio::sync::RwLock;

/// Summarize large tool output using a one-shot LLM call.
///
/// Preserves key data (IPs, ports, versions, URLs, errors) while removing noise.
/// Falls back to truncated content on failure.
pub(super) async fn summarize_tool_output(
    client: &Arc<RwLock<golish_llm_providers::LlmClient>>,
    tool_name: &str,
    content: &str,
) -> anyhow::Result<String> {
    let system = r#"You are a technical output summarizer for a penetration testing agent.
Summarize the tool output below, preserving ALL:
- IP addresses, hostnames, domain names
- Port numbers and service versions
- HTTP status codes and response headers
- Error messages and warnings
- Vulnerability identifiers (CVE, CWE)
- Credentials, tokens, or sensitive data found
- File paths and URLs

Remove: redundant lines, progress bars, banner art, duplicate entries, verbose formatting.
Output a clean, structured summary. Keep it under 800 tokens."#;

    let user_msg = format!("Tool: {}\n\nOutput to summarize:\n{}", tool_name, content);

    let summary = mentor_one_shot(client, system, &user_msg).await?;
    if summary.trim().is_empty() {
        return Err(anyhow::anyhow!("LLM returned empty summary"));
    }
    Ok(format!(
        "[LLM-summarized output from '{}']\n\n{}\n\n[End of summary — original output was {} chars]",
        tool_name,
        summary.trim(),
        content.len()
    ))
}

/// One-shot LLM completion for the Execution Mentor.
///
/// Uses the session's model to generate strategic advice when the agent is stuck.
/// Delegates to [`LlmClient::one_shot_completion`] which centralizes provider dispatch.
pub(super) async fn mentor_one_shot(
    client: &Arc<RwLock<golish_llm_providers::LlmClient>>,
    system_prompt: &str,
    user_message: &str,
) -> anyhow::Result<String> {
    let client_guard = client.read().await;
    client_guard
        .one_shot_completion(system_prompt, user_message, Some(0.4f64), Some(500))
        .await
}
