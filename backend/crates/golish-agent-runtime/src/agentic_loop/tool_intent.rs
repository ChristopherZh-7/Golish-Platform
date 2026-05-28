//! Normalized model intent to call a tool.
//!
//! Tool intents sit between provider output and tool dispatch. Native provider
//! tool calls and recovered textual calls can share one safety gate before any
//! executor sees them.

use rig::message::{ToolCall, ToolFunction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolIntentSource {
    NativeToolCall,
    TextualXml,
    TextualJson,
    Recovered,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolIntent {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
    pub source: ToolIntentSource,
    pub confidence: f32,
    pub raw_span: Option<String>,
}

impl ToolIntent {
    pub fn from_native(call: ToolCall) -> Self {
        Self {
            id: call.id,
            name: call.function.name,
            args: call.function.arguments,
            source: ToolIntentSource::NativeToolCall,
            confidence: 1.0,
            raw_span: None,
        }
    }

    pub fn recovered_textual_xml(
        id: String,
        name: String,
        args: serde_json::Value,
        raw_span: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            args,
            source: ToolIntentSource::TextualXml,
            confidence: 0.7,
            raw_span,
        }
    }

    pub fn into_tool_call(self) -> ToolCall {
        ToolCall {
            id: self.id.clone(),
            call_id: Some(self.id),
            function: ToolFunction {
                name: self.name,
                arguments: self.args,
            },
            signature: None,
            additional_params: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_tool_call_becomes_high_confidence_intent() {
        let call = ToolCall {
            id: "tc-1".to_string(),
            call_id: Some("tc-1".to_string()),
            function: ToolFunction {
                name: "read_file".to_string(),
                arguments: json!({"path": "README.md"}),
            },
            signature: None,
            additional_params: None,
        };

        let intent = ToolIntent::from_native(call);
        assert_eq!(intent.id, "tc-1");
        assert_eq!(intent.name, "read_file");
        assert_eq!(intent.source, ToolIntentSource::NativeToolCall);
        assert_eq!(intent.confidence, 1.0);
    }

    #[test]
    fn textual_xml_intent_round_trips_to_tool_call() {
        let intent = ToolIntent::recovered_textual_xml(
            "textual-1".to_string(),
            "ask_human".to_string(),
            json!({"question": "Proceed?"}),
            Some("<function=ask_human>...</function>".to_string()),
        );

        assert_eq!(intent.source, ToolIntentSource::TextualXml);
        assert!(intent.confidence < 1.0);

        let call = intent.into_tool_call();
        assert_eq!(call.id, "textual-1");
        assert_eq!(call.call_id.as_deref(), Some("textual-1"));
        assert_eq!(call.function.name, "ask_human");
        assert_eq!(call.function.arguments["question"], "Proceed?");
    }
}
