//! Task-local "current agent session id".
//!
//! Tools execute through the session-agnostic [`crate::Tool`] trait
//! (`execute(args, workspace)`), so work spawned *during* a turn — most notably
//! background shell jobs (see `golish-app-core/background_jobs.rs`) — has no
//! built-in way to know which session's agentic loop is running. Rather than
//! thread a session id through every tool signature and factory, the agent
//! bridge wraps each loop in [`with_agent_session`]; any inline-`await`ed work
//! (including tool execution) can then read [`current_agent_session`].
//!
//! This is a *best-effort* attribution: it is `None` outside a wrapped loop
//! (e.g. the eval harness or a direct `execute_tool` call), and callers must
//! treat the absence of a session id as "not attributable", never as an error.

tokio::task_local! {
    static CURRENT_AGENT_SESSION: Option<String>;
}

/// Run `fut` with `session_id` set as the current agent session for the whole
/// future, including any inline-`await`ed tool execution. Nested scopes simply
/// override for their own span (sub-agents reuse the same session id).
pub async fn with_agent_session<F, T>(session_id: Option<String>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_AGENT_SESSION.scope(session_id, fut).await
}

/// The current agent session id, or `None` when not running inside
/// [`with_agent_session`]. Never panics if the task-local is unset.
pub fn current_agent_session() -> Option<String> {
    CURRENT_AGENT_SESSION.try_with(|s| s.clone()).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unset_outside_scope_is_none() {
        assert_eq!(current_agent_session(), None);
    }

    #[tokio::test]
    async fn reads_inside_scope() {
        let got = with_agent_session(Some("sess-1".to_string()), async {
            current_agent_session()
        })
        .await;
        assert_eq!(got, Some("sess-1".to_string()));
    }

    #[tokio::test]
    async fn nested_scope_overrides_then_restores() {
        let outer = with_agent_session(Some("outer".to_string()), async {
            let inner =
                with_agent_session(Some("inner".to_string()), async { current_agent_session() })
                    .await;
            (inner, current_agent_session())
        })
        .await;
        assert_eq!(outer.0, Some("inner".to_string()));
        assert_eq!(outer.1, Some("outer".to_string()));
    }

    #[tokio::test]
    async fn none_scope_is_not_attributable() {
        let got = with_agent_session(None, async { current_agent_session() }).await;
        assert_eq!(got, None);
    }
}
