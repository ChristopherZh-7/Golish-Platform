//! Task-local agent attribution.
//!
//! Tools execute through the session-agnostic [`crate::Tool`] trait
//! (`execute(args, workspace)`), so work spawned *during* a turn — most notably
//! background shell jobs (see `golish-app-core/background_jobs.rs`) — has no
//! built-in way to know which session's agentic loop is running. Rather than
//! thread a session id through every tool signature and factory, the agent
//! bridge wraps each loop in [`with_agent_session`]; any inline-`await`ed work
//! (including tool execution) can then read [`current_agent_session`].
//!
//! Individual tool executors can also read [`current_agent_tool_context`] when
//! they need to emit live output back to the exact tool card that launched them.
//!
//! This is a *best-effort* attribution: it is `None` outside a wrapped loop
//! (e.g. the eval harness or a direct `execute_tool` call), and callers must
//! treat the absence of a session id as "not attributable", never as an error.

use tokio::sync::mpsc;

use crate::events::{AiEvent, ToolSource};

/// The currently executing tool call, when a tool is running inside the agentic
/// loop. This is intentionally tiny and UI-oriented: it is only for correlating
/// side-channel output (for example background job stdout/stderr chunks) back to
/// the visible tool card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentToolContext {
    pub request_id: String,
    pub tool_name: String,
    pub source: ToolSource,
    /// Trusted harness operation/stage-attempt identity captured by the agent
    /// runtime. Active tools use this instead of accepting a caller-supplied
    /// operation id from model-visible arguments.
    pub operation_id: Option<uuid::Uuid>,
    /// Active harness organization for tools spawned from a stage run. Background
    /// jobs finish outside the agent turn, so completion listeners need this to
    /// persist structured coverage facts into the correct org.
    pub organization_id: Option<uuid::Uuid>,
}

tokio::task_local! {
    static CURRENT_AGENT_SESSION: Option<String>;
    static CURRENT_AGENT_TOOL_CONTEXT: Option<AgentToolContext>;
    static CURRENT_AGENT_TOOL_OUTPUT_SENDER: Option<mpsc::UnboundedSender<AiEvent>>;
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

/// Run `fut` with `tool_context` set as the current tool call for the whole
/// future. Nested scopes override their own span and restore the outer context.
pub async fn with_agent_tool_context<F, T>(tool_context: Option<AgentToolContext>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_AGENT_TOOL_CONTEXT.scope(tool_context, fut).await
}

/// Run `fut` with a sender that lets ordinary tool implementations emit live
/// output chunks back to the visible tool card without widening the `Tool`
/// trait signature.
pub async fn with_agent_tool_output_sender<F, T>(
    output_sender: Option<mpsc::UnboundedSender<AiEvent>>,
    fut: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_AGENT_TOOL_OUTPUT_SENDER
        .scope(output_sender, fut)
        .await
}

/// The current agent session id, or `None` when not running inside
/// [`with_agent_session`]. Never panics if the task-local is unset.
pub fn current_agent_session() -> Option<String> {
    CURRENT_AGENT_SESSION.try_with(|s| s.clone()).ok().flatten()
}

/// The current tool-call attribution, or `None` when no tool is actively running
/// inside a [`with_agent_tool_context`] scope.
pub fn current_agent_tool_context() -> Option<AgentToolContext> {
    CURRENT_AGENT_TOOL_CONTEXT
        .try_with(|ctx| ctx.clone())
        .ok()
        .flatten()
}

/// The current live-output sender, or `None` outside an agent tool execution
/// scope.
pub fn current_agent_tool_output_sender() -> Option<mpsc::UnboundedSender<AiEvent>> {
    CURRENT_AGENT_TOOL_OUTPUT_SENDER
        .try_with(|sender| sender.clone())
        .ok()
        .flatten()
}

/// Emit a live output chunk for the currently executing tool call. This is
/// best-effort and no-ops when a tool is run outside the agent UI.
pub fn emit_current_agent_tool_output_chunk(chunk: impl Into<String>, stream: impl Into<String>) {
    let chunk = chunk.into();
    if chunk.is_empty() {
        return;
    }
    let Some(tool_context) = current_agent_tool_context() else {
        return;
    };
    let Some(sender) = current_agent_tool_output_sender() else {
        return;
    };
    let _ = sender.send(AiEvent::ToolOutputChunk {
        request_id: tool_context.request_id,
        tool_name: tool_context.tool_name,
        chunk,
        stream: stream.into(),
        source: tool_context.source,
    });
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

    #[tokio::test]
    async fn tool_context_reads_inside_scope() {
        let ctx = AgentToolContext {
            request_id: "req-1".to_string(),
            tool_name: "pentest_run".to_string(),
            source: ToolSource::Main,
            operation_id: Some(uuid::Uuid::new_v4()),
            organization_id: None,
        };
        let got =
            with_agent_tool_context(Some(ctx.clone()), async { current_agent_tool_context() })
                .await;
        assert_eq!(got, Some(ctx));
    }

    #[tokio::test]
    async fn nested_tool_context_overrides_then_restores() {
        let outer = AgentToolContext {
            request_id: "outer".to_string(),
            tool_name: "run_command".to_string(),
            source: ToolSource::Main,
            operation_id: None,
            organization_id: None,
        };
        let inner = AgentToolContext {
            request_id: "inner".to_string(),
            tool_name: "pentest_run".to_string(),
            source: ToolSource::SubAgent {
                agent_id: "recon".to_string(),
                agent_name: "Recon".to_string(),
            },
            operation_id: Some(uuid::Uuid::new_v4()),
            organization_id: Some(uuid::Uuid::nil()),
        };
        let got = with_agent_tool_context(Some(outer.clone()), async {
            let nested = with_agent_tool_context(Some(inner.clone()), async {
                current_agent_tool_context()
            })
            .await;
            (nested, current_agent_tool_context())
        })
        .await;
        assert_eq!(got.0, Some(inner));
        assert_eq!(got.1, Some(outer));
    }

    #[tokio::test]
    async fn tool_output_chunk_emits_inside_scope() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ctx = AgentToolContext {
            request_id: "req-1".to_string(),
            tool_name: "browser_collect_js_api".to_string(),
            source: ToolSource::Main,
            operation_id: None,
            organization_id: None,
        };

        with_agent_tool_context(
            Some(ctx),
            with_agent_tool_output_sender(Some(tx), async {
                emit_current_agent_tool_output_chunk("opening page\n", "stderr");
            }),
        )
        .await;

        match rx.try_recv().expect("chunk should be emitted") {
            AiEvent::ToolOutputChunk {
                request_id,
                tool_name,
                chunk,
                stream,
                source,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(tool_name, "browser_collect_js_api");
                assert_eq!(chunk, "opening page\n");
                assert_eq!(stream, "stderr");
                assert_eq!(source, ToolSource::Main);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
