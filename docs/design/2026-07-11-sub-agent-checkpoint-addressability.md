# Sub-agent checkpoint addressability

## Status

Approved as the failure-path complement to `2026-07-11-sub-agent-history-tool-pair-durability.md`.

## Problem

Atomic body checkpoints preserve a completed provider tool batch if the worker is later cancelled, hits a provider error, or is dropped by the outer wall-clock timeout. A fresh chain can nevertheless become unreachable: the current success path exposes its UUID only through a response-tail marker, while graceful failure and outer timeout return before that marker is appended. The database then contains a valid chain body that `stage_run` cannot bind to the org worker checkpoint.

The durable contract therefore needs two separate guarantees:

1. a provider-valid body exists; and
2. upper layers receive the UUID of that successfully checkpointed body, including failure paths.

## Decision

### 1. Structured chain identity

`SubAgentResult` gains an optional, serde-defaulted `chain_id`. Existing serialized results without the field remain readable. The textual `[sub_agent_session_id: ...]` marker remains for backward compatibility, but new runtime code uses the structured field first.

### 2. Publish only successfully checkpointed identities

After exact/latest restore or fresh creation, the executor appends the invocation's user prompt and any restored repair directive, then writes an initial provider-valid chain snapshot before the first model request. Only after that update succeeds may it publish the UUID as `checkpointed_chain_id`.

Every completed Assistant tool-call batch plus its complete User ToolResult turn advances the same snapshot. A dangling call, partial multi-call batch, or failed chain update never advances the published identity.

### 3. Outer-timeout handoff

The outer `execute_sub_agent` timeout wrapper and inner executor share a small in-memory slot containing the last successfully checkpointed UUID. Dropping the inner future cannot perform async cleanup, but the wrapper can return the already-durable UUID. This guarantees addressability up to the last completed snapshot; it does not claim the interrupted in-flight model/tool work was persisted.

### 4. Runtime propagation

The direct sub-agent tool result includes `chain_id`. `stage_run` resolves the worker chain from the structured field first and falls back to the legacy marker. Typed provider-context-limit failures also include their existing optional chain UUID in the error JSON, allowing the worker mapping to remain exact even when the failure is non-retryable.

## Invariants

- A returned `chain_id` always names a chain whose body update succeeded.
- Initial snapshot and batch snapshots pass the same compaction and tool-pair validation as final persistence.
- Usage/duration is written only by normal finalization; checkpoints do not duplicate usage accounting.
- Current incomplete tool batches are not fabricated or persisted. DB/worklist truth remains the recovery source for interrupted side effects.
- Hard process death before the database confirms a snapshot cannot be recovered by an async Rust destructor and is outside this guarantee.

## Verification

- Backward-compatible serde test for results without `chain_id`.
- Fresh invocation test proving the initial snapshot occurs before a later stream failure publishes the UUID.
- Outer-timeout test proving the returned UUID matches the last successful snapshot.
- Failure JSON test for typed provider context limits.
- `stage_run` test proving structured UUID precedence with marker fallback.
- Full `golish-sub-agents` and `golish-agent-runtime` suites, clippy, fmt, and final startup smoke.
