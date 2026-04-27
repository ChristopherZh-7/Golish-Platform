//! `execute_with_hitl_generic` — wraps `execute_tool_direct_generic` with
//! HITL (human-in-the-loop) approval prompting.
//!
//! When the tool policy says `Allow`, runs immediately. When it says
//! `Prompt`, emits a `ToolApprovalRequest` event and blocks on a
//! `oneshot::Receiver<ApprovalDecision>` registered in
//! `ctx.access.pending_approvals` (or via the coordinator). Honours
//! `APPROVAL_TIMEOUT_SECS` to avoid hanging forever, and applies
//! `PolicyConstraintResult` arg modifications inline.

use anyhow::Result;
use rig::completion::CompletionModel as RigCompletionModel;
use serde_json::json;

use golish_core::events::AiEvent;
use golish_core::hitl::RiskLevel;
use golish_sub_agents::SubAgentContext;

use super::direct::execute_tool_direct_generic;
use super::super::{
    emit_event, emit_to_frontend, AgenticLoopContext, LoopCaptureContext, ToolExecutionResult,
    APPROVAL_TIMEOUT_SECS,
};
use crate::tool_policy::{PolicyConstraintResult, ToolPolicy};


/// Execute a tool with HITL approval check for generic models.
pub async fn execute_with_hitl_generic<M>(
    tool_name: &str,
    tool_args: &serde_json::Value,
    tool_id: &str,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    context: &SubAgentContext,
) -> Result<ToolExecutionResult>
where
    M: RigCompletionModel + Sync,
{
    capture_ctx.process(&AiEvent::ToolRequest {
        request_id: tool_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_args.clone(),
        source: golish_core::events::ToolSource::Main,
    });

    let agent_mode = *ctx.access.agent_mode.read().await;

    let is_auto_approve =
        agent_mode.is_auto_approve() || ctx.events.runtime.is_some_and(|r| r.auto_approve());

    // Planning mode: only allow read-only tools
    if agent_mode.is_planning() {
        use crate::tool_policy::ALLOW_TOOLS;
        if !ALLOW_TOOLS.contains(&tool_name) {
            let denied_event = AiEvent::ToolDenied {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_args.clone(),
                reason: "Planning mode: only read-only tools are allowed".to_string(),
                source: golish_core::events::ToolSource::Main,
            };
            emit_to_frontend(ctx, denied_event.clone());
            capture_ctx.process(&denied_event);
            return Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Tool '{}' is not allowed in planning mode (read-only)", tool_name),
                    "planning_mode_denied": true
                }),
                success: false,
            });
        }
    }

    if !is_auto_approve && ctx.access.tool_policy_manager.is_denied(tool_name).await {
        let denied_event = AiEvent::ToolDenied {
            request_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            args: tool_args.clone(),
            reason: "Tool is denied by policy".to_string(),
            source: golish_core::events::ToolSource::Main,
        };
        emit_to_frontend(ctx, denied_event.clone());
        capture_ctx.process(&denied_event);
        return Ok(ToolExecutionResult {
            value: json!({
                "error": format!("Tool '{}' is denied by policy", tool_name),
                "denied_by_policy": true
            }),
            success: false,
        });
    }

    let (effective_args, constraint_note) = match ctx
        .access
        .tool_policy_manager
        .apply_constraints(tool_name, tool_args)
        .await
    {
        PolicyConstraintResult::Allowed => (tool_args.clone(), None),
        PolicyConstraintResult::Violated(reason) => {
            emit_event(
                ctx,
                AiEvent::ToolDenied {
                    request_id: tool_id.to_string(),
                    tool_name: tool_name.to_string(),
                    args: tool_args.clone(),
                    reason: reason.clone(),
                    source: golish_core::events::ToolSource::Main,
                },
            );
            return Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Tool constraint violated: {}", reason),
                    "constraint_violated": true
                }),
                success: false,
            });
        }
        PolicyConstraintResult::Modified(modified_args, note) => {
            tracing::info!("Tool '{}' args modified by constraint: {}", tool_name, note);
            (modified_args, Some(note))
        }
    };

    let policy = ctx.access.tool_policy_manager.get_policy(tool_name).await;
    if policy == ToolPolicy::Allow {
        let reason = if let Some(note) = constraint_note {
            format!("Allowed by policy ({})", note)
        } else {
            "Allowed by tool policy".to_string()
        };
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason,
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if ctx.access.approval_recorder.should_auto_approve(tool_name).await {
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: "Auto-approved based on learned patterns or always-allow list".to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if tool_name.starts_with("pentest_") {
        tracing::info!("[hitl] Auto-approving pentest tool: {}", tool_name);
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: "Auto-approved: Golish platform tool".to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    if is_auto_approve {
        let reason = if agent_mode.is_auto_approve() {
            "Auto-approved via agent mode"
        } else {
            "Auto-approved via --auto-approve flag"
        };
        emit_event(
            ctx,
            AiEvent::ToolAutoApproved {
                request_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args: effective_args.clone(),
                reason: reason.to_string(),
                source: golish_core::events::ToolSource::Main,
            },
        );

        return execute_tool_direct_generic(tool_name, &effective_args, ctx, model, context, tool_id)
            .await;
    }

    // Need HITL approval
    let stats = ctx.access.approval_recorder.get_pattern(tool_name).await;
    let risk_level = RiskLevel::for_tool(tool_name);
    let config = ctx.access.approval_recorder.get_config().await;
    let can_learn = !config
        .always_require_approval
        .contains(&tool_name.to_string());
    let suggestion = ctx.access.approval_recorder.get_suggestion(tool_name).await;

    let rx = if let Some(coordinator) = ctx.access.coordinator {
        coordinator.register_approval(tool_id.to_string())
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = ctx.access.pending_approvals.write().await;
            pending.insert(tool_id.to_string(), tx);
        }
        rx
    };

    emit_to_frontend(
        ctx,
        AiEvent::ToolApprovalRequest {
            request_id: tool_id.to_string(),
            tool_name: tool_name.to_string(),
            args: effective_args.clone(),
            stats,
            risk_level,
            can_learn,
            suggestion,
            source: golish_core::events::ToolSource::Main,
        },
    );

    tracing::info!(
        "[hitl] Waiting for user approval: tool={}, risk={:?}, id={}",
        tool_name,
        risk_level,
        tool_id
    );

    match tokio::time::timeout(std::time::Duration::from_secs(APPROVAL_TIMEOUT_SECS), rx).await {
        Ok(Ok(decision)) => {
            tracing::info!(
                "[hitl] User decision: tool={}, approved={}",
                tool_name,
                decision.approved
            );
            if decision.approved {
                let _ = ctx
                    .access
                    .approval_recorder
                    .record_approval(tool_name, true, decision.reason, decision.always_allow)
                    .await;

                execute_tool_direct_generic(
                    tool_name,
                    &effective_args,
                    ctx,
                    model,
                    context,
                    tool_id,
                )
                .await
            } else {
                let _ = ctx
                    .access
                    .approval_recorder
                    .record_approval(tool_name, false, decision.reason, false)
                    .await;

                Ok(ToolExecutionResult {
                    value: json!({"error": "Tool execution denied by user. Do NOT retry this tool with the same or similar arguments.", "denied": true}),
                    success: false,
                })
            }
        }
        Ok(Err(_)) => Ok(ToolExecutionResult {
            value: json!({"error": "Approval request cancelled", "cancelled": true}),
            success: false,
        }),
        Err(_) => {
            tracing::warn!(
                "[hitl] Approval TIMED OUT after {}s: tool={}",
                APPROVAL_TIMEOUT_SECS,
                tool_name
            );
            if ctx.access.coordinator.is_none() {
                let mut pending = ctx.access.pending_approvals.write().await;
                pending.remove(tool_id);
            }

            Ok(ToolExecutionResult {
                value: json!({
                    "error": format!("Approval request timed out after {} seconds. The user did not respond to the approval prompt. Do NOT retry this tool or attempt alternative approaches — inform the user that the tool requires their approval and wait for their next message.", APPROVAL_TIMEOUT_SECS),
                    "timeout": true,
                    "requires_user_action": true
                }),
                success: false,
            })
        }
    }
}
