# Sub-agent exact-chain context budget

## Status

Implemented; scoped verification pending local build-space recovery.

## Problem

An Enumeration worker resumed an exact durable chain containing 69 messages and
about 3.5 MiB of provider-visible history. Two old
`RESUME REPAIR DIRECTIVE` user messages each embedded all 1,176 coverage-gap
actions, while several worklist and batch tool results were 270+ KiB. The
sub-agent executor has no `ContextManager`; normal tool-result projection only
applies when a new tool finishes, so it cannot repair an oversized chain before
the first resumed provider request.

When that provider request failed for context length, the next exact resume
appended another repair/task context in memory and retried the same oversized
body. The provider therefore rejected the retry before any ordinary compaction
seam could run.

## Invariants

1. Exact/latest restore validates tool-call/tool-result pairs, then compacts the
   provider-visible history before the first model request.
2. A changed restored body is durably rewritten before model I/O. A context
   failure therefore retries from the already-compacted base rather than the
   old oversized row.
3. Every model iteration enforces the same history ceiling, so new tool results
   created inside one long worker segment cannot accumulate without bound.
4. Tool call ids, tool result call ids, and their immediate adjacency are never
   rewritten or split.
5. Historical duplicate repair directives collapse to the newest directive.
   That directive is a bounded projection and tells the worker to refresh the
   authoritative paged worklist instead of replaying the full gap list.
6. Known worklist/browser/JS/route/submit results retain counts, exact target
   identity, terminal/retry/checkpoint state, and bounded samples. Raw evidence
   remains in transcript/DB, not the provider replay.
7. Per-result projections are capped at 32 KiB and total replay history at
   512 KiB. If necessary, the oldest complete conversation units are omitted;
   an assistant tool-call turn and its immediate result move as one atomic unit.
8. Final chain persistence applies the same deterministic compactor. Replaying
   an already-compacted body is byte-stable and does not rewrite the row.
9. A provider rejection that explicitly identifies input-context overflow is a
   typed `ProviderContextLimitExceeded` failure. Runtime exposes the stable
   `sub_agent_provider_context_limit_exceeded` / `context_limit` contract, and
   `stage_run` never retries it inside the same request. Generic HTTP 400 and
   token-per-minute rate limits are not classified as context overflow.

## Failure policy

Malformed or unpaired histories still fail closed under the existing typed
exact/latest restore contract. A failure to durably rewrite a compacted restore
also fails restore before provider or tool execution; it cannot silently replay
the oversized body or create a fresh worker.

The compactor never treats retained prose as authoritative completion. When old
turns are omitted, its synthetic note explicitly directs the worker back to
`stage_worklist_status` / `stage_worklist_next` and DB/evidence truth.

If provider preflight still rejects a compacted request for context length, the
executor returns the typed error before final chain persistence. This prevents
the newly appended task/repair turn from being committed and prevents
`stage_run` from growing the same rejected request through an automatic retry.

## Verification

- RED fixture: 200 verbose worklist cells plus two 1,176-action repair
  directives produces a multi-megabyte, pair-valid exact chain.
- Restore must durably rewrite that chain below the configured budget while
  preserving worklist counts/next action and the newest repair directive.
- A second exact resume of the rewritten value must be byte-identical and make
  no database update.
- Scoped `golish-sub-agents` tests and clippy cover restore, persistence, and
  per-iteration preflight behavior.
- Classifier tests cover stream-start and SSE error strings (including the
  DeepSeek `Request body has ... tokens ... limit` HTTP 400 shape) plus false
  positives; runtime contract and stage-run policy tests pin the non-retryable
  mapping.
