# Enumeration planned terminal exceptions implementation plan

> Superseded by `2026-07-10-enumeration-trusted-transport-preflight.md`.

## Scope

Close the Enumeration pre-submit deadlock without changing DB schema, outcome
writers, or final gate semantics.

## Steps

1. Add the bounded `terminal_exceptions` JSON schema to
   `check_stage_asset_coverage`, `stage_worklist_status`, and
   `stage_worklist_next`.
2. Add one pure validator/projector in `golish-agent-kit` that atomically checks
   Enumeration stage, exact current snapshot cell, registered technique,
   pending-only source state, allowed status, concrete note, and duplicate
   conflicts.
3. Apply the projected snapshot to all three read tools and return explicit
   accepted counts plus a preview-only/non-persistent contract. Ensure worklist
   pagination omits planned terminal cells and readiness counts them as terminal;
   retain the 200-cell cap while adding a 50-distinct-origin cap and root counts.
4. Update the Enumerator prompt and Enumeration methodology: carry the complete
   planned exception array on every preview call and submit the same coverage
   only after the newest preview reports `ready_to_submit=true`.
5. Update affected module cards and focused tests.

## Verification

```bash
cd backend && cargo fmt --all -- --check
cd backend && cargo nextest run -p golish-agent-kit tool_executors
cd backend && cargo nextest run -p golish-sub-agents defaults
cd backend && cargo nextest run -p golish-tools definitions
cd backend && cargo check -p golish-agent-kit -p golish-sub-agents -p golish-tools
cd backend && cargo clippy -p golish-agent-kit -p golish-sub-agents -p golish-tools --all-targets -- -D warnings
git diff --check
```

Required branches include valid blocked/not-applicable previews, non-empty note,
unregistered/unknown/foreign cell rejection, duplicate rejection, current
`error`/`partial` precedence, and pagination advancing past planned cells.
