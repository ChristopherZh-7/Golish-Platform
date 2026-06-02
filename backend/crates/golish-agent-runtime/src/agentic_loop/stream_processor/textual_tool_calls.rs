//! Agent-runtime adapter over the shared textual tool-call parser.
//!
//! The parsing/stripping logic lives in [`golish_core::textual_tool_call`] so it
//! can be shared with the sub-agent executor. This module only wraps the parsed
//! result into the runtime's [`ToolIntent`] / `ToolCall` shapes.

use rig::message::ToolCall;

pub(super) use golish_core::strip_textual_tool_call_markup;

use crate::agentic_loop::tool_intent::ToolIntent;

pub(super) fn extract_textual_tool_call(text: &str, iteration: usize) -> Option<ToolCall> {
    extract_textual_tool_intent(text, iteration).map(ToolIntent::into_tool_call)
}

pub(super) fn extract_textual_tool_intent(text: &str, iteration: usize) -> Option<ToolIntent> {
    let selected = golish_core::select_textual_tool_call(text)?;

    let id = format!("textual-tool-call-{iteration}-0");
    Some(ToolIntent::recovered_textual_xml(
        id,
        selected.name,
        selected.arguments,
        Some(selected.raw_span),
    ))
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

        let intent = extract_textual_tool_intent(text, 2).expect("expected textual tool intent");
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
    fn extracts_json_parameter_values() {
        let text = r#"<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>"#;

        let call = extract_textual_tool_call(text, 1).expect("expected textual tool call");
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

        let intent = extract_textual_tool_intent(text, 3).expect("expected recovered intent");
        assert_eq!(intent.name, "ask_human");
        assert_eq!(intent.args["question"], "是否添加 example.com 到目标列表?");

        let decision = crate::agentic_loop::tool_gate::decide_tool_intent(&intent, false);
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
