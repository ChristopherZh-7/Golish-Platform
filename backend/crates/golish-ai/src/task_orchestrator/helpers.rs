//! Free helper functions shared by [`TaskOrchestrator`] phases:
//! [`parse_agent_type`] maps the generator's free-form `agent` strings onto
//! the strongly-typed `AgentType` enum, [`truncate`] is a UTF-8-safe truncator
//! used everywhere subtask results are summarized for the UI, and
//! [`looks_like_text_only_response`] is a simple heuristic that detects
//! "I would do X" responses lacking evidence of actual tool execution.

pub(super) fn parse_agent_type(agent: &Option<String>) -> Option<golish_db::models::AgentType> {
    agent.as_ref().and_then(|a| match a.as_str() {
        "pentester" => Some(golish_db::models::AgentType::Pentester),
        "coder" => Some(golish_db::models::AgentType::Coder),
        "searcher" | "researcher" => Some(golish_db::models::AgentType::Searcher),
        "memorist" => Some(golish_db::models::AgentType::Memorist),
        "reporter" => Some(golish_db::models::AgentType::Reporter),
        "adviser" => Some(golish_db::models::AgentType::Adviser),
        _ => Some(golish_db::models::AgentType::Primary),
    })
}

pub(super) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Heuristic to detect responses that are purely descriptive text without
/// evidence of actual tool execution. PentAGI uses barrier functions to
/// enforce structured output; this is a lighter alternative.
pub(super) fn looks_like_text_only_response(response: &str) -> bool {
    let trimmed = response.trim();
    if trimmed.len() < 50 {
        return false;
    }

    // Markers that indicate real tool work was performed
    let tool_evidence = [
        "```",           // code blocks from tool output
        "scan result",
        "output:",
        "found ",
        "discovered ",
        "vulnerable",
        "port ",
        "service ",
        "HTTP/",
        "200 OK",
        "404",
        "nmap",
        "subfinder",
        "httpx",
        "nuclei",
        ".golish/",
        "successfully",
        "executed",
        "Error:",
    ];

    let lower = trimmed.to_lowercase();
    let has_evidence = tool_evidence
        .iter()
        .any(|marker| lower.contains(&marker.to_lowercase()));

    // Phrases that indicate the agent is describing rather than doing
    let description_phrases = [
        "i would",
        "i will",
        "i can",
        "let me",
        "we should",
        "we could",
        "the next step",
        "here's my plan",
        "i recommend",
    ];
    let has_description = description_phrases
        .iter()
        .any(|phrase| lower.contains(phrase));

    !has_evidence && has_description
}
