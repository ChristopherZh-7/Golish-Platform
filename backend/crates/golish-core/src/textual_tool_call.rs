//! Pure parser for textual (pseudo-XML) tool calls.
//!
//! Some model families (notably Xiaomi MiMo) express tool calls as
//! `<tool_call><function=name><parameter=key>value</parameter></function></tool_call>`
//! markup embedded in assistant text instead of emitting native structured
//! tool calls. Runtimes that only collect native tool calls would otherwise
//! leak this markup as prose and never execute the tool.
//!
//! This module is intentionally dependency-light (serde_json + std only) so it
//! can be shared by both the main agentic loop (`golish-agent-runtime`) and the
//! sub-agent executor (`golish-sub-agents`). Each caller converts the returned
//! [`TextualToolCall`] into its own tool-call representation.

use serde_json::{Map, Value};

/// A tool call recovered from textual pseudo-XML markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextualToolCall {
    pub name: String,
    pub arguments: Value,
    pub raw_span: String,
}

/// Select the most relevant textual tool call from `text`.
///
/// Prefers an `ask_human` barrier over any other call so that a self-answered
/// follow-up side effect in the same textual block cannot bypass the human
/// gate; otherwise returns the first parsed call.
pub fn select_textual_tool_call(text: &str) -> Option<TextualToolCall> {
    let candidates = parse_textual_tool_calls(text);
    candidates
        .iter()
        .find(|call| call.name == "ask_human")
        .or_else(|| candidates.first())
        .cloned()
}

/// Parse every `<function=...>...</function>` block from `text`.
pub fn parse_textual_tool_calls(text: &str) -> Vec<TextualToolCall> {
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

/// Remove `<tool_call>...</tool_call>` and `<function=...>...</function>` markup
/// from `text` so leaked markup is not shown to the user. Unterminated blocks
/// are dropped from the opening tag onward.
pub fn strip_textual_tool_call_markup(text: &str) -> String {
    let without_tool_blocks = remove_tag_blocks(text, "<tool_call", "</tool_call>");
    remove_tag_blocks(&without_tool_blocks, "<function=", "</function>")
}

/// Outcome of finalizing assistant text that may carry textual tool-call markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedAssistantText {
    /// Text with all `<tool_call>` / `<function=...>` markup removed. Always safe
    /// to stream, persist, or return to the user.
    pub clean_text: String,
    /// A tool call recovered from the markup, present only when `allow_recovery`
    /// was set and the text contained a parseable call.
    pub recovered: Option<TextualToolCall>,
}

/// Finalize assistant text for display and optional tool recovery in one place.
///
/// Stripping markup is **unconditional** — leaked `<tool_call>` markup must never
/// reach the user, regardless of whether a tool was recovered or the provider
/// already produced native tool calls. Recovery of a call to execute is gated
/// behind `allow_recovery`, which callers set to `true` only when no native tool
/// call was produced (so the same intent is never executed twice).
///
/// This is the single entry point every assistant-text finalization site should
/// route through (main loop, sub-agent loop, tool-less final summary) so no path
/// can forget to strip markup.
pub fn finalize_assistant_text(text: &str, allow_recovery: bool) -> FinalizedAssistantText {
    let recovered = if allow_recovery {
        select_textual_tool_call(text)
    } else {
        None
    };

    FinalizedAssistantText {
        clean_text: strip_textual_tool_call_markup(text),
        recovered,
    }
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
    fn selects_ask_human_before_follow_up_side_effect() {
        let text = r#"需要确认。
<tool_call>
<function=ask_human>
<parameter=question>example.com 未注册为目标，是否添加？</parameter>
<parameter=options>None</parameter>
</function>
</tool_call>
<tool_call>
<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>
</tool_call>"#;

        let call = select_textual_tool_call(text).expect("expected textual tool call");
        assert_eq!(call.name, "ask_human");
        assert_eq!(call.arguments["options"], Value::Array(Vec::new()));
        assert!(call.raw_span.contains("ask_human"));
    }

    #[test]
    fn parses_json_parameter_values() {
        let text = r#"<function=manage_targets>
<parameter=action>add</parameter>
<parameter=targets>[{"value":"example.com"}]</parameter>
</function>"#;

        let call = select_textual_tool_call(text).expect("expected textual tool call");
        assert_eq!(call.name, "manage_targets");
        assert_eq!(call.arguments["action"], "add");
        assert_eq!(call.arguments["targets"][0]["value"], "example.com");
    }

    #[test]
    fn returns_none_without_markup() {
        assert!(select_textual_tool_call("just regular assistant prose").is_none());
        assert!(parse_textual_tool_calls("no markup here").is_empty());
    }

    #[test]
    fn strips_complete_and_incomplete_markup() {
        let text =
            "before <tool_call><function=ask_human></function></tool_call> after <function=x>";
        assert_eq!(strip_textual_tool_call_markup(text).trim(), "before  after");
    }

    #[test]
    fn finalize_strips_markup_even_when_recovery_disabled() {
        // Mirrors the tool-less final-summary path and the "native call already
        // present" path: markup must be stripped, but nothing may be recovered.
        let text = "I'll read the files.\n\
<tool_call>\n\
<function=read_file>\n\
<parameter=path>a.txt</parameter>\n\
</function>\n\
</tool_call>";

        let finalized = finalize_assistant_text(text, false);

        assert_eq!(finalized.recovered, None, "recovery must stay disabled");
        assert!(
            !finalized.clean_text.contains("<tool_call>")
                && !finalized.clean_text.contains("<function="),
            "markup must be stripped, got: {}",
            finalized.clean_text
        );
        assert!(finalized.clean_text.contains("I'll read the files."));
    }

    #[test]
    fn finalize_recovers_and_strips_when_allowed() {
        let text = "checking.\n\
<tool_call>\n\
<function=search_memories>\n\
<parameter=query>example.com recon</parameter>\n\
</function>\n\
</tool_call>";

        let finalized = finalize_assistant_text(text, true);

        let recovered = finalized.recovered.expect("expected recovered call");
        assert_eq!(recovered.name, "search_memories");
        assert_eq!(recovered.arguments["query"], "example.com recon");
        assert!(
            !finalized.clean_text.contains("<tool_call>")
                && !finalized.clean_text.contains("<function="),
            "markup must be stripped even after recovery, got: {}",
            finalized.clean_text
        );
        assert!(finalized.clean_text.contains("checking."));
    }

    #[test]
    fn finalize_leaves_plain_prose_untouched() {
        let finalized = finalize_assistant_text("just a normal answer", true);
        assert_eq!(finalized.recovered, None);
        assert_eq!(finalized.clean_text, "just a normal answer");
    }
}
