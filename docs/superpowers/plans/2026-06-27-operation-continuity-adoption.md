# Operation Continuity Adoption Plan

## Goal

When a fresh Task/Profile session sees reusable DB-backed progress from an older
session, ask the user whether to adopt it. If confirmed, start the new operation
from the first unsatisfied stage instead of replaying already satisfied stages.

## Steps

1. Add an IO-free continuity model in `golish-agent-kit::harness`.
   - Represent stage reuse statuses.
   - Compute the adoption cursor by replaying reusable prefix stages through the
     profile-projected DAG.
   - Preserve live branch semantics for no-progress stages.

2. Add a DB-backed continuity preflight in `task_orchestrator`.
   - Read in-scope orgs through `DbRepoProvider`.
   - Read `org_stage_completions` for fan-out stages.
   - Convert completion freshness into a `ContinuitySnapshot`.

3. Wire adoption into `TaskOrchestrator`.
   - Accept a user-confirmed `ContinuityAdoptionPlan`.
   - Start at the plan entry stage.
   - Set the remaining-stage allowlist so the metalcraft executor starts at the
     same stage.
   - Persist the plan inside the initial resume state.

4. Wire the Task/Profile entry.
   - Extend `start_operation` with `continuity_decision`.
   - Default to `ask_before_reuse`.
   - If reusable progress exists and the user has not chosen, return a
     confirmation prompt without creating a task.
   - If the user chooses reuse, apply the adoption plan.

5. Validate with focused Rust tests.
   - Pure DAG cursor tests.
   - Completion freshness classification tests.
   - Start-operation payload and prompt parsing tests.

## Non-Goals

- No DB migration in this pass.
- No frontend continuity card yet.
- No fuzzy target/entity matching beyond the existing in-scope org axis.
