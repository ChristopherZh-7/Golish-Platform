//! E3 · provider failover decision logic.
//!
//! When the primary model fails with a transient / unavailable error and a
//! fallback model is configured via `GOLISH_LLM_FALLBACK_MODEL`, the bridge
//! rebuilds a client for the fallback model (via the LLM client factory) and
//! retries the turn once. Default OFF: env unset → no behaviour change.
//!
//! These functions are the pure, unit-tested core of that decision; the actual
//! rebuild + re-dispatch lives in `execution.rs`.

/// Read the configured fallback model name, if any.
///
/// `GOLISH_LLM_FALLBACK_MODEL` is a model id on the **same provider** as the
/// primary. The forks bake the model into the client at construction (so a
/// `CompletionRequest.model` override is a no-op); the failover client is
/// therefore rebuilt for this model name via the client factory. Empty /
/// whitespace-only values are treated as unset (OFF).
pub(super) fn fallback_model() -> Option<String> {
    std::env::var("GOLISH_LLM_FALLBACK_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether a primary-model failure is the kind a *different model* can
/// plausibly recover.
///
/// Conservative by design: user cancellations, auth failures, and context
/// overflow are NOT eligible — a fallback model won't help, and re-running after
/// the user explicitly stopped would be wrong. Everything else (transient 5xx,
/// timeouts, rate limits, model-unavailable, generic provider errors) is
/// eligible.
pub(super) fn error_is_failover_eligible(error_str: &str) -> bool {
    let lower = error_str.to_ascii_lowercase();
    let non_recoverable = lower.contains("stopped by user")
        || lower.contains("cancel")
        || lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("prompt is too long")
        || lower.contains("context_length")
        || lower.contains("too many tokens");
    !non_recoverable
}

/// Decide the failover target model, if a failed turn should fail over.
///
/// Returns `Some(fallback_model)` only when **all** hold: a non-empty fallback
/// is configured, it differs from the current model, a client factory is
/// available to build it, and the error is recoverable by a different model.
pub(super) fn failover_decision(
    error_str: &str,
    fallback: Option<&str>,
    current_model: &str,
    has_factory: bool,
) -> Option<String> {
    let fb = fallback?;
    if fb.is_empty() || fb == current_model || !has_factory {
        return None;
    }
    if !error_is_failover_eligible(error_str) {
        return None;
    }
    Some(fb.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_errors_are_failover_eligible() {
        assert!(error_is_failover_eligible("503 Service Unavailable"));
        assert!(error_is_failover_eligible("Rate limit exceeded (429)"));
        assert!(error_is_failover_eligible("Request timed out after 180s"));
        assert!(error_is_failover_eligible(
            "The selected model is not deployed on the NVIDIA NIM endpoint"
        ));
        assert!(error_is_failover_eligible(
            "Failed to start streaming response"
        ));
    }

    #[test]
    fn non_recoverable_errors_are_not_failover_eligible() {
        assert!(!error_is_failover_eligible("Agent stopped by user"));
        assert!(!error_is_failover_eligible("Agent cancelled by user"));
        assert!(!error_is_failover_eligible(
            "Authentication failed. Please check your API credentials."
        ));
        assert!(!error_is_failover_eligible("401 Unauthorized"));
        assert!(!error_is_failover_eligible(
            "prompt is too long for this model"
        ));
        assert!(!error_is_failover_eligible(
            "context_length_exceeded: too many tokens"
        ));
    }

    #[test]
    fn no_fallback_configured_means_no_failover() {
        assert_eq!(
            failover_decision("503", None, "gpt-4o-mini", true),
            None,
            "unset fallback → OFF (no behaviour change)"
        );
        assert_eq!(
            failover_decision("503", Some(""), "gpt-4o-mini", true),
            None,
            "empty fallback → OFF"
        );
    }

    #[test]
    fn fallback_equal_to_current_model_is_skipped() {
        assert_eq!(
            failover_decision("503", Some("gpt-4o-mini"), "gpt-4o-mini", true),
            None,
            "failing over to the same model is pointless"
        );
    }

    #[test]
    fn no_factory_means_no_failover() {
        assert_eq!(
            failover_decision("503", Some("gpt-4o"), "gpt-4o-mini", false),
            None,
            "can't rebuild a client without a factory"
        );
    }

    #[test]
    fn cancellation_does_not_fail_over_even_with_fallback() {
        assert_eq!(
            failover_decision("Agent stopped by user", Some("gpt-4o"), "gpt-4o-mini", true),
            None,
            "never re-run after the user stopped the agent"
        );
    }

    #[test]
    fn eligible_error_with_distinct_fallback_and_factory_fails_over() {
        assert_eq!(
            failover_decision(
                "503 Service Unavailable",
                Some("gpt-4o"),
                "gpt-4o-mini",
                true
            ),
            Some("gpt-4o".to_string())
        );
    }
}
