# Stage Team scope-bounded worker admission implementation plan

**Goal:** Remove the fixed lifetime child/WorkerRun quota from Company Controller stages while keeping
live concurrency, exact scope, replay, per-WorkItem retry, evidence, and Gate boundaries intact.

**Architecture:** `coordination_mode=company_controller` selects scope/epoch admission. Existing
StageSpec/TeamPlan lifetime values remain byte-identical for restart replay, but are ignored by Company
Controller admission/claim/retry/runtime loops. UI reports live K only.

**Constraints:** Do not run `init.sh`; do not change schema/migrations/generated IPC; do not perform a
real scan or external provider request. Use focused RED→GREEN tests and run `just space-guard` before
Cargo commands.

## Task 1: Lock the regression with RED tests

- Keep StageSpec/scheduler replay-shape tests unchanged so an active operation can reseed exactly.
- Add a golish-db integration test where a real Company Controller accepts more requests than both the
  historical `max_requests` and compatibility `max_workers_total` values.
- Add a DB integration test proving K still blocks a third live child, then allows it after another
  child terminalizes even though the historical WorkerRun total has been reached.
- Add a frontend assertion that the plan displays live concurrency without a lifetime total.

## Task 2: Remove Company Controller lifetime admission

- Retain `max_dynamic_requests`, derived `max_workers_total`, and `max_requests` only as frozen
  compatibility metadata; document that they are not Company Controller admission authority.
- In `runtime_memory_tx`, apply request/work-item total checks only to non-Controller legacy plans.
- In claim and retry transactions, apply WorkerRun total checks only to non-Controller legacy plans.
- Replace Company Controller runtime loops that used `max_workers_total` with durable queue/barrier
  termination.

## Task 3: Converge UI and documentation

- Change StageTeamRunView from `active / total` to an active-concurrency label.
- Update golish-agent-kit harness, golish-agent-runtime agent loop, golish-db/repo, frontend components,
  and module INDEX contracts.
- Record focused RED/GREEN evidence in `feature_list.json` and `agent-progress.md`.

## Verification

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -p golish-agent-runtime -p golish-db \
  -E 'test(company_controller) | test(stage_team_scope_bounded)' --status-level fail
pnpm exec vitest run frontend/components/Engagement/StageTeamRunView.test.tsx
cd backend && cargo check -p golish-agent-kit -p golish-agent-runtime -p golish-db
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-runtime -p golish-db --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
pnpm typecheck
pnpm exec biome check frontend/components/Engagement/StageTeamRunView.tsx frontend/components/Engagement/StageTeamRunView.test.tsx
jq empty feature_list.json && git diff --check
```
