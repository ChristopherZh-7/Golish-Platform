use rig::message::ToolCall;
use serde_json::{Map, Value};

use crate::agentic_loop::tool_intent::ToolIntent;

#[derive(Debug, Clone)]
struct TextualToolCall {
    name: String,
    arguments: Value,
    raw_span: String,
}

pub(super) fn extract_textual_tool_call(text: &str, iteration: usize) -> Option<ToolCall> {
    extract_textual_tool_intent(text, iteration).map(ToolIntent::into_tool_call)
}

pub(super) fn extract_textual_tool_intent(text: &str, iteration: usize) -> Option<ToolIntent> {
    let candidates = parse_textual_tool_calls(text);
    let selected = candidates
        .iter()
        .find(|call| call.name == "ask_human")
        .or_else(|| candidates.first())?;

    let id = format!("textual-tool-call-{iteration}-0");
    Some(ToolIntent::recovered_textual_xml(
        id,
        selected.name.clone(),
        selected.arguments.clone(),
        Some(selected.raw_span.clone()),
    ))
}

pub(super) fn strip_textual_tool_call_markup(text: &str) -> String {
    let without_tool_blocks = remove_tag_blocks(text, "<tool_call", "</tool_call>");
    remove_tag_blocks(&without_tool_blocks, "<function=", "</function>")
}

fn parse_textual_tool_calls(text: &str) -> Vec<TextualToolCall> {
    let mut calls = Vec::new();
    let mut search_start = 0;

    while let Some(function_rel) = text[search_start..].find("<function=") {
        let function_start = search_start + function_rel;
        let name_start = function_start + "<function=".len();
        let Some(name_end_rel) = text[name_start..].find('>') else {
            break;
        };
        let name_end = name_start + name_end_rel;
        let name = text[name_start..name_end].trim();
        if name.is_empty() {
            search_start = name_end + 1;
            continue;
        }

        let body_start = name_end + 1;
        let Some(body_end_rel) = text[body_start..].find("</function>") else {
            break;
        };
        let body_end = body_start + body_end_rel;
        let body = &text[body_start..body_end];
        let raw_span = text[function_start..body_end + "</function>".len()].to_string();

        calls.push(TextualToolCall {
            name: name.to_string(),
            arguments: parse_parameters(body),
            raw_span,
        });
        search_start = body_end + "</function>".len();
    }

    calls
}

fn parse_parameters(body: &str) -> Value {
    let mut args = Map::new();
    let mut search_start = 0;

    while let Some(param_rel) = body[search_start..].find("<parameter=") {
        let param_start = search_start + param_rel;
        let key_start = param_start + "<parameter=".len();
        let Some(key_end_rel) = body[key_start..].find('>') else {
            break;
        };
        let key_end = key_start + key_end_rel;
        let key = body[key_start..key_end].trim();
        if key.is_empty() {
            search_start = key_end + 1;
            continue;
        }

        let value_start = key_end + 1;
        let Some(value_end_rel) = body[value_start..].find("</parameter>") else {
            break;
        };
        let value_end = value_start + value_end_rel;
        let raw_value = body[value_start..value_end].trim();
        args.insert(key.to_string(), parse_parameter_value(key, raw_value));
        search_start = value_end + "</parameter>".len();
    }

    Value::Object(args)
}

fn parse_parameter_value(key: &str, raw_value: &str) -> Value {
    if key == "options" && matches!(raw_value, "" | "None" | "none" | "null") {
        return Value::Array(Vec::new());
    }

    serde_json::from_str(raw_value).unwrap_or_else(|_| Value::String(raw_value.to_string()))
}

fn remove_tag_blocks(text: &str, open_prefix: &str, close_tag: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(open_rel) = text[cursor..].find(open_prefix) {
        let open = cursor + open_rel;
        output.push_str(&text[cursor..open]);

        let block_body_start = open + open_prefix.len();
        if let Some(close_rel) = text[block_body_start..].find(close_tag) {
            cursor = block_body_start + close_rel + close_tag.len();
        } else {
            cursor = text.len();
            break;
        }
    }

    output.push_str(&text[cursor..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

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
