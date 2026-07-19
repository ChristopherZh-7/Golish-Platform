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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, Notify};

use crate::events::{AiEvent, ToolSource};

/// Trusted worker lease/fencing tuple captured by the runtime before a tool is
/// dispatched. The unit id is intentionally duplicated from
/// [`AgentToolContext::stage_run_unit_id`]: consumers must keep the two equal so
/// a lease cannot be detached from the exact stage unit that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLeaseContext {
    pub worker_run_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub lease_token: uuid::Uuid,
    pub attempt_epoch: i64,
}

/// The currently executing tool call, when a tool is running inside the agentic
/// loop. This is intentionally tiny and UI-oriented: it is only for correlating
/// side-channel output (for example background job stdout/stderr chunks) back to
/// the visible tool card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentToolContext {
    pub request_id: String,
    /// Awaited durable `tool_calls.id`, when tracking is authoritative for this
    /// dispatch. This is never accepted from model-visible tool arguments.
    pub tool_call_record_id: Option<uuid::Uuid>,
    pub tool_name: String,
    pub source: ToolSource,
    /// Trusted harness operation/stage-attempt identity captured by the agent
    /// runtime. Active tools use this instead of accepting a caller-supplied
    /// operation id from model-visible arguments.
    pub operation_id: Option<uuid::Uuid>,
    /// Trusted `stage_runs.id` for the active execution.
    pub stage_execution_id: Option<uuid::Uuid>,
    /// Trusted per-organization stage unit, absent only before Scoping binds its
    /// frozen scope unit or outside the harness.
    pub stage_run_unit_id: Option<uuid::Uuid>,
    /// Active harness organization for tools spawned from a stage run. Background
    /// jobs finish outside the agent turn, so completion listeners need this to
    /// persist structured coverage facts into the correct org.
    pub organization_id: Option<uuid::Uuid>,
    /// Active worker fencing tuple for specialist tools.
    pub worker_lease: Option<WorkerLeaseContext>,
    /// Opaque identity for an approved Candidate verification attempt. The
    /// model never supplies this value and it deliberately contains no action
    /// recipe or authorization material.
    pub candidate_attempt: Option<crate::CandidateAttemptContextRef>,
}

