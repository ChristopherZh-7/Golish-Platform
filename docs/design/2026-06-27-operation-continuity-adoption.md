# Operation Continuity Adoption

## Problem

A user can abandon chat session A while keeping the embedded DB. When they start
chat session B for the same engagement, Golish should not blindly restart from
Scoping or ignore durable facts already in the database. It also must not
silently reuse old evidence, because scope, freshness, and user intent may have
changed.

Recent gate failures exposed the gap: stage workers had already produced
DB-backed progress and per-org pass tokens, but the new/final session context
did not have the same source-query rows, so the harness behaved as if the stage
was unfinished.

## Design

Continuity is split into two distinct flows:

1. Resume: same operation, same checkpoint, same chat session lineage. Existing
   `latest_resumable_by_session` and graph-flow checkpoints handle this.
2. Adopt: new operation in a different or fresh chat session reuses durable DB
   facts from older runs after the user confirms reuse.

Adopt is not a prompt convention. The backend builds a deterministic
`ContinuitySnapshot`, asks the user when reusable progress exists, and only after
confirmation applies a `ContinuityAdoptionPlan`.

## Snapshot Rules

The first implementation is intentionally conservative and schema-free:

- Scoping is reusable only as "existing in-scope organizations are present";
  user confirmation is still required before adopting it.
- Fan-out stages use `org_stage_completions` freshness across the in-scope org
  axis.
- A stage is reusable only when all required orgs have fresh completion rows.
- Partial, stale, conflict, or missing rows stop the cursor at that stage.
- The plan follows the same DAG branch rule as live execution, so a reusable
  no-progress stage can bail to Reporting instead of running downstream stages.

## Runtime Behavior

When Task/Profile mode is about to start a fresh operation:

- If the same chat session has a resumable checkpoint, resume wins.
- If older DB-backed progress exists and the user has not chosen a continuity
  strategy, Golish asks whether to reuse or start fresh and does not create a
  new operation yet.
- If the user chooses reuse, the orchestrator starts at the first non-reusable
  stage and restricts the projected DAG to the remaining stages.
- If the user chooses start fresh, older DB progress is ignored for cursor
  seeding.

The adoption plan is written into `operation_state.state_blob` with the initial
resume snapshot so later debugging can see which stages were skipped by
confirmation.

## Deferred Work

- Target matching should narrow "existing DB progress" to the requested
  engagement root instead of relying on the legacy in-scope org axis.
- A frontend continuity card should replace the plain text confirmation.
- Future DB support can record explicit adoption audit rows. This pass avoids a
  migration and stores only the chosen plan inside operation state.
