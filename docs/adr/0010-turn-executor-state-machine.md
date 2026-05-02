# ADR-0010: TurnExecutor — Agentic Loop as an Explicit State Machine

## Status

**Proposed** (2026-05-02). Implementation in progress — PoC of the
`phase_pre_flight` step has landed as a first proof that the pattern
composes with existing shared state. Remaining phases will migrate in
subsequent PRs, each introducing exactly one phase handler.

## Context

`backend/crates/golish-agent-runtime/src/agentic_loop/mod.rs` hosts
`run_agentic_loop_unified` — the 300-line function that drives every
agent turn. Although each step already delegates to a helper module
(`stream_processor`, `tool_dispatch`, `reflector`, `compaction_loop`,
etc.), the **orchestration order is hard-coded in the main function**.
Symptoms:

1. **Untestable in isolation.** The loop body mutates 8+ local
   variables (`iteration`, `accumulated_response`, `accumulated_thinking`,
   `total_usage`, `consecutive_no_tool_turns`, `total_reflector_nudges`,
   `reflector_active`, `chat_history`). Any phase-level test has to
   reconstruct this whole context.
2. **Policy tangling.** Compaction, HITL, and the reflector nudge are
   each conditional branches inlined in the orchestration — there is
   no first-class concept of a "turn interceptor" or "policy".
3. **Hard to evolve.** Adding a new phase (e.g. "output validator" or
   "safety classifier") requires editing the 300-line function, which
   is a merge-conflict magnet and blows past the 500-line file budget
   on every feature attempt.

