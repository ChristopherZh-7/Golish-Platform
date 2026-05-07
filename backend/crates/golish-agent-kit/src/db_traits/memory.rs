//! Memory gatekeeper: pure logic for deciding what to store and how to
//! format it. No database dependency.

use super::types::{MemoryType, ToolcallStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreDecision {
    Store(MemoryType),
    StoreSummary(MemoryType),
    Skip,
}

/// Determine whether a tool's result should be stored.
pub fn should_store(tool_name: &str, status: ToolcallStatus) -> StoreDecision {
    match (tool_name, status) {
        ("run_command", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),
        ("bash" | "shell", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),
        ("web_search" | "tavily_search" | "web_fetch", _) => {
            StoreDecision::Store(MemoryType::Observation)
        }
        ("write_file" | "edit_file" | "create_file", ToolcallStatus::Finished) => {
            StoreDecision::StoreSummary(MemoryType::Technique)
        }
        (
            "nmap" | "nikto" | "sqlmap" | "nuclei" | "ffuf" | "gobuster" | "dirsearch",
            ToolcallStatus::Finished,
        ) => StoreDecision::Store(MemoryType::Observation),
        _ if tool_name.starts_with("pentest_") && status == ToolcallStatus::Finished => {
            StoreDecision::Store(MemoryType::Vulnerability)
        }
        _ => StoreDecision::Skip,
    }
}

const MIN_CONTENT_LEN: usize = 50;
const MAX_CONTENT_LEN: usize = 8192;
const TRUNCATION_KEEP: usize = 3072;

/// Filter and clean content for memory storage.
pub fn filter_content(result: &str) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() || trimmed.len() < MIN_CONTENT_LEN {
        return None;
    }
    let cleaned = strip_ansi(trimmed);
    if cleaned.len() <= MAX_CONTENT_LEN {
        return Some(cleaned);
    }
    let head = &cleaned[..TRUNCATION_KEEP];
    let tail = &cleaned[cleaned.len() - 512..];
    Some(format!(
        "{}\n\n... [{} bytes omitted] ...\n\n{}",
        head,
        cleaned.len() - TRUNCATION_KEEP - 512,
        tail
    ))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for nc in chars.by_ref() {
                    if nc.is_ascii_alphabetic() || nc == 'm' {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Build a search-friendly markdown document from tool invocation details.
pub fn build_memory_content(tool_name: &str, args: &serde_json::Value, result: &str) -> String {
    match tool_name {
        "run_command" | "bash" | "shell" => {
            let cmd = extract_str(args, "command")
                .or_else(|| extract_str(args, "cmd"))
                .unwrap_or_default();
            format!("## Command Execution\n**Command:** `{cmd}`\n\n**Output:**\n```\n{result}\n```")
        }
        "web_search" | "tavily_search" => {
            let query = extract_str(args, "query").unwrap_or_default();
            format!("## Web Search\n**Query:** {query}\n\n**Results:**\n{result}")
        }
        "web_fetch" => {
            let url = extract_str(args, "url").unwrap_or_default();
            format!("## Web Fetch\n**URL:** {url}\n\n**Content:**\n{result}")
        }
        "write_file" | "create_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!("## File Created/Written\n**Path:** `{path}`\n\n**Summary:** {result}")
        }
        "edit_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!("## File Edited\n**Path:** `{path}`\n\n**Change:** {result}")
        }
        _ => {
            let args_preview = truncate_json(args, 300);
            format!("## {tool_name}\n**Args:** {args_preview}\n\n**Result:**\n{result}")
        }
    }
}

fn extract_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|v| v.as_str())
}

fn truncate_json(v: &serde_json::Value, max_len: usize) -> String {
    let s = v.to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}
