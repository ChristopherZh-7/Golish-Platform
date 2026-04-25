//! Terminal error state propagation between the agentic loop and the bridge.
//!
//! When the agentic loop emits a terminal `AiEvent::Error` it wraps it in
//! [`TerminalErrorEmitted`] (defined in `agentic_loop::context`). The bridge
//! uses the helpers here to (a) detect that marker so it can avoid emitting a
//! duplicate error event, and (b) recover any partial response / final history
//! the loop captured before bailing out.

use rig::completion::Message;

use super::super::agentic_loop::TerminalErrorEmitted;

/// Recovered state attached to a [`TerminalErrorEmitted`] error.
#[derive(Debug, Clone)]
pub(super) struct TerminalErrorState {
    pub partial_response: Option<String>,
    pub final_history: Option<Vec<Message>>,
}

/// Returns true when the bridge should emit an `AiEvent::Error` for this error.
///
/// Errors carrying [`TerminalErrorEmitted`] have already been surfaced by the
/// agentic loop, so we suppress duplicate emissions here.
pub(super) fn should_emit_execution_error_event(error: &anyhow::Error) -> bool {
    !error.is::<TerminalErrorEmitted>()
}

/// Extract recoverable state (partial response / final history) from a
/// terminal-error wrapped result, if present.
pub(super) fn extract_terminal_error_state(error: &anyhow::Error) -> Option<TerminalErrorState> {
    let terminal_error = error.downcast_ref::<TerminalErrorEmitted>()?;

    Some(TerminalErrorState {
        partial_response: terminal_error.partial_response().map(ToOwned::to_owned),
        final_history: terminal_error.final_history().map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{Text, UserContent};
    use rig::one_or_many::OneOrMany;

    #[test]
    fn should_emit_execution_error_for_normal_errors() {
        let err = anyhow::anyhow!("regular execution failure");
        assert!(should_emit_execution_error_event(&err));
    }

    #[test]
    fn should_not_emit_execution_error_for_terminal_error_marker() {
        let err = anyhow::Error::new(TerminalErrorEmitted::new("already emitted"));
        assert!(!should_emit_execution_error_event(&err));
    }

    #[test]
    fn extract_terminal_error_state_returns_none_for_non_terminal_error() {
        let err = anyhow::anyhow!("regular execution failure");
        assert!(extract_terminal_error_state(&err).is_none());
    }

    #[test]
    fn extract_terminal_error_state_returns_partial_response_and_history() {
        let history = vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "hello".to_string(),
            })),
        }];

        let err = anyhow::Error::new(TerminalErrorEmitted::with_partial_state(
            "stream failed",
            Some("partial assistant text".to_string()),
            Some(history),
        ));

        let state = extract_terminal_error_state(&err).expect("expected terminal error state");
        assert_eq!(
            state.partial_response.as_deref(),
            Some("partial assistant text")
        );
        assert_eq!(state.final_history.as_ref().map(Vec::len), Some(1));
    }
}
