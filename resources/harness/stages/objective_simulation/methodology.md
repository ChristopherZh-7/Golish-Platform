# Objective Simulation methodology (C6 / P6b)

## Contract

Objective Simulation may prepare a closed, typed side-effect action, but a
prepared plan is not proof that the action ran. Preparation and execution are
separate durable phases.

## P6b execution boundary

- `post_exploit_execute_action(mode=prepare)` atomically writes the action, one exact cleanup obligation, both evidence sets, and `PostExploitActionPrepared.v1`; it returns `approval_required` and performs no external mutation.
- `mode=execute` accepts only persisted action/approval IDs and an exact approval row version. It reloads plan hash, frozen scope, active principal, obligation, expiry, and CAS state; model text cannot approve or replace the plan.
- External execution occurs only after the DB fencing transaction commits. Transport ambiguity becomes `recovery_required`/outcome unknown and is never automatically retried.
- Production is fail-closed (`post_exploit_executor_unavailable`) until a closed typed executor is installed; raw shell, URL, credential, or exploit recipe inputs are absent from the schema.

## Canonical result

Prepared side-effect authority is `post_exploit_actions + cleanup_obligations +
post_exploit_approvals + evidence + knowledge_outbox_events`. A pending approval
or prepared action must never be rendered as executed. Side-effect-free
ObjectiveAttempt rows retain the original C4 simulation semantics.
