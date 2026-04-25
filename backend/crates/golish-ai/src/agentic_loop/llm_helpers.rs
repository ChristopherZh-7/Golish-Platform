use std::sync::Arc;
use tokio::sync::RwLock;
use rig::one_or_many::OneOrMany;
use rig::completion::Message;

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

    let user_msg = format!(
        "Tool: {}\n\nOutput to summarize:\n{}",
        tool_name, content
    );

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
pub(super) async fn mentor_one_shot(
    client: &Arc<RwLock<golish_llm_providers::LlmClient>>,
    system_prompt: &str,
    user_message: &str,
) -> anyhow::Result<String> {
    use rig::completion::{AssistantContent, CompletionModel as _, CompletionRequest};
    use rig::message::{Text as RigText, UserContent};

    let request = CompletionRequest {
        preamble: Some(system_prompt.to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(RigText {
                text: user_message.to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.4),
        max_tokens: Some(500),
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    let client_guard = client.read().await;

    macro_rules! complete {
        ($model:expr) => {{
            let response = $model
                .completion(request)
                .await
                .map_err(|e| anyhow::anyhow!("Mentor LLM call failed: {}", e))?;
            let text = response
                .choice
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            Ok(text)
        }};
    }

    match &*client_guard {
        golish_llm_providers::LlmClient::VertexAnthropic(m) => complete!(m),
        golish_llm_providers::LlmClient::VertexGemini(m) => complete!(m),
        golish_llm_providers::LlmClient::RigOpenRouter(m) => complete!(m),
        golish_llm_providers::LlmClient::RigOpenAi(m) => complete!(m),
        golish_llm_providers::LlmClient::RigOpenAiResponses(m) => complete!(m),
        golish_llm_providers::LlmClient::OpenAiReasoning(m) => complete!(m),
        golish_llm_providers::LlmClient::RigAnthropic(m) => complete!(m),
        golish_llm_providers::LlmClient::RigOllama(m) => complete!(m),
        golish_llm_providers::LlmClient::RigGemini(m) => complete!(m),
        golish_llm_providers::LlmClient::RigGroq(m) => complete!(m),
        golish_llm_providers::LlmClient::RigXai(m) => complete!(m),
        golish_llm_providers::LlmClient::RigZaiSdk(m) => complete!(m),
        golish_llm_providers::LlmClient::RigNvidia(m) => complete!(m),
        golish_llm_providers::LlmClient::Mock => Err(anyhow::anyhow!("Mock client")),
    }
}
