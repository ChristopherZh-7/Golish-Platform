//! Dispatch a batch of tool calls produced by one assistant turn and append the
//! collected tool-result user message to the chat history.
//!
//! Sub-agent calls run concurrently when there are >= 2 of them (no spawn
//! overhead for single calls); other tool calls always run sequentially.
//! System hooks emitted by individual tool executions are merged into the same
//! tool-results user message to avoid "user after tool" ordering violations
//! with OpenAI-compatible APIs.

use rig::completion::Message;
use rig::message::{Text, ToolCall, ToolResult, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::events::AiEvent;
use golish_sub_agents::SubAgentContext;

use super::context::{is_cancelled, AgenticLoopContext, LoopCaptureContext};
use super::single_tool_call::execute_single_tool_call;
use super::sub_agent_dispatch::partition_tool_calls;
use golish_agent_kit::system_hooks::{format_system_hooks, HookRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolDispatchHaltReason {
    CompanyControllerBlocked,
    CompanyControllerFinalizationFailed,
    CompanyControllerFinalSubmissionMissing,
    CompanyControllerRuntimeRecovered,
    OperatorRecoveryRequired,
    StageRunReentryBlocked,
}

impl ToolDispatchHaltReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CompanyControllerBlocked => "company_controller_blocked",
            Self::CompanyControllerFinalizationFailed => "company_controller_finalization_failed",
            Self::CompanyControllerFinalSubmissionMissing => {
                "company_controller_final_submission_missing"
            }
            Self::CompanyControllerRuntimeRecovered => "company_controller_runtime_recovered",
            Self::OperatorRecoveryRequired => "operator_recovery_required",
            Self::StageRunReentryBlocked => "stage_run_reentry_blocked",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolDispatchOutcome {
    pub stage_submission_accepted: bool,
    pub halt_current_request: Option<ToolDispatchHaltReason>,
}

fn first_tool_result_json(content: &UserContent) -> Option<serde_json::Value> {
    let UserContent::ToolResult(result) = content else {
        return None;
    };
    result.content.iter().find_map(|content| {
        let ToolResultContent::Text(text) = content else {
            return None;
        };
        serde_json::Deserializer::from_str(&text.text)
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(Result::ok)
    })
}

fn tool_result_has_json_status(content: &UserContent, expected: &str) -> bool {
    first_tool_result_json(content)
        .and_then(|value| {
            value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        == Some(expected)
}

fn stage_run_halt_reason(tool_name: &str, content: &UserContent) -> Option<ToolDispatchHaltReason> {
    if tool_name != "stage_run" {
        return None;
    }
    let value = first_tool_result_json(content)?;
    let control = value.get("runtime_control")?;
    if control.get("kind").and_then(serde_json::Value::as_str) != Some("halt_current_request") {
        return None;
    }
    match control.get("reason").and_then(serde_json::Value::as_str) {
        Some("operator_recovery_required")
            if value
                .get("operator_recovery_required")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("scheduler").and_then(serde_json::Value::as_str)
                    == Some("company_controller_v1") =>
        {
            Some(ToolDispatchHaltReason::OperatorRecoveryRequired)
        }
        Some("company_controller_runtime_recovered")
            if value
                .get("operator_recovery_required")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("scheduler").and_then(serde_json::Value::as_str)
                    == Some("company_controller_v1")
                && value
                    .get("gaps")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|gaps| {
                        gaps.iter().any(|gap| {
                            gap.get("code").and_then(serde_json::Value::as_str)
                                == Some("COMPANY_CONTROLLER_RUNTIME_RECOVERED")
                        })
                    }) =>
        {
            Some(ToolDispatchHaltReason::CompanyControllerRuntimeRecovered)
        }
        Some("company_controller_final_submission_missing")
            if value
                .get("operator_recovery_required")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("scheduler").and_then(serde_json::Value::as_str)
                    == Some("company_controller_v1") =>
        {
            Some(ToolDispatchHaltReason::CompanyControllerFinalSubmissionMissing)
        }
        Some("company_controller_finalization_failed")
            if value
                .get("operator_recovery_required")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false)
                && value.get("scheduler").and_then(serde_json::Value::as_str)
                    == Some("company_controller_v1") =>
        {
            Some(ToolDispatchHaltReason::CompanyControllerFinalizationFailed)
        }
        Some("company_controller_blocked")
            if value
                .get("operator_recovery_required")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false)
                && value
                    .get("gaps")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|gaps| !gaps.is_empty())
                && value.get("scheduler").and_then(serde_json::Value::as_str)
                    == Some("company_controller_v1") =>
        {
            Some(ToolDispatchHaltReason::CompanyControllerBlocked)
        }
        Some("stage_run_reentry_blocked")
            if value
                .get("reentry_blocked")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && value
                    .get("retry_budget_exhausted")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                && value.get("passed").and_then(serde_json::Value::as_bool) == Some(false) =>
        {
            Some(ToolDispatchHaltReason::StageRunReentryBlocked)
        }
        _ => None,
    }
}

