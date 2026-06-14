//! Pure parser for textual (pseudo-XML) tool calls.
//!
//! Some model families express tool calls as pseudo-XML markup embedded in
//! assistant text instead of emitting native structured tool calls. Runtimes
//! that only collect native tool calls would otherwise leak this markup as prose
//! and never execute the tool. Two dialects are recognized:
//!
//! - **MiMo / GLM** `<function=name>`:
//!   `<tool_call><function=name><parameter=key>value</parameter></function></tool_call>`
//! - **Anthropic-style** `<invoke name=...>` (emitted as text by DeepSeek V4
//!   `native_best_effort` models when their native tool-call channel degrades):
//!   `<tool_calls><invoke name="name"><parameter name="key">value</parameter></invoke></tool_calls>`
//!   The `<parameter>` tag may carry extra attributes (e.g. `string="true"`),
//!   which are ignored — only the `name` attribute and inner value matter.
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

/// Select every textual tool call to execute from `text`.
///
/// Like [`select_textual_tool_call`] but returns *all* parsed calls so a turn
/// that batches multiple `<function=...>` blocks (notably Xiaomi MiMo, which
/// frequently emits 2–3 in one response) executes them all instead of silently
/// dropping every call after the first — the dropped calls otherwise force the
/// model to re-issue them on later turns, inflating iteration count.
///
/// The `ask_human` barrier still wins: if any call is an `ask_human`, ONLY that
/// one is returned so a self-answered follow-up side effect in the same block
/// cannot bypass the human gate (identical guarantee to
/// [`select_textual_tool_call`]).
pub fn select_textual_tool_calls(text: &str) -> Vec<TextualToolCall> {
    let candidates = parse_textual_tool_calls(text);
    if let Some(ask) = candidates.iter().find(|call| call.name == "ask_human") {
        return vec![ask.clone()];
    }
    candidates
}

/// Parse every textual tool call from `text`, across both supported dialects
/// (`<function=...>` MiMo/GLM and `<invoke name=...>` Anthropic-style). Calls
/// are returned in dialect order (function-style first, then invoke-style); the
/// `ask_human` barrier in [`select_textual_tool_call(s)`] still applies across
/// the combined set.
pub fn parse_textual_tool_calls(text: &str) -> Vec<TextualToolCall> {
    let mut calls = parse_function_style_tool_calls(text);
    calls.extend(parse_invoke_style_tool_calls(text));
    calls
}