/// Cooperative cancellation owned by a self-bounded tool wrapper. Callers may
/// signal it, but must keep awaiting the wrapper until its child process has
/// been killed, reaped, and its partial/error evidence has landed.
#[derive(Debug, Clone, Default)]
pub struct AgentToolCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl AgentToolCancellation {
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

tokio::task_local! {
    static CURRENT_AGENT_SESSION: Option<String>;
    static CURRENT_AGENT_TOOL_CONTEXT: Option<AgentToolContext>;
    static CURRENT_AGENT_TOOL_OUTPUT_SENDER: Option<mpsc::UnboundedSender<AiEvent>>;
    static CURRENT_AGENT_TOOL_CANCELLATION: Option<AgentToolCancellation>;
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

/// Run a self-bounded wrapper with its host-owned cooperative cancellation
/// handle installed for all inline child-process work.
pub async fn with_agent_tool_cancellation<F, T>(
    cancellation: Option<AgentToolCancellation>,
    fut: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_AGENT_TOOL_CANCELLATION
        .scope(cancellation, fut)
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

/// Cooperative cancellation for the current self-bounded wrapper, if any.
pub fn current_agent_tool_cancellation() -> Option<AgentToolCancellation> {
    CURRENT_AGENT_TOOL_CANCELLATION
        .try_with(|cancellation| cancellation.clone())
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
    async fn tool_cancellation_is_sticky_and_visible_inside_scope() {
        let cancellation = AgentToolCancellation::default();
        cancellation.cancel();

        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            cancellation.cancelled(),
        )
        .await
        .expect("a cancellation signalled before waiting must remain observable");
        let current = with_agent_tool_cancellation(Some(cancellation.clone()), async {
            current_agent_tool_cancellation()
        })
        .await
        .expect("scoped cancellation");
        assert!(current.is_cancelled());
        assert!(current_agent_tool_cancellation().is_none());
    }

    #[tokio::test]
    async fn tool_context_reads_inside_scope() {
        let ctx = AgentToolContext {
            request_id: "req-1".to_string(),
            tool_call_record_id: None,
            tool_name: "pentest_run".to_string(),
            source: ToolSource::Main,
            operation_id: Some(uuid::Uuid::new_v4()),
            stage_execution_id: None,
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
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
            tool_call_record_id: None,
            tool_name: "run_command".to_string(),
            source: ToolSource::Main,
            operation_id: None,
            stage_execution_id: None,
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };
        let inner = AgentToolContext {
            request_id: "inner".to_string(),
            tool_call_record_id: None,
            tool_name: "pentest_run".to_string(),
            source: ToolSource::SubAgent {
                agent_id: "recon".to_string(),
                agent_name: "Recon".to_string(),
            },
            operation_id: Some(uuid::Uuid::new_v4()),
            stage_execution_id: None,
            stage_run_unit_id: None,
            organization_id: Some(uuid::Uuid::nil()),
            worker_lease: None,
            candidate_attempt: None,
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
            tool_call_record_id: None,
            tool_name: "browser_collect_js_api".to_string(),
            source: ToolSource::Main,
            operation_id: None,
            stage_execution_id: None,
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
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

    #[tokio::test]
    async fn trusted_tool_context_nested_scopes_do_not_cross_stage_or_worker_identity() {
        let outer_unit = uuid::Uuid::from_u128(0x101);
        let inner_unit = uuid::Uuid::from_u128(0x201);
        let outer = AgentToolContext {
            request_id: "outer-request".to_string(),
            tool_call_record_id: Some(uuid::Uuid::from_u128(0x102)),
            tool_name: "outer-tool".to_string(),
            source: ToolSource::Main,
            operation_id: Some(uuid::Uuid::from_u128(0x103)),
            stage_execution_id: Some(uuid::Uuid::from_u128(0x104)),
            stage_run_unit_id: Some(outer_unit),
            organization_id: Some(uuid::Uuid::from_u128(0x105)),
            worker_lease: Some(WorkerLeaseContext {
                worker_run_id: uuid::Uuid::from_u128(0x106),
                stage_run_unit_id: outer_unit,
                lease_token: uuid::Uuid::from_u128(0x107),
                attempt_epoch: 1,
            }),
            candidate_attempt: None,
        };
        let inner = AgentToolContext {
            request_id: "inner-request".to_string(),
            tool_call_record_id: Some(uuid::Uuid::from_u128(0x202)),
            tool_name: "inner-tool".to_string(),
            source: ToolSource::SubAgent {
                agent_id: "worker".to_string(),
                agent_name: "Worker".to_string(),
            },
            operation_id: Some(uuid::Uuid::from_u128(0x203)),
            stage_execution_id: Some(uuid::Uuid::from_u128(0x204)),
            stage_run_unit_id: Some(inner_unit),
            organization_id: Some(uuid::Uuid::from_u128(0x205)),
            worker_lease: Some(WorkerLeaseContext {
                worker_run_id: uuid::Uuid::from_u128(0x206),
                stage_run_unit_id: inner_unit,
                lease_token: uuid::Uuid::from_u128(0x207),
                attempt_epoch: 2,
            }),
            candidate_attempt: None,
        };

        let observed = with_agent_tool_context(Some(outer.clone()), async {
            let nested = with_agent_tool_context(Some(inner.clone()), async {
                current_agent_tool_context()
            })
            .await;
            (nested, current_agent_tool_context())
        })
        .await;

        assert_eq!(observed.0, Some(inner));
        assert_eq!(observed.1, Some(outer));
        assert_eq!(current_agent_tool_context(), None);
    }
}