The architecture review (`.cursor/rules/architecture-evaluation.mdc`)
flagged this as one of the **five critical issues** (Problem 4: "God
function of 300 lines").

## Decision

Refactor `run_agentic_loop_unified` into an **explicit state machine**
with three collaborating types:

```rust
// 1. Mutable state owned by one turn.
pub struct TurnState { … }

// 2. Enum describing where we are in the turn.
pub enum TurnPhase { … }

// 3. Trait for phase handlers.
#[async_trait]
pub trait TurnPhaseHandler { … }
```

Each **phase** is a small unit that advances `TurnState` and returns a
`PhaseOutcome` enum:

```rust
pub enum PhaseOutcome {
    /// Continue to the next phase in the current iteration.
    Continue,
    /// Skip to the next iteration (e.g. reflector injected nudge).
    NextIteration,
    /// Terminate the loop (max iterations, cancellation, break).
    Break(BreakReason),
    /// Unrecoverable error.
    Fail(anyhow::Error),
}
```

The loop body in `mod.rs` becomes a small scheduler:

```rust
for phase in phase_order() {
    match phase.run(&mut state, &ctx, &config).await {
        Continue => continue_phase_list,
        NextIteration => continue 'iter,
        Break(r) => break 'iter,
        Fail(e) => return Err(e),
    }
}
```

### Phase inventory

Extracted from the current 300-line function, there are **10 phases**:

| # | Phase | Triggers | Current code ref |
|---|---|---|---|
| 1 | `PreFlight` | Every iteration | `MAX_TOOL_ITERATIONS` check + `cancelled` flag + reset compaction state |
| 2 | `CompactionPre` | Iteration == 1 | `pre_turn_compaction` |
| 3 | `CompactionInter` | Iteration > 1 | `inter_turn_compaction` |
| 4 | `FirstIterHooks` | Iteration == 1 && !is_sub_agent | `run_first_iteration_hooks` |
| 5 | `TokenEstimate` | Every iteration | Proactive pre-call token counter |
| 6 | `StreamStart` | Every iteration | `start_completion_stream` |
| 7 | `StreamProcess` | Every iteration | `process_stream` → StreamProcessOutcome |
| 8 | `AssistantPush` | Every iteration | `push_assistant_message` |
| 9 | `ReflectorOrBreak` | No tool calls | `maybe_run_reflector` → Injected/Skipped |
| 10 | `ToolDispatch` | Has permitted tools | Allow-list filter + `dispatch_tool_calls` |

Before the loop: one-time **Setup** (span creation, hook registry,
tool list, context manager sync). After the loop: **Finalization**
(`record_turn_completion`, `record_final_output_and_usage`).

### Interceptor hook

A separate `TurnInterceptor` trait fires **around** each phase:

```rust
#[async_trait]
pub trait TurnInterceptor {
    async fn before(&self, phase: &TurnPhase, state: &TurnState, ctx: &AgenticLoopContext<'_>);
    async fn after(&self, phase: &TurnPhase, state: &TurnState, outcome: &PhaseOutcome);
}
```

Use cases:
- **Langfuse span maintenance** (currently inline in mod.rs)
- **HITL approval recording** (currently coupled with ToolDispatch)
- **Custom logging** for evals

Interceptors are registered alongside phase handlers in
`TurnExecutor::new()`.

## Migration plan (incremental PRs)

Each milestone is a separate PR. `cargo check --workspace --tests`
must remain green throughout.

| PR | Milestone |
|---|---|
| **C1-0** | This ADR + empty `turn/` module with `TurnState`, `TurnPhase`, `PhaseOutcome` skeleton. No behavior change. |
| **C1-1** | Extract **`PreFlight`** phase (pre-iteration guards). Most independent; zero shared-state mutation aside from `state.iteration`. **This PR is the PoC** that proves the pattern composes. |
| **C1-2** | Extract **`CompactionPre` + `CompactionInter`** phases. |
| **C1-3** | Extract **`FirstIterHooks` + `TokenEstimate`** phases. |
| **C1-4** | Extract **`StreamStart` + `StreamProcess`** phases. |
| **C1-5** | Extract **`AssistantPush` + `ReflectorOrBreak` + `ToolDispatch`**. |
| **C1-6** | Replace the main `loop { … }` body with the phase scheduler. mod.rs shrinks to ≤150 LOC. |
| **C1-7** | Introduce `TurnInterceptor` trait and migrate Langfuse span management + HITL recording out of phases. |
| **C1-8** | Add per-phase unit tests with mock `TurnState` (target: 1+ test per phase). |

**Estimated effort**: 3–5 days of focused work, spread across 8 PRs.
Each PR is ≤500 LOC diff.

## Alternatives considered

### A. Leave as-is

Rejected. The 300-line function already blocked one large feature PR
(extended thinking history) which had to inline yet another branch.
The trend is unsustainable.

### B. Stackful actor per phase

Use `tokio::task::spawn` + message channels per phase. Rejected: the
loop is inherently sequential, actors add latency and make
backtrace/profiling harder. The phases are already async so coupling
them via `async fn` is ergonomic enough.

### C. Full rewrite with a BPMN-style DSL

Some multi-agent frameworks (graph-flow crate, already a workspace
dependency) offer a declarative workflow DSL. Rejected for this turn
loop because:
- The loop is tightly coupled to rig-core's streaming iterator API.
- BPMN-style graphs shine for multi-agent orchestration (already
  handled by `task_orchestrator`), not for a single turn's internal
  phases.

## Consequences

**Benefits**
- Each phase becomes unit-testable in isolation (inject mock
  `TurnState` and `AgenticLoopContext` collaborators).
- `mod.rs` shrinks from 495 to ~150 LOC — back within the 500-line
  budget enforced by `arch-check` CI.
- New phases (output validator, safety classifier) add as a new
  `impl TurnPhaseHandler` without touching the main function.
- Reflector / Compaction / HITL become named, discoverable concepts
  rather than inline if-branches.

**Costs**
- New `TurnState` struct is inherently a shared-mutable-state bag. It
  will be large (8+ fields), but now **explicit** rather than hidden
  in local variables of a 300-line function.
- Adds 8 PRs of incremental migration — requires discipline to land
  them back-to-back rather than leaving the codebase half-migrated.
- Tests and docs must be updated with every PR.

**Risks**
- Borrow-checker friction: sharing `&mut TurnState` across async
  boundaries needs care (will likely require explicit `.await` points
  between phases, which matches current structure).
- Langfuse span lifetimes currently span the whole loop — moving into
  interceptors must preserve parent/child relationships. Verify with a
  trace snapshot test in C1-7.

## References

- `.cursor/rules/architecture-evaluation.mdc` — Problem 4 (300-line
  god function).
- `.cursor/rules/refactor-execution.mdc` — C1 entry in the roadmap.
- `backend/crates/golish-agent-runtime/src/agentic_loop/mod.rs` —
  current implementation.
- [ADR-0003](./0003-graph-flow-for-multi-agent.md) — why `graph-flow`
  was chosen for *multi*-agent orchestration (different problem).
