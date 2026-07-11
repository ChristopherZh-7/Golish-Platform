# Sub-agent history tool-pair durability

## Status

Implemented and awaiting final live Enumeration validation.

## Problem

The Test1 Enumeration live run proved that the durable worker chain could be
created and resumed across capacity segments, then exposed a second invariant
violation. A worker ended a segment with `submit_result`. The executor wrote the
assistant message containing that tool call, treated the barrier as terminal,
and persisted before adding the matching tool result. The next exact resume was
rejected by the OpenAI-compatible provider with HTTP 400 because every assistant
tool call must be followed immediately by its tool result.

The stream processor also treated an SSE item error as a normal end-of-stream.
That caused each failed retry to append another user objective/repair directive,
persist the still-invalid history, publish a resume marker, and report success.

## Invariants

1. A durable history may contain an assistant tool-call turn only when the next
   user turn contains matching results for every provider call id.
2. `submit_result` is a terminal control barrier, but it still produces a
   synthetic, model-visible result for its original call id.
3. Calls batched after `submit_result` are not executed and each receives an
   explicit skipped result; no assistant call may be left unmatched.
4. Barrier and stage-stall exits append the complete result turn before breaking.
5. Persistence validates the invariant before updating the database; exact and
   latest restore validate it again before replay.
6. Provider stream errors terminate the worker as `success=false`, do not
   dispatch partial calls, and do not persist the current incomplete turn.
7. A clean text-only completion is retained as an assistant message so exact
   continuation does not lose the worker conclusion.
8. Dispatch lifecycle tracking maps `SubAgentResult.success=false` to `failed`,
   not `completed`.

## Failure policy

Semantically invalid legacy histories fail closed through the existing typed
chain contracts (`restore_exact` / `restore_latest`). The executor does not
guess or manufacture results for ordinary tools, because doing so could claim an
external side effect that never happened. A failed restore therefore cannot
fall through to a fresh worker under the same exact-resume request.

## Verification

- Focused tests cover missing and balanced multi-call histories, barrier call-id
  preservation, provider stream error propagation, restore rejection, and
  persistence rejection before a database update.
- Full `golish-sub-agents` and `golish-agent-runtime` suites must pass.
- The final Test1 live run must show a valid exact-chain continuation after a
  barrier/capacity boundary and must reach an authoritative Enumeration PASS.
