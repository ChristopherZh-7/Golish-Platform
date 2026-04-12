//! Memory Gatekeeper: decides what tool results are worth persisting
//! as long-term semantic memories.
//!
//! Three-layer filtering pipeline:
//! 1. **Tool whitelist** — only certain tools produce storable output
//! 2. **Content quality** — empty, too-short, or low-signal results are dropped
//! 3. **Structured builder** — wraps tool args + result into a search-friendly document

use crate::models::{MemoryType, ToolcallStatus};

/// The gatekeeper's verdict for a given tool result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreDecision {
    /// Store the full result (after quality filtering).
    Store(MemoryType),
    /// Store a truncated/summarized version.
    StoreSummary(MemoryType),
    /// Do not store.
    Skip,
}

// ── Layer 1: Tool whitelist ─────────────────────────────────────────

/// Determine whether a tool's result should be stored based on
/// tool name and execution status.
pub fn should_store(tool_name: &str, status: ToolcallStatus) -> StoreDecision {
    match (tool_name, status) {
        // Terminal commands that succeeded
        ("run_command", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),
        ("bash" | "shell", ToolcallStatus::Finished) => StoreDecision::Store(MemoryType::Technique),

        // Web / search tools (always store regardless of status — even empty
        // results are a useful negative signal: "we searched X but found nothing")
        ("web_search" | "tavily_search" | "web_fetch", _) => {
            StoreDecision::Store(MemoryType::Observation)
        }

        // File mutations — only store a summary (path + what changed)
        ("write_file" | "edit_file" | "create_file", ToolcallStatus::Finished) => {
            StoreDecision::StoreSummary(MemoryType::Technique)
        }

        // Security-specific tools
        ("nmap" | "nikto" | "sqlmap" | "nuclei" | "ffuf" | "gobuster" | "dirsearch",
         ToolcallStatus::Finished) => {
            StoreDecision::Store(MemoryType::Observation)
        }

        // Pentest / exploitation results
        _ if tool_name.starts_with("pentest_") && status == ToolcallStatus::Finished => {
            StoreDecision::Store(MemoryType::Vulnerability)
        }

        // Everything else (read_file, list_directory, ast_grep, failed commands…)
        _ => StoreDecision::Skip,
    }
}

// ── Layer 2: Content quality filter ─────────────────────────────────

const MIN_CONTENT_LEN: usize = 50;
const MAX_CONTENT_LEN: usize = 8192;
const TRUNCATION_KEEP: usize = 3072;

/// Returns `None` if the content is too low-quality to store.
/// Otherwise returns a cleaned, possibly truncated version.
pub fn filter_content(result: &str) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() || trimmed.len() < MIN_CONTENT_LEN {
        return None;
    }

    let cleaned = strip_ansi(trimmed);

    if cleaned.len() <= MAX_CONTENT_LEN {
        return Some(cleaned);
    }

    // Keep head and tail with a truncation marker in the middle
    let head = &cleaned[..TRUNCATION_KEEP];
    let tail_start = cleaned.len().saturating_sub(TRUNCATION_KEEP);
    let tail = &cleaned[tail_start..];
    let omitted = cleaned.len() - TRUNCATION_KEEP * 2;
    Some(format!(
        "{head}\n\n... [{omitted} bytes omitted] ...\n\n{tail}"
    ))
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... <letter> sequences
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

// ── Layer 3: Structured content builder ─────────────────────────────

/// Build a search-friendly markdown document from tool invocation details.
pub fn build_memory_content(
    tool_name: &str,
    args: &serde_json::Value,
    result: &str,
) -> String {
    match tool_name {
        "run_command" | "bash" | "shell" => {
            let cmd = extract_str(args, "command")
                .or_else(|| extract_str(args, "cmd"))
                .unwrap_or_default();
            format!(
                "## Command Execution\n\
                 **Command:** `{cmd}`\n\n\
                 **Output:**\n```\n{result}\n```"
            )
        }

        "web_search" | "tavily_search" => {
            let query = extract_str(args, "query").unwrap_or_default();
            format!(
                "## Web Search\n\
                 **Query:** {query}\n\n\
                 **Results:**\n{result}"
            )
        }

        "web_fetch" => {
            let url = extract_str(args, "url").unwrap_or_default();
            format!(
                "## Web Fetch\n\
                 **URL:** {url}\n\n\
                 **Content:**\n{result}"
            )
        }

        "write_file" | "create_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!(
                "## File Created/Written\n\
                 **Path:** `{path}`\n\n\
                 **Summary:** {result}"
            )
        }

        "edit_file" => {
            let path = extract_str(args, "path").unwrap_or_default();
            format!(
                "## File Edited\n\
                 **Path:** `{path}`\n\n\
                 **Change:** {result}"
            )
        }

        _ => {
            let args_preview = truncate_json(args, 300);
            format!(
                "## {tool_name}\n\
                 **Args:** {args_preview}\n\n\
                 **Result:**\n{result}"
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_store_run_command_finished() {
        assert_eq!(
            should_store("run_command", ToolcallStatus::Finished),
            StoreDecision::Store(MemoryType::Technique)
        );
    }

    #[test]
    fn test_should_store_run_command_failed() {
        assert_eq!(
            should_store("run_command", ToolcallStatus::Failed),
            StoreDecision::Skip
        );
    }

    #[test]
    fn test_should_store_read_file() {
        assert_eq!(
            should_store("read_file", ToolcallStatus::Finished),
            StoreDecision::Skip
        );
    }

    #[test]
    fn test_should_store_web_search() {
        assert_eq!(
            should_store("web_search", ToolcallStatus::Finished),
            StoreDecision::Store(MemoryType::Observation)
        );
    }

    #[test]
    fn test_should_store_write_file() {
        assert_eq!(
            should_store("write_file", ToolcallStatus::Finished),
            StoreDecision::StoreSummary(MemoryType::Technique)
        );
    }

    #[test]
    fn test_filter_content_empty() {
        assert!(filter_content("").is_none());
        assert!(filter_content("   ").is_none());
    }

    #[test]
    fn test_filter_content_too_short() {
        assert!(filter_content("ok").is_none());
        assert!(filter_content("done").is_none());
    }

    #[test]
    fn test_filter_content_normal() {
        let content = "a".repeat(100);
        let result = filter_content(&content);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 100);
    }

    #[test]
    fn test_filter_content_truncates_long() {
        let content = "x".repeat(20000);
        let result = filter_content(&content).unwrap();
        assert!(result.contains("bytes omitted"));
        assert!(result.len() < content.len());
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn test_build_memory_content_run_command() {
        let args = serde_json::json!({"command": "nmap -sV 10.0.0.1"});
        let result = build_memory_content("run_command", &args, "22/tcp open ssh");
        assert!(result.contains("nmap -sV 10.0.0.1"));
        assert!(result.contains("22/tcp open ssh"));
    }

    #[test]
    fn test_build_memory_content_web_search() {
        let args = serde_json::json!({"query": "CVE-2024-1234"});
        let result = build_memory_content("web_search", &args, "Found 3 results");
        assert!(result.contains("CVE-2024-1234"));
    }

    #[test]
    fn test_pentest_tool_stored_as_vulnerability() {
        assert_eq!(
            should_store("pentest_exploit", ToolcallStatus::Finished),
            StoreDecision::Store(MemoryType::Vulnerability)
        );
    }
}
