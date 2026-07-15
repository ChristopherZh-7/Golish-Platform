# Stage Team downstream Company Controller convergence implementation plan

**Goal:** make EAS, Enumeration and Vuln use the same durable Company Controller, local `update_plan`, child
WorkItem and DB-backed frontend contract as Target Intel without changing Candidate/Verification semantics.

## Task 1: Lock the downstream stage contract with RED tests

- Extend StageSpec tests to require bounded Company Controller policies on all four company-scoped stages.
- Extend runtime tests to require frozen specialist routing for Controller and allowed child roles, including
  cross-stage fail-closed cases.
- Extend frontend routing coverage with an EAS exact Team pointer that must suppress the legacy specialist card.

## Task 2: Enable downstream Team policies

- Add server-owned `team_scheduler` policies to EAS, Enumeration and Vuln specs.
- Keep stage-specific role, request-kind, concurrency and risk-lane allowlists explicit.
- Keep Candidate/Verification specs without the general Team policy.

## Task 3: Generalize executor routing safely

- Resolve `company_stage_controller` through the durable Unit's frozen specialist.
- Accept only the exact stage specialist as a downstream child role; keep Intel's provider/critic aliases bound
  to Recon only.
- Thread the frozen mapping through Controller, child and final turns; reject mismatches before provider use.

## Task 4: Focused verification

- Run `just space-guard` before every Cargo command.
- Run StageSpec, Stage Team scheduler/Controller, sub-agent tool fence and frontend Stage Team tests.
- Run scoped Clippy, rustfmt, Biome/typecheck, JSON validation and diff checks.

## Task 5: Isolated CLI closure

- Build the current CLI and run a fresh ephemeral-DB slice against the local HTTP fixture through `vuln_triage`.
- Inspect `transcript.json`, `run.log`, `scripts/run_tree.py --full --db`, CLI DB summary and exit code.
- Iterate until each company-scoped stage reports the Controller scheduler and deterministic terminal truth.

## Task 6: Record evidence

- Update affected module cards, module index, feature evidence and `agent-progress.md`.
- Keep the parent feature `in_progress` unless its broader Candidate/Verification and migration DoD is also met.

## Execution status (2026-07-16)

- Tasks 1-3 completed with RED-to-GREEN StageSpec, frozen specialist, stage admission, final materialization,
  Controller heartbeat, telemetry enum, repair-budget, anonymous eligible-set, and frontend routing tests.
- Task 4 completed with focused Rust/frontend suites, scoped Clippy, rustfmt, TypeScript, Biome, JSON and diff
  checks; full-repository `just precommit` is recorded separately because the parent feature spans a large
  shared dirty tree.
- Task 5 completed on a localhost-only fixture. Final session
  `stage-run-2cebfd1b-87cf-4863-97b6-df263032aead` exited 0 with EAS, Enumeration and Vuln PASS and exact
  `company_controller_v1` DB truth.
- Task 6 completed for this downstream slice. The parent Candidate-to-Verification feature remains
  `in_progress`; this slice does not claim its wider recovery/migration DoD.