fn paired_synthetic_tool_result(tool_call: &ToolCall, value: serde_json::Value) -> UserContent {
    UserContent::ToolResult(ToolResult {
        id: tool_call.id.clone(),
        call_id: Some(
            tool_call
                .call_id
                .clone()
                .unwrap_or_else(|| tool_call.id.clone()),
        ),
        content: OneOrMany::one(ToolResultContent::Text(Text {
            text: value.to_string(),
        })),
    })
}

fn stage_submission_barrier_result(tool_call: &ToolCall) -> (UserContent, Vec<String>) {
    (
        paired_synthetic_tool_result(
            tool_call,
            serde_json::json!({
                "status": "skipped",
                "blocked_by_stage_submission": true,
                "error": "Skipped without execution because an earlier submit_stage_deliverable in this tool batch was accepted."
            }),
        ),
        Vec::new(),
    )
}

fn stage_run_halt_barrier_result(
    tool_call: &ToolCall,
    reason: ToolDispatchHaltReason,
) -> (UserContent, Vec<String>) {
    (
        paired_synthetic_tool_result(
            tool_call,
            serde_json::json!({
                "status": "skipped",
                "blocked_by_stage_run_halt": true,
                "halt_reason": reason.as_str(),
                "error": "Skipped without execution because an earlier stage_run result ended the current top-level request."
            }),
        ),
        Vec::new(),
    )
}

fn cancelled_batch_tool_result(tool_call: &ToolCall) -> (UserContent, Vec<String>) {
    (
        paired_synthetic_tool_result(
            tool_call,
            serde_json::json!({
                "status": "cancelled",
                "cancelled": true,
                "error": "Skipped without execution because the agent was cancelled."
            }),
        ),
        Vec::new(),
    )
}

/// Execute a batch containing a harness terminal candidate in original
/// assistant order. Once a stage submission is accepted or `stage_run` returns
/// closed server-authored request control, every later call receives a paired
/// synthetic ToolResult and is never dispatched. A terminal barrier and
/// speculative parallel work cannot safely coexist in the same assistant batch.
async fn dispatch_harness_terminal_batch<F, Fut>(
    calls: Vec<(usize, ToolCall)>,
    mut execute: F,
) -> (
    Vec<(usize, (UserContent, Vec<String>))>,
    ToolDispatchOutcome,
)
where
    F: FnMut(ToolCall) -> Fut,
    Fut: std::future::Future<Output = (UserContent, Vec<String>)>,
{
    let mut results = Vec::with_capacity(calls.len());
    let mut outcome = ToolDispatchOutcome::default();
    for (index, tool_call) in calls {
        if outcome.stage_submission_accepted {
            results.push((index, stage_submission_barrier_result(&tool_call)));
            continue;
        }
        if let Some(reason) = outcome.halt_current_request {
            results.push((index, stage_run_halt_barrier_result(&tool_call, reason)));
            continue;
        }
        let tool_name = tool_call.function.name.clone();
        let result = execute(tool_call).await;
        if tool_name == "submit_stage_deliverable"
            && tool_result_has_json_status(&result.0, "accepted")
        {
            outcome.stage_submission_accepted = true;
        } else if let Some(reason) = stage_run_halt_reason(&tool_name, &result.0) {
            outcome.halt_current_request = Some(reason);
        }
        results.push((index, result));
    }
    (results, outcome)
}

