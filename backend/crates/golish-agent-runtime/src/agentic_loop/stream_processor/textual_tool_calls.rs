//! Agent-runtime adapter over the shared textual tool-call parser.
//!
//! The parsing/stripping logic lives in [`golish_core::textual_tool_call`] so it
//! can be shared with the sub-agent executor. This module only wraps the parsed
//! result into the runtime's [`ToolIntent`] / `ToolCall` shapes.

use rig::message::ToolCall;

pub(super) use golish_core::strip_textual_tool_call_markup;

use crate::agentic_loop::tool_intent::ToolIntent;

pub(super) fn extract_textual_tool_calls(text: &str, iteration: usize) -> Vec<ToolCall> {
    extract_textual_tool_intents(text, iteration)
        .into_iter()
        .map(ToolIntent::into_tool_call)
        .collect()
}

pub(super) fn extract_textual_tool_intents(text: &str, iteration: usize) -> Vec<ToolIntent> {
    golish_core::select_textual_tool_calls(text)
        .into_iter()
        .enumerate()
        .map(|(idx, selected)| {
            // Index the synthetic id per call so a batched multi-call turn
            // produces distinct ids (the loop balances one tool_result per id).
            let id = format!("textual-tool-call-{iteration}-{idx}");
            ToolIntent::recovered_textual_xml(
                id,
                selected.name,
                selected.arguments,
                Some(selected.raw_span),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn extracts_ask_human_before_follow_up_add() {
        let text = r#"需要确认。
<tool_call>
<function=ask_human>
<parameter=context>Target example.com is not registered.</parameter>
<parameter=input_type>confirmation</parameter>
<parameter=options>None</parameter>
<parameter=question>example.com 未注册为目标，是否添加？</parameter>
</function>
</tool_call>
<tool_call>
<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>
</tool_call>"#;

        let intents = extract_textual_tool_intents(text, 2);
        assert_eq!(intents.len(), 1, "ask_human barrier returns only itself");
        let intent = &intents[0];
        assert_eq!(intent.name, "ask_human");
        assert_eq!(
            intent.source,
            crate::agentic_loop::tool_intent::ToolIntentSource::TextualXml
        );
        assert!(intent.confidence < 1.0);
        assert!(intent
            .raw_span
            .as_deref()
            .unwrap_or("")
            .contains("ask_human"));
        assert_eq!(
            intent.args["input_type"],
            Value::String("confirmation".to_string())
        );
        assert_eq!(intent.args["options"], Value::Array(Vec::new()));
    }

    #[test]
    fn extracts_all_batched_non_barrier_calls_with_distinct_ids() {
        // A MiMo turn that batches two non-barrier calls must yield both, each
        // with a distinct synthetic id (one tool_result is balanced per id).
        let text = r#"列候选并更新计划。
<tool_call>
<function=manage_organizations>
<parameter=action>propose_candidates</parameter>
<parameter=candidates>["ACME Corp"]</parameter>
</function>
</tool_call>
<tool_call>
<function=update_plan>
<parameter=plan>[{"status":"in_progress","step":"create"}]</parameter>
</function>
</tool_call>"#;

        let calls = extract_textual_tool_calls(text, 7);
        assert_eq!(calls.len(), 2, "both batched calls must be recovered");
        assert_eq!(calls[0].function.name, "manage_organizations");
        assert_eq!(calls[1].function.name, "update_plan");
        assert_ne!(calls[0].id, calls[1].id, "ids must be distinct per call");
        assert_eq!(calls[0].id, "textual-tool-call-7-0");
        assert_eq!(calls[1].id, "textual-tool-call-7-1");
    }

    #[test]
    fn extracts_json_parameter_values() {
        let text = r#"<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>"#;

        let calls = extract_textual_tool_calls(text, 1);
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.function.name, "manage_targets");
        assert_eq!(call.function.arguments["action"], "add");
        assert_eq!(
            call.function.arguments["targets"][0]["value"],
            "example.com"
        );
    }

    #[test]
    fn mimo_textual_tool_call_prioritizes_human_barrier_over_follow_up_add() {
        let text = r#"
example.com 不在当前目标列表中。
<tool_call>
<function=ask_human>
<parameter=question>是否添加 example.com 到目标列表?</parameter>
</function>
<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>
</tool_call>
"#;

        let intents = extract_textual_tool_intents(text, 3);
        assert_eq!(
            intents.len(),
            1,
            "ask_human barrier drops the follow-up add"
        );
        let intent = &intents[0];
        assert_eq!(intent.name, "ask_human");
        assert_eq!(intent.args["question"], "是否添加 example.com 到目标列表?");

        let decision = crate::agentic_loop::tool_gate::decide_tool_intent(intent, false);
        assert!(matches!(
            decision,
            crate::agentic_loop::tool_gate::ToolGateDecision::RequireHumanAnswer { .. }
        ));
    }

    #[test]
    fn strips_complete_and_incomplete_markup() {
        let text =
            "before <tool_call><function=ask_human></function></tool_call> after <function=x>";

        assert_eq!(strip_textual_tool_call_markup(text).trim(), "before  after");
    }
}
