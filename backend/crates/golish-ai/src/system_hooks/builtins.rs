//! Built-in system hooks.
//!
//! This module defines all the default hooks that ship with the system.

use golish_core::ToolName;

use super::hooks::{MessageHook, PreToolResult, ToolHook};
use super::matcher::ToolMatcher;

/// Get all built-in message hooks.
pub fn message_hooks() -> Vec<MessageHook> {
    vec![]
}

/// Get all built-in tool hooks.
pub fn tool_hooks() -> Vec<ToolHook> {
    vec![
        plan_completion_hook(),
        security_tool_redirect_hook(),
        sub_agent_auto_store_hook(),
    ]
}

/// Hook that fires when all plan tasks are completed.
///
/// Reminds the agent to update documentation after completing a multi-step task.
fn plan_completion_hook() -> ToolHook {
    ToolHook::post(
        "plan_completion",
        ToolMatcher::tool(ToolName::UpdatePlan),
        |ctx| {
            if !is_plan_complete(ctx.result) {
                return None;
            }

            Some(
                "[Plan Complete - Documentation Check]

SKIP documentation updates for:
- Bug fixes
- Refactors
- Minor tweaks
- Test changes
- Internal implementation details
- Any work that doesn't change external behavior or developer workflow

UPDATE documentation ONLY when changes ADD or MODIFY:
- Public APIs or SDK interfaces
- CLI commands or flags
- Configuration options or environment variables
- Installation or setup steps
- Breaking changes to existing functionality

Before updating, confirm: Does this change affect how someone USES or DEVELOPS against this code?

Documentation targets:
- **Developer docs** (README.md, docs/*.md): Update commands, setup instructions, API references
- **Agent docs** (CLAUDE.md, AGENTS.md, ...): Update code patterns, conventions, build/test commands

STOP CONDITIONS:
- Do not call update_plan or create new plan tasks as a result of this message
- In the context of this reminder, do not create documentation update todos or subtasks
- If no docs need updating, disregard this message entirely"
                    .to_string(),
            )
        },
    )
}

/// Pre-tool hook that intercepts direct security tool usage by the main agent.
///
/// When the main agent tries to run nmap, sqlmap, gobuster, nikto, etc.
/// directly via run_pty_cmd, this hook injects a reminder to delegate to
/// the pentester sub-agent instead.
fn security_tool_redirect_hook() -> ToolHook {
    ToolHook::pre(
        "security_tool_redirect",
        ToolMatcher::tool(ToolName::RunPtyCmd),
        |ctx| {
            let command = ctx
                .args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let security_tools = [
                "nmap", "masscan", "gobuster", "ffuf", "nikto", "sqlmap",
                "hydra", "wfuzz", "dirb", "dirsearch", "nuclei",
                "burpsuite", "zap", "metasploit", "msfconsole",
                "hashcat", "john", "aircrack", "responder",
            ];

            let is_security_tool = security_tools
                .iter()
                .any(|tool| command.split_whitespace().next() == Some(tool));

            if is_security_tool {
                PreToolResult::AllowWithMessage(
                    "[Agent Orchestration] You are running a security tool directly. \
                     For better results, consider delegating to the `pentester` sub-agent \
                     which has specialized knowledge for interpreting results and planning \
                     next steps. Use: sub_agent_pentester with the task description. \
                     If you've already delegated and this is the pentester executing, proceed."
                        .to_string(),
                )
            } else {
                PreToolResult::Allow
            }
        },
    )
}

/// Post-tool hook that reminds the agent to store significant findings
/// after a sub-agent completes its work.
fn sub_agent_auto_store_hook() -> ToolHook {
    ToolHook::post(
        "sub_agent_auto_store",
        ToolMatcher::custom_post(|ctx| {
            ctx.tool_name_raw.starts_with("sub_agent_pentester")
                || ctx.tool_name_raw.starts_with("sub_agent_researcher")
                || ctx.tool_name_raw.starts_with("sub_agent_js_harvester")
                || ctx.tool_name_raw.starts_with("sub_agent_js_analyzer")
        }),
        |ctx| {
            if !ctx.success {
                return None;
            }

            let response_len = ctx
                .result
                .get("response")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0);

            if response_len < 50 {
                return None;
            }

            Some(format!(
                "[Memory Checkpoint] The {} sub-agent has completed with results. \
                 You SHOULD now call `store_memory` to persist significant findings \
                 (discovered hosts, vulnerabilities, credentials, etc.) for future sessions. \
                 Use category tags: recon, vulnerability, credential, configuration, technique.",
                ctx.tool_name_raw.strip_prefix("sub_agent_").unwrap_or(ctx.tool_name_raw)
            ))
        },
    )
}

/// Check if the update_plan result indicates all tasks are completed.
fn is_plan_complete(value: &serde_json::Value) -> bool {
    value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        && value
            .get("summary")
            .map(|s| {
                let total = s.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let completed = s.get("completed").and_then(|v| v.as_u64()).unwrap_or(0);
                total > 0 && total == completed
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_hooks::context::PostToolContext;
    use serde_json::json;

    #[test]
    fn test_is_plan_complete_all_done() {
        let value = json!({
            "success": true,
            "summary": { "total": 3, "completed": 3, "in_progress": 0, "pending": 0 }
        });
        assert!(is_plan_complete(&value));
    }

    #[test]
    fn test_is_plan_complete_some_pending() {
        let value = json!({
            "success": true,
            "summary": { "total": 3, "completed": 2, "in_progress": 0, "pending": 1 }
        });
        assert!(!is_plan_complete(&value));
    }

    #[test]
    fn test_is_plan_complete_in_progress() {
        let value = json!({
            "success": true,
            "summary": { "total": 3, "completed": 2, "in_progress": 1, "pending": 0 }
        });
        assert!(!is_plan_complete(&value));
    }

    #[test]
    fn test_is_plan_complete_empty_plan() {
        let value = json!({
            "success": true,
            "summary": { "total": 0, "completed": 0, "in_progress": 0, "pending": 0 }
        });
        assert!(!is_plan_complete(&value)); // Empty plan is not "complete"
    }

    #[test]
    fn test_is_plan_complete_failed_update() {
        let value = json!({
            "success": false,
            "error": "something went wrong"
        });
        assert!(!is_plan_complete(&value));
    }

    #[test]
    fn test_is_plan_complete_malformed_response() {
        let value = json!({"foo": "bar"});
        assert!(!is_plan_complete(&value));
    }

    #[test]
    fn test_plan_completion_hook_fires() {
        let hook = plan_completion_hook();
        let args = json!({});
        let result = json!({
            "success": true,
            "summary": { "total": 2, "completed": 2, "in_progress": 0, "pending": 0 }
        });

        let ctx = PostToolContext::new("update_plan", &args, &result, true, 50, "s1");
        assert!(hook.matches_post(&ctx));

        let message = hook.execute_post(&ctx);
        assert!(message.is_some());
        assert!(message.unwrap().contains("Plan Complete"));
    }

    #[test]
    fn test_plan_completion_hook_does_not_fire_incomplete() {
        let hook = plan_completion_hook();
        let args = json!({});
        let result = json!({
            "success": true,
            "summary": { "total": 3, "completed": 2, "in_progress": 1, "pending": 0 }
        });

        let ctx = PostToolContext::new("update_plan", &args, &result, true, 50, "s1");
        assert!(hook.matches_post(&ctx)); // Matches the tool
        assert!(hook.execute_post(&ctx).is_none()); // But doesn't produce output
    }

    #[test]
    fn test_plan_completion_hook_wrong_tool() {
        let hook = plan_completion_hook();
        let args = json!({});
        let result = json!({
            "success": true,
            "summary": { "total": 2, "completed": 2, "in_progress": 0, "pending": 0 }
        });

        let ctx = PostToolContext::new("run_pty_cmd", &args, &result, true, 50, "s1");
        assert!(!hook.matches_post(&ctx));
    }

    #[test]
    fn test_builtin_hooks_loaded() {
        let message = message_hooks();
        let tool = tool_hooks();

        assert_eq!(message.len(), 0);
        assert_eq!(tool.len(), 3);
        assert_eq!(tool[0].name, "plan_completion");
        assert_eq!(tool[1].name, "security_tool_redirect");
        assert_eq!(tool[2].name, "sub_agent_auto_store");
    }
}