/// Run all `tool_calls_to_execute` and append the resulting user message
/// (tool results + any merged system hooks) to `chat_history`.
pub(crate) async fn dispatch_tool_calls<M>(
    tool_calls_to_execute: Vec<ToolCall>,
    ctx: &AgenticLoopContext<'_>,
    capture_ctx: &LoopCaptureContext,
    model: &M,
    sub_agent_context: &SubAgentContext,
    hook_registry: &HookRegistry,
    llm_span: &tracing::Span,
    chat_history: &mut Vec<Message>,
) -> ToolDispatchOutcome
where
    M: rig::completion::CompletionModel + Sync,
{
    let total_tool_count = tool_calls_to_execute.len();
    let has_stage_submission = tool_calls_to_execute
        .iter()
        .any(|call| call.function.name == "submit_stage_deliverable");
    let has_stage_run = tool_calls_to_execute
        .iter()
        .any(|call| call.function.name == "stage_run");

    let mut indexed_results: Vec<Option<(UserContent, Vec<String>)>> = vec![None; total_tool_count];
    let outcome = if has_stage_submission || has_stage_run {
        let indexed_calls = tool_calls_to_execute.into_iter().enumerate().collect();
        let (results, outcome) =
            dispatch_harness_terminal_batch(indexed_calls, |tool_call| async move {
                if is_cancelled(ctx) {
                    tracing::info!(
                        "Agent cancelled before tool execution: {}",
                        tool_call.function.name
                    );
                    cancelled_batch_tool_result(&tool_call)
                } else {
                    execute_single_tool_call(
                        tool_call,
                        ctx,
                        capture_ctx,
                        model,
                        sub_agent_context,
                        hook_registry,
                        llm_span,
                    )
                    .await
                }
            })
            .await;
        for (index, result) in results {
            indexed_results[index] = Some(result);
        }
        outcome
    } else {
        let (sub_agent_calls, other_calls) = partition_tool_calls(tool_calls_to_execute);
        let has_concurrent_sub_agents = sub_agent_calls.len() >= 2;

        if has_concurrent_sub_agents {
            tracing::info!(
                count = sub_agent_calls.len(),
                "Executing sub-agent tool calls concurrently"
            );

            let futures: Vec<_> = sub_agent_calls
                .into_iter()
                .map(|(original_idx, tool_call)| async move {
                    let result = execute_single_tool_call(
                        tool_call,
                        ctx,
                        capture_ctx,
                        model,
                        sub_agent_context,
                        hook_registry,
                        llm_span,
                    )
                    .await;
                    (original_idx, result)
                })
                .collect();

            let concurrent_results = futures::future::join_all(futures).await;
            for (idx, result) in concurrent_results {
                indexed_results[idx] = Some(result);
            }
        } else {
            // 0 or 1 sub-agent calls — execute sequentially (no spawn overhead)
            for (original_idx, tool_call) in sub_agent_calls {
                if is_cancelled(ctx) {
                    tracing::info!(
                        "Agent cancelled before sub-agent call: {}",
                        tool_call.function.name
                    );
                    break;
                }
                let result = execute_single_tool_call(
                    tool_call,
                    ctx,
                    capture_ctx,
                    model,
                    sub_agent_context,
                    hook_registry,
                    llm_span,
                )
                .await;
                indexed_results[original_idx] = Some(result);
            }
        }

        for (original_idx, tool_call) in other_calls {
            if is_cancelled(ctx) {
                tracing::info!(
                    "Agent cancelled before tool execution: {}",
                    tool_call.function.name
                );
                break;
            }
            let result = execute_single_tool_call(
                tool_call,
                ctx,
                capture_ctx,
                model,
                sub_agent_context,
                hook_registry,
                llm_span,
            )
            .await;
            indexed_results[original_idx] = Some(result);
        }
        ToolDispatchOutcome::default()
    };
    let mut tool_results: Vec<UserContent> = Vec::with_capacity(total_tool_count);
    let mut system_hooks: Vec<String> = vec![];
    for (user_content, hooks) in indexed_results.into_iter().flatten() {
        tool_results.push(user_content);
        system_hooks.extend(hooks);
    }

    if !system_hooks.is_empty() {
        let formatted_hooks = format_system_hooks(&system_hooks);

        tracing::info!(
            count = system_hooks.len(),
            content_len = formatted_hooks.len(),
            "Injecting system hooks into tool results message"
        );

        let _ = ctx.events.event_tx.send(AiEvent::SystemHooksInjected {
            hooks: system_hooks.clone(),
        });

        let _system_hook_event = tracing::info_span!(
            parent: llm_span,
            "system_hooks_injected",
            "langfuse.observation.type" = "event",
            "langfuse.observation.level" = "DEFAULT",
            "langfuse.session.id" = ctx.events.session_id.unwrap_or(""),
            hook_count = system_hooks.len(),
            "langfuse.observation.input" = %formatted_hooks,
        );

        tool_results.push(UserContent::Text(Text {
            text: formatted_hooks,
        }));
    }

    chat_history.push(Message::User {
        content: OneOrMany::many(tool_results).unwrap_or_else(|_| {
            OneOrMany::one(UserContent::Text(Text {
                text: "Tool executed".to_string(),
            }))
        }),
    });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{Text, ToolFunction, ToolResult};

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_id: Some(format!("provider-{id}")),
            function: ToolFunction {
                name: name.to_string(),
                arguments: serde_json::json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    fn fake_result(call: &ToolCall, status: &str) -> (UserContent, Vec<String>) {
        (
            UserContent::ToolResult(ToolResult {
                id: call.id.clone(),
                call_id: call.call_id.clone(),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: serde_json::json!({"status": status}).to_string(),
                })),
            }),
            Vec::new(),
        )
    }

    #[test]
    fn accepted_submit_status_is_read_from_json_before_runtime_notes() {
        let content = UserContent::ToolResult(ToolResult {
            id: "call-1".to_string(),
            call_id: Some("provider-call-1".to_string()),
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: "{\"status\":\"accepted\",\"note\":\"ok\"}\n\n--- RUNTIME NOTE ---"
                    .to_string(),
            })),
        });
        assert!(tool_result_has_json_status(&content, "accepted"));
        assert!(!tool_result_has_json_status(&content, "needs_fix"));
    }

    #[tokio::test]
    async fn accepted_submit_short_circuits_later_batch_calls_with_paired_results() {
        let calls = vec![
            (0, tool_call("before", "update_plan")),
            (1, tool_call("submit", "submit_stage_deliverable")),
            (2, tool_call("after", "manage_targets")),
        ];
        let executed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = std::sync::Arc::clone(&executed);

        let (results, outcome) = dispatch_harness_terminal_batch(calls, move |call| {
            observed.lock().unwrap().push(call.id.clone());
            let status = if call.function.name == "submit_stage_deliverable" {
                "accepted"
            } else {
                "ok"
            };
            async move { fake_result(&call, status) }
        })
        .await;

        assert!(outcome.stage_submission_accepted);
        assert_eq!(outcome.halt_current_request, None);
        assert_eq!(
            executed.lock().unwrap().as_slice(),
            &["before".to_string(), "submit".to_string()]
        );
        assert_eq!(
            results.len(),
            3,
            "every assistant call needs one ToolResult"
        );
        let (index, (skipped, hooks)) = &results[2];
        assert_eq!(*index, 2);
        assert!(hooks.is_empty());
        let UserContent::ToolResult(skipped) = skipped else {
            panic!("skipped call must be paired with a ToolResult")
        };
        assert_eq!(skipped.id, "after");
        assert_eq!(skipped.call_id.as_deref(), Some("provider-after"));
        let ToolResultContent::Text(text) = skipped.content.first() else {
            panic!("skipped ToolResult must carry JSON text")
        };
        let value: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        assert_eq!(value["blocked_by_stage_submission"], true);
        assert_eq!(value["status"], "skipped");
    }

    #[tokio::test]
    async fn stage_run_operator_recovery_short_circuits_later_batch_calls() {
        let calls = vec![
            (0, tool_call("stage-run", "stage_run")),
            (1, tool_call("coverage", "check_stage_asset_coverage")),
            (2, tool_call("submit", "submit_stage_deliverable")),
        ];
        let executed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = std::sync::Arc::clone(&executed);

        let (results, outcome) = dispatch_harness_terminal_batch(calls, move |call| {
            observed.lock().unwrap().push(call.id.clone());
            async move {
                let value = if call.function.name == "stage_run" {
                    serde_json::json!({
                        "operator_recovery_required": true,
                        "passed": false,
                        "retry_budget_exhausted": true,
                        "runtime_control": {
                            "kind": "halt_current_request",
                            "reason": "operator_recovery_required",
                        },
                        "scheduler": "company_controller_v1",
                    })
                } else {
                    serde_json::json!({"status": "ok"})
                };
                (
                    UserContent::ToolResult(ToolResult {
                        id: call.id,
                        call_id: call.call_id,
                        content: OneOrMany::one(ToolResultContent::Text(Text {
                            text: value.to_string(),
                        })),
                    }),
                    Vec::new(),
                )
            }
        })
        .await;

        assert_eq!(
            outcome.halt_current_request,
            Some(ToolDispatchHaltReason::OperatorRecoveryRequired)
        );
        assert!(!outcome.stage_submission_accepted);
        assert_eq!(executed.lock().unwrap().as_slice(), &["stage-run"]);
        assert_eq!(results.len(), 3, "every tool call remains provider-paired");
        for (_, (content, _)) in results.iter().skip(1) {
            let value = first_tool_result_json(content).expect("synthetic JSON result");
            assert_eq!(value["blocked_by_stage_run_halt"], true);
        }
    }

    #[tokio::test]
    async fn stage_run_company_controller_block_short_circuits_later_batch_calls() {
        let calls = vec![
            (0, tool_call("stage-run", "stage_run")),
            (1, tool_call("coverage", "check_stage_asset_coverage")),
            (2, tool_call("submit", "submit_stage_deliverable")),
        ];
        let executed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = std::sync::Arc::clone(&executed);

        let (results, outcome) = dispatch_harness_terminal_batch(calls, move |call| {
            observed.lock().unwrap().push(call.id.clone());
            async move {
                let value = if call.function.name == "stage_run" {
                    serde_json::json!({
                        "gaps": [{"code": "COMPANY_CONTROLLER_FAILED"}],
                        "operator_recovery_required": false,
                        "passed": false,
                        "retry_budget_exhausted": true,
                        "runtime_control": {
                            "kind": "halt_current_request",
                            "reason": "company_controller_blocked",
                        },
                        "scheduler": "company_controller_v1",
                    })
                } else {
                    serde_json::json!({"status": "ok"})
                };
                (
                    UserContent::ToolResult(ToolResult {
                        id: call.id,
                        call_id: call.call_id,
                        content: OneOrMany::one(ToolResultContent::Text(Text {
                            text: value.to_string(),
                        })),
                    }),
                    Vec::new(),
                )
            }
        })
        .await;

        assert_eq!(
            outcome.halt_current_request,
            Some(ToolDispatchHaltReason::CompanyControllerBlocked)
        );
        assert!(!outcome.stage_submission_accepted);
        assert_eq!(executed.lock().unwrap().as_slice(), &["stage-run"]);
        assert_eq!(results.len(), 3, "every tool call remains provider-paired");
        for (_, (content, _)) in results.iter().skip(1) {
            let value = first_tool_result_json(content).expect("synthetic JSON result");
            assert_eq!(value["blocked_by_stage_run_halt"], true);
            assert_eq!(value["halt_reason"], "company_controller_blocked");
        }
    }

    #[test]
    fn runtime_control_is_closed_to_stage_run_and_known_reasons() {
        let call = tool_call("stage-run", "stage_run");
        let result = fake_result(&call, "ok");
        assert_eq!(stage_run_halt_reason("stage_run", &result.0), None);

        let controlled = UserContent::ToolResult(ToolResult {
            id: "stage-run".to_string(),
            call_id: Some("provider-stage-run".to_string()),
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: serde_json::json!({
                    "operator_recovery_required": true,
                    "passed": false,
                    "retry_budget_exhausted": true,
                    "runtime_control": {
                        "kind": "halt_current_request",
                        "reason": "operator_recovery_required",
                    },
                    "scheduler": "company_controller_v1",
                })
                .to_string(),
            })),
        });
        assert_eq!(
            stage_run_halt_reason("stage_run", &controlled),
            Some(ToolDispatchHaltReason::OperatorRecoveryRequired)
        );
        assert_eq!(stage_run_halt_reason("update_plan", &controlled), None);

        let ordinary_company_block = UserContent::ToolResult(ToolResult {
            id: "stage-run-blocked".to_string(),
            call_id: Some("provider-stage-run-blocked".to_string()),
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: serde_json::json!({
                    "gaps": [{"code": "COMPANY_CONTROLLER_FAILED"}],
                    "operator_recovery_required": false,
                    "passed": false,
                    "retry_budget_exhausted": true,
                    "runtime_control": {
                        "kind": "halt_current_request",
                        "reason": "company_controller_blocked",
                    },
                    "scheduler": "company_controller_v1",
                })
                .to_string(),
            })),
        });
        assert_eq!(
            stage_run_halt_reason("stage_run", &ordinary_company_block),
            Some(ToolDispatchHaltReason::CompanyControllerBlocked)
        );

        for (reason, gap_code, expected) in [
            (
                "company_controller_finalization_failed",
                "COMPANY_CONTROLLER_FINAL_SEAL_FAILED",
                ToolDispatchHaltReason::CompanyControllerFinalizationFailed,
            ),
            (
                "company_controller_final_submission_missing",
                "COMPANY_CONTROLLER_FINAL_SUBMISSION_MISSING",
                ToolDispatchHaltReason::CompanyControllerFinalSubmissionMissing,
            ),
            (
                "company_controller_runtime_recovered",
                "COMPANY_CONTROLLER_RUNTIME_RECOVERED",
                ToolDispatchHaltReason::CompanyControllerRuntimeRecovered,
            ),
        ] {
            let closeout_halt = UserContent::ToolResult(ToolResult {
                id: format!("stage-run-{reason}"),
                call_id: Some(format!("provider-stage-run-{reason}")),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: serde_json::json!({
                        "gaps": [{"code": gap_code}],
                        "operator_recovery_required": false,
                        "passed": false,
                        "retry_budget_exhausted": true,
                        "runtime_control": {
                            "kind": "halt_current_request",
                            "reason": reason,
                        },
                        "scheduler": "company_controller_v1",
                    })
                    .to_string(),
                })),
            });
            assert_eq!(
                stage_run_halt_reason("stage_run", &closeout_halt),
                Some(expected)
            );
        }

        let lookalike = UserContent::ToolResult(ToolResult {
            id: "stage-run-lookalike".to_string(),
            call_id: Some("provider-stage-run-lookalike".to_string()),
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: serde_json::json!({
                    "gaps": [{"code": "COMPANY_CONTROLLER_FAILED"}],
                    "operator_recovery_required": false,
                    "passed": false,
                    "retry_budget_exhausted": false,
                    "runtime_control": {
                        "kind": "halt_current_request",
                        "reason": "company_controller_blocked",
                    },
                    "scheduler": "company_controller_v1",
                })
                .to_string(),
            })),
        });
        assert_eq!(stage_run_halt_reason("stage_run", &lookalike), None);
    }
}
