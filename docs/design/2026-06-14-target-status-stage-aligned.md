# Stage-aligned target status (per-target resume/skip signal)

> 2026-06-14. Replaces the coarse `target_status` (`new/recon/recon_done/scanning/tested`)
> with a stage-aligned lifecycle so each target's badge reflects which pentest stage it
> has reached — and, more importantly, so an AI agent picking up a task can **skip targets
> that already passed a given stage** (per-target resume / no re-scan).

## Problem

`target_status` had one coarse "Recon Done" state. The user wants finer, stage-aligned
statuses (e.g. passive vs active scan) for two reasons:

1. **Display**: a target's badge should show how far it got in the real pentest pipeline.
2. **Machine-actionable (the real driver)**: the status is the AI's memory of "how far each
   target went". Before running stage *S* on a target, the agent reads the target's status
   and **skips** it when `status >= S`. This mirrors the engine's existing org-level resume
   oracle (`scheduler::OrgCompletionOracle::is_already_complete(org, stage)`), pushed down
   to the target level.

## Decision

New `TargetStatus` = furthest completed stage, aligned to the harness pipeline
(`StageKind` + recon sub-stages `PassiveInternet`/`ActiveCollection`):

| order | value (DB / wire / as_str) | label | maps from old |
|---|---|---|---|
| 0 | `new` | New | `new` |
| 1 | `passive` | Passive | `recon` |
| 2 | `active` | Active | `recon_done` |
| 3 | `enumerated` | Enumerated | *(new)* |
| 4 | `vuln_scan` | Vuln Scan | `scanning` |
| 5 | `verified` | Verified | `tested` |

- **One representation everywhere**: switch the serde derive from `rename_all = "lowercase"`
  to `rename_all = "snake_case"` so the serde/ts-rs wire form == the DB enum value ==
  `as_str()`. (Old code had a footgun: `ReconDone` serialized to `recondone` while the DB
  value was `recon_done`.) Only `vuln_scan` is multi-word; the rest are identical in both.
- **Who advances it (phase 1)**: the AI, via `manage_targets(action:"update_status")` after
  finishing a stage on a target, and reads it via `manage_targets(action:"list")` to skip.
  Auto-advance from the recon/scan flows is a phase-2 follow-up (not in this change).
- **`from_str` accepts legacy aliases** (`recon→passive`, `recon_done→active`,
  `scanning→vuln_scan`, `tested→verified`) so any lingering old string maps correctly.

## Impact surface

- **DB**: new migration `20260614000001_target_status_stage_aligned.sql`. Postgres enum, so
  text round-trip (drop default → column to `text` → remap values → drop+recreate type →
  column back to enum → restore default). Only `targets.status` uses the type (verified: no
  other migration references `target_status`), so the recreate is safe. Forward-only; never
  edits the old migration (invariant I10).
- **Rust**: `golish-app-core::domain::targets::TargetStatus` (variants + serde + as_str +
  from_str). Consumers (`ports/recon/targets.rs`, `recon-app/targets/*`) only call
  `from_str`/`as_str`, no exhaustive external matches.
- **Tool**: `manage_targets` status enum + lifecycle description + skip guidance (the AI's
  authoritative contract).
- **Frontend**: regenerated `lib/generated/TargetStatus.ts`; exhaustive
  `TargetTreeRow.STATUS_CONFIG` badge map (typecheck-enforced); `buildTopologyModel` status
  literals.
- queries are unchecked `sqlx::query(...)` with `$1::target_status` / `status::text`, and
  there is no `.sqlx` offline cache, so the enum change needs no live DB at compile time.

## Verification

`cargo nextest -p golish-db -p golish-recon-app -p golish-pentest-app` + `clippy -D warnings`
on touched crates + `just gen-types` (+ check-types drift) + `just check-fe` + `just test-fe`.
Migration itself is exercised at app boot (embedded PG; user env) — written as a standard,
fully-transactional DDL round-trip.

## Rollback

Revert the migration file is not enough once applied (it mutates the enum). To roll back,
add a reverse migration mapping the new values back. The Rust/TS/tool changes are plain
reverts. Git history retains the prior coarse enum.
