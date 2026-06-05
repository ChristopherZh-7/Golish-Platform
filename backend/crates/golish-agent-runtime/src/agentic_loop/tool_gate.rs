//! Deterministic gate for normalized tool intents.
//!
//! The gate is deliberately conservative for recovered textual calls. A model
//! can ask to perform a side effect, but Golish decides whether that is allowed,
//! requires approval, or must be rejected before dispatch.

use crate::agentic_loop::tool_intent::{ToolIntent, ToolIntentSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGateDecision {
    Allow,
    RequireApproval { reason: String },
    RequireHumanAnswer { question: String },
    Reject { reason: String },
}

pub fn decide_tool_intent(intent: &ToolIntent, target_registered: bool) -> ToolGateDecision {
    if intent.name == "ask_human" {
        let question = intent
            .args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("Please confirm before continuing.")
            .to_string();
        return ToolGateDecision::RequireHumanAnswer { question };
    }

    if intent.name == "manage_targets" {
        let action = intent
            .args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if action == "add" && intent.source != ToolIntentSource::NativeToolCall {
            return ToolGateDecision::RequireApproval {
                reason: "Recovered textual target-add intent requires explicit user approval"
                    .to_string(),
            };
        }
    }

    ToolGateDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn intent(name: &str, args: serde_json::Value, source: ToolIntentSource) -> ToolIntent {
        ToolIntent {
            id: "intent-1".to_string(),
            name: name.to_string(),
            args,
            source,
            confidence: if source == ToolIntentSource::NativeToolCall {
                1.0
            } else {
                0.7
            },
            raw_span: None,
        }
    }

    #[test]
    fn ask_human_is_hard_barrier() {
        let intent = intent(
            "ask_human",
            json!({"question": "Add example.com?"}),
            ToolIntentSource::TextualXml,
        );

        assert_eq!(
            decide_tool_intent(&intent, false),
            ToolGateDecision::RequireHumanAnswer {
                question: "Add example.com?".to_string()
            }
        );
    }

    #[test]
    fn recovered_target_add_requires_approval() {
        let intent = intent(
            "manage_targets",
            json!({"action": "add", "targets": [{"value": "example.com"}]}),
            ToolIntentSource::TextualXml,
        );

        assert!(matches!(
            decide_tool_intent(&intent, false),
            ToolGateDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn native_read_only_or_non_add_action_is_allowed() {
        let intent = intent(
            "manage_targets",
            json!({"action": "list"}),
            ToolIntentSource::NativeToolCall,
        );

        assert_eq!(decide_tool_intent(&intent, false), ToolGateDecision::Allow);
    }

}
