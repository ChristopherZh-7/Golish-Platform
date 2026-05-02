//! Turn interceptors — cross-cutting concerns around phase scheduling.
//!
//! ADR-0010 (C1-7) introduces the `TurnInterceptor` trait so that
//! observability, HITL recording, eval logging, and other concerns
//! that span *all* phases live in one named place rather than being
//! re-implemented inline in `turn::executor` and the phase modules.
//!
//! Today only one interceptor exists, [`LangfuseInterceptor`], which
//! finalises Langfuse trace fields once the loop terminates. The
//! span *creation* still happens inside `turn::executor::run_turn_loop`
//! because phases (pre_flight, completion) depend on the spans being
//! threaded into their signatures — pulling that into an interceptor
//! would change every phase signature and is intentionally out of
//! C1-7's scope. C1-8 (per-phase tests) ships first; later PRs may
//! widen the trait surface as patterns emerge.
//!
//! ## Why a trait, not a free function?
//!
//! The trait makes it explicit that "things that observe a turn from
//! the outside" are a category, and gives future contributors a stable
//! seam to add new observers (HITL recorder, eval timing, custom
//! dashboards). `LangfuseInterceptor` is currently the only impl, but
//! the loop can be retrofitted to iterate over a registry of
//! `Box<dyn TurnInterceptor>` without touching phase code.

use async_trait::async_trait;
use tracing::Span;

use golish_context::token_budget::TokenUsage;

use super::super::context::AgenticLoopContext;
use super::super::unified_helpers::record_final_output_and_usage;

/// Observability spans owned by one agentic-loop invocation.
///
/// `chat_message_span` is the outer Langfuse trace; `agent_span` is
/// the root agent observation, child of `chat_message_span`. Both are
/// shared with phases (read-only) so iteration-level spans
/// (`llm_completion`, `tool_call`) can be created as their children.
pub struct TurnSpans {
    /// Outer Langfuse trace span — root of the entire conversation.
    pub chat_message_span: Span,
    /// Root agent observation, parent of every per-iteration span.
    pub agent_span: Span,
}

/// Cross-cutting hook around the agentic loop.
///
/// Implementations carry no per-turn state today; all per-turn
/// information arrives through the method parameters. If a future
/// interceptor needs state, store it on the `impl` itself (the
/// executor holds the interceptor by value / `Box<dyn>`).
#[async_trait]
pub trait TurnInterceptor: Send + Sync {
    /// Called once after the loop terminates (success or graceful
    /// `break`) with the accumulated response, total token usage, and
    /// the spans that were active during the loop. Used to write
    /// trailing fields onto observability spans, persist eval data,
    /// etc.
    async fn after_turn(
        &self,
        ctx: &AgenticLoopContext<'_>,
        spans: &TurnSpans,
        accumulated_response: &str,
        total_usage: &TokenUsage,
    );
}

/// Default interceptor: writes the final response + token usage onto
/// the `chat_message` (trace) and `agent` (root observation) spans so
/// Langfuse renders complete output / token-usage fields.
///
/// Stateless — instantiate with `LangfuseInterceptor` directly.
pub struct LangfuseInterceptor;

#[async_trait]
impl TurnInterceptor for LangfuseInterceptor {
    async fn after_turn(
        &self,
        ctx: &AgenticLoopContext<'_>,
        spans: &TurnSpans,
        accumulated_response: &str,
        total_usage: &TokenUsage,
    ) {
        record_final_output_and_usage(
            ctx,
            accumulated_response,
            total_usage,
            &spans.chat_message_span,
            &spans.agent_span,
        );
    }
}
