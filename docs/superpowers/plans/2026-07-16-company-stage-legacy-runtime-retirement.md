# Company-scoped stage legacy runtime retirement implementation plan

**Goal:** remove the old company-stage specialist scheduler and frontend card after an attested Runtime
Memory V2 cutover, without changing Candidate/Verification or weakening recovery authority.

## Task 1: Prove the fork and freeze the checkpoint

- Confirm the four StageSpecs declare Company Controller.
- Prove runtime Team dispatch is currently gated by `V2Only` and DB mutations reject other contracts.
- Commit the complete pre-deletion worktree after full precommit.

Evidence: checkpoint `5af4f31a`; full precommit printed `OK` before commit.

## Task 2: Add RED retirement tests

- Runtime selection tests: all four company stages must select Company Controller or typed rerun-required;
  none may select the generic specialist loop.
- Frontend test: rows without an exact Team pointer show rerun-required and never `Main Agent`/specialist
  cards or drill-in actions.
- Rollout tests: promotion remains adjacent and attested; frozen historical operation contracts do not
  mutate.

Status: complete. Runtime route/rerun invariants, completed replay, Team resume authority and frontend
missing/mixed pointer tests are present.

## Task 3: Verify the V2 cutover authority

- Use the existing cohort gate/promotion receipt contract; do not bypass triggers.
- If a forward migration is needed for clean installations, add a new migration rather than editing an
  applied file, and keep existing operations immutable.
- Stop if the current cohort is not ready; report its exact deterministic reason.

Status: complete without mutation. The current deployment singleton was already `v2_only` (rank 3,
row version 3), so no schema, migration, rollout row or frozen operation contract changed.

## Task 4: Delete the old company-stage runtime and UI

- Make Company Controller selection mandatory for Target Intel/EAS/Enumeration/Vuln.
- Add a final guard before the generic per-org loop.
- Remove legacy `StageRunOrgRows` card rendering and its tests/helpers/imports.
- Preserve Candidate/Verification and later typed schedulers.

Status: complete. Historical/non-V2 company-stage operations are typed rerun-required; the generic
specialist loop has a defense-in-depth company-stage invariant, and the old frontend collector cards are
removed.

## Task 5: Verify and commit

- Run focused Runtime/DB/frontend tests, scoped Clippy/rustfmt/TypeScript/Biome and source-retirement checks.
- Run full `just precommit`.
- Build the CLI and run a localhost-only EAS -> Enumeration -> Vuln acceptance; inspect run tree and DB.
- Update module cards, INDEX, feature/progress evidence, then commit the retirement change.

Status: complete. Localhost CLI acceptance exited 0 and the final full precommit printed `OK`; this
documented closeout is included in the retirement commit.