/// Parse every `<function=name>...</function>` block (MiMo / GLM dialect).
fn parse_function_style_tool_calls(text: &str) -> Vec<TextualToolCall> {
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

/// Parse every `<invoke name="...">...</invoke>` block (Anthropic-style dialect).
///
/// Inner parameters use `<parameter name="key">value</parameter>` (the tag may
/// carry extra attributes such as `string="true"`, which are ignored). This is
/// the shape DeepSeek V4 (`native_best_effort`) leaks into assistant text when
/// its native tool-call channel degrades.
fn parse_invoke_style_tool_calls(text: &str) -> Vec<TextualToolCall> {
    let mut calls = Vec::new();
    let mut search_start = 0;

    while let Some(rel) = text[search_start..].find("<invoke") {
        let invoke_start = search_start + rel;
        let attrs_start = invoke_start + "<invoke".len();
        let Some(tag_end_rel) = text[attrs_start..].find('>') else {
            break;
        };
        let tag_end = attrs_start + tag_end_rel;
        let name = extract_quoted_attr(&text[attrs_start..tag_end], "name").unwrap_or_default();

        let body_start = tag_end + 1;
        let Some(body_end_rel) = text[body_start..].find("</invoke>") else {
            break;
        };
        let body_end = body_start + body_end_rel;
        if name.is_empty() {
            search_start = body_end + "</invoke>".len();
            continue;
        }
        let body = &text[body_start..body_end];
        let raw_span = text[invoke_start..body_end + "</invoke>".len()].to_string();

        calls.push(TextualToolCall {
            name,
            arguments: parse_invoke_parameters(body),
            raw_span,
        });
        search_start = body_end + "</invoke>".len();
    }

    calls
}

/// Remove every textual tool-call dialect's markup from `text` so leaked markup
/// is not shown to the user: the MiMo `<tool_call>` / `<function=...>` shape and
/// the Anthropic-style `<tool_calls>` / `<invoke ...>` / `<parameter ...>` shape.
/// Unterminated blocks are dropped from the opening tag onward.
pub fn strip_textual_tool_call_markup(text: &str) -> String {
    // Order matters: strip the plural Anthropic `<tool_calls>` wrapper before the
    // singular MiMo `<tool_call>` — the latter's open prefix `<tool_call` is a
    // prefix of `<tool_calls`, so doing it first would mis-span a plural wrapper.
    let s = remove_tag_blocks(text, "<tool_calls", "</tool_calls>");
    let s = remove_tag_blocks(&s, "<tool_call", "</tool_call>");
    let s = remove_tag_blocks(&s, "<invoke", "</invoke>");
    let s = remove_tag_blocks(&s, "<function=", "</function>");
    // Drop any orphaned `<parameter ...>...</parameter>` left outside a block
    // (e.g. a streamed leak whose enclosing invoke/function was already removed).
    remove_tag_blocks(&s, "<parameter", "</parameter>")
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

/// Parse `<parameter name="key">value</parameter>` pairs (Anthropic-style).
///
/// The opening tag may carry extra attributes (e.g. `string="true"`); only the
/// `name` attribute is read for the key and the inner text for the value.
fn parse_invoke_parameters(body: &str) -> Value {
    let mut args = Map::new();
    let mut search_start = 0;

    while let Some(param_rel) = body[search_start..].find("<parameter") {
        let param_start = search_start + param_rel;
        let attrs_start = param_start + "<parameter".len();
        let Some(tag_end_rel) = body[attrs_start..].find('>') else {
            break;
        };
        let tag_end = attrs_start + tag_end_rel;
        let key = extract_quoted_attr(&body[attrs_start..tag_end], "name");

        let value_start = tag_end + 1;
        let Some(value_end_rel) = body[value_start..].find("</parameter>") else {
            break;
        };
        let value_end = value_start + value_end_rel;
        let raw_value = body[value_start..value_end].trim();
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            args.insert(k.clone(), parse_parameter_value(&k, raw_value));
        }
        search_start = value_end + "</parameter>".len();
    }

    Value::Object(args)
}

/// Extract the value of a quoted attribute (`key="value"` or `key='value'`) from
/// an opening-tag attribute slice. Returns `None` if the attribute is absent or
/// unquoted. Only matches standalone attributes (preceded by start/whitespace)
/// so `name=` is not found inside another attribute's value.
fn extract_quoted_attr(attrs: &str, key: &str) -> Option<String> {
    let mut search = 0;
    while let Some(rel) = attrs[search..].find(key) {
        let kpos = search + rel;
        let preceded_ok = kpos == 0 || attrs[..kpos].ends_with(char::is_whitespace);
        let after = kpos + key.len();
        let rest = attrs[after..].trim_start();
        if preceded_ok {
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim_start();
                let quote = rest.chars().next()?;
                if quote == '"' || quote == '\'' {
                    let val = &rest[quote.len_utf8()..];
                    if let Some(end) = val.find(quote) {
                        return Some(val[..end].to_string());
                    }
                }
                return None;
            }
        }
        search = after;
    }
    None
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
    fn select_all_returns_every_non_barrier_call_in_order() {
        // MiMo frequently batches several calls in one turn; all must execute,
        // not just the first (the silent-drop churn bug).
        let text = r#"先列候选再创建。
<tool_call>
<function=manage_organizations>
<parameter=action>propose_candidates</parameter>
<parameter=candidates>["ACME Corp","Example Ltd"]</parameter>
</function>
</tool_call>
<tool_call>
<function=update_plan>
<parameter=plan>[{"status":"in_progress","step":"create"}]</parameter>
</function>
</tool_call>"#;

        let calls = select_textual_tool_calls(text);
        assert_eq!(calls.len(), 2, "both batched calls must be returned");
        assert_eq!(calls[0].name, "manage_organizations");
        assert_eq!(calls[0].arguments["action"], "propose_candidates");
        assert_eq!(calls[1].name, "update_plan");
    }

    #[test]
    fn select_all_keeps_ask_human_barrier_and_drops_follow_up_side_effects() {
        // When an ask_human is present the barrier must still win: only the
        // ask_human runs so a self-answered create cannot bypass the human gate.
        let text = r#"<tool_call>
<function=ask_human>
<parameter=question>确认候选单元?</parameter>
<parameter=input_type>unit_review</parameter>
</function>
</tool_call>
<tool_call>
<function=manage_organizations>
<parameter=action>create</parameter>
<parameter=name>ACME Corp</parameter>
</function>
</tool_call>"#;

        let calls = select_textual_tool_calls(text);
        assert_eq!(calls.len(), 1, "ask_human barrier must drop the follow-up");
        assert_eq!(calls[0].name, "ask_human");
        assert_eq!(calls[0].arguments["input_type"], "unit_review");
    }

    #[test]
    fn select_all_empty_without_markup() {
        assert!(select_textual_tool_calls("plain prose, no tool calls").is_empty());
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

    // ── Anthropic-style `<invoke name=...>` dialect (DeepSeek V4 leak) ──────────

    #[test]
    fn parses_invoke_style_tool_call_with_extra_attributes() {
        // Exact shape leaked by deepseek-v4-flash into the target_intel sub-agent
        // output (the `string="true"` attribute must be ignored).
        let text = r#"Let me read the amass output.
<tool_calls>
<invoke name="pentest_run">
<parameter name="args" string="true">/tmp/pingan_amass.txt 2>/dev/null || echo "File not found"</parameter>
<parameter name="tool_name" string="true">dig</parameter>
</invoke>
</tool_calls>"#;

        let call = select_textual_tool_call(text).expect("expected invoke-style call");
        assert_eq!(call.name, "pentest_run");
        assert_eq!(call.arguments["tool_name"], "dig");
        assert_eq!(
            call.arguments["args"],
            "/tmp/pingan_amass.txt 2>/dev/null || echo \"File not found\""
        );
        assert!(call.raw_span.contains("<invoke name=\"pentest_run\">"));
    }

    #[test]
    fn strips_invoke_style_and_tool_calls_wrapper() {
        let text = "Recon summary done.\n\
<tool_calls>\n\
<invoke name=\"pentest_run\">\n\
<parameter name=\"tool_name\" string=\"true\">dig</parameter>\n\
</invoke>\n\
</tool_calls>";

        let stripped = strip_textual_tool_call_markup(text);
        assert!(!stripped.contains("<invoke"), "leaked invoke: {stripped}");
        assert!(
            !stripped.contains("<parameter"),
            "leaked parameter: {stripped}"
        );
        assert!(
            !stripped.contains("tool_calls"),
            "leaked wrapper: {stripped}"
        );
        assert!(stripped.contains("Recon summary done."));
    }

    #[test]
    fn strips_unterminated_invoke_leak_from_open_tag_onward() {
        // A truncated turn (e.g. hitting max_tokens) can leave an unterminated
        // `<tool_calls>` / `<invoke>` — everything from the open tag is dropped.
        let text = "Partial answer.\n<tool_calls>\n<invoke name=\"pentest_run\">\n<parameter name=\"args\">/tmp/x";
        let stripped = strip_textual_tool_call_markup(text);
        assert_eq!(stripped.trim(), "Partial answer.");
    }

    #[test]
    fn finalize_strips_invoke_leak_when_recovery_disabled() {
        let text = "done.\n<tool_calls><invoke name=\"x\"><parameter name=\"a\">1</parameter></invoke></tool_calls>";
        let finalized = finalize_assistant_text(text, false);
        assert_eq!(finalized.recovered, None, "recovery must stay disabled");
        assert!(!finalized.clean_text.contains("<invoke"));
        assert!(finalized.clean_text.contains("done."));
    }

    #[test]
    fn finalize_recovers_invoke_style_when_allowed() {
        let text = "checking.\n<invoke name=\"search_memories\"><parameter name=\"query\">acme recon</parameter></invoke>";
        let finalized = finalize_assistant_text(text, true);
        let recovered = finalized.recovered.expect("expected recovered invoke call");
        assert_eq!(recovered.name, "search_memories");
        assert_eq!(recovered.arguments["query"], "acme recon");
        assert!(!finalized.clean_text.contains("<invoke"));
        assert!(finalized.clean_text.contains("checking."));
    }

    #[test]
    fn invoke_ask_human_barrier_wins_over_follow_up() {
        let text = r#"<tool_calls>
<invoke name="ask_human">
<parameter name="question">add example.com?</parameter>
</invoke>
<invoke name="manage_targets">
<parameter name="action">add</parameter>
</invoke>
</tool_calls>"#;
        let calls = select_textual_tool_calls(text);
        assert_eq!(calls.len(), 1, "ask_human barrier must drop the follow-up");
        assert_eq!(calls[0].name, "ask_human");
    }

    #[test]
    fn extract_quoted_attr_handles_quotes_and_absence() {
        assert_eq!(
            extract_quoted_attr(" name=\"args\" string=\"true\"", "name").as_deref(),
            Some("args")
        );
        assert_eq!(
            extract_quoted_attr(" name='single'", "name").as_deref(),
            Some("single")
        );
        assert_eq!(extract_quoted_attr(" string=\"true\"", "name"), None);
    }
}
