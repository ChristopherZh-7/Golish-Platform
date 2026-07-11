# Enumeration planned terminal exceptions

> Superseded by `2026-07-10-enumeration-trusted-transport-preflight.md`.

## Problem

Enumeration preflight (`stage_worklist_status`, `stage_worklist_next`, and
`check_stage_asset_coverage`) currently derives readiness only from the current
DB snapshot. The final deliverable is the only place an Enumerator can declare
an honest `blocked` or `not_applicable` cell. A pending cell that is genuinely
blocked therefore keeps `ready_to_submit=false` forever and is repeatedly
returned by pagination, even though the unchanged gate would accept the same
terminal exception at submit time.

## Decision

The three read-only preflight tools accept an optional Enumeration-only
`terminal_exceptions` array. Each entry uses the same deliverable coverage
identity and terminal fields:

```json
{
  "asset": "https://app.example.com:443",
  "technique": "GOLISH-ENUM-DIR",
  "status": "blocked",
  "note": "connection timed out from the current execution environment"
}
```

Validation is atomic and fail-closed. An entry is accepted only when:

- the active stage is `enumeration`;
- `status` is exactly `blocked` or `not_applicable` and `note` is non-empty;
- `asset` is the canonical exact-origin value of a current organization
  snapshot row;
- `technique` is registered for Enumeration and the exact snapshot cell exists;
- the current DB state of that cell is `pending`.

Unknown fields/values, foreign or absent origins, unregistered/missing
techniques, duplicate cells, and conflicting entries reject the entire preview.
In particular, a current `error` or `partial` marker outranks every planned
exception and cannot be hidden. Existing `found`, `checked_empty`, `blocked`,
`not_applicable`, and `next_wave_pending` cells also cannot be rewritten.

## Read model semantics

After validation, the tool clones the snapshot and projects accepted pending
cells as planned terminal cells for this response only. This makes pagination
skip those cells and lets readiness reflect the exact coverage the Enumerator
plans to submit. Every response reports accepted counts and an explicit
`preview_only=true`, `persisted=false` contract.

No DB row, evidence ledger entry, outcome, target, or operation state is
written. The original snapshot remains unchanged. Every subsequent preflight
call must pass the full same `terminal_exceptions` array; omitting it returns to
pure DB truth.

The first array normally starts empty. It may instead be initialized from
pinned top-level operator constraints only when they explicitly identify
same-environment preflight exact origins, prohibit producer calls for those
origins, and provide a concrete blocked note. Even then, the validator accepts
only cells that are present and pending in the current snapshot; stale-run data
and inferred/guessed exceptions are forbidden.

`stage_worklist_next.limit` remains a cell cap (maximum 200). Enumeration also
caps one page at 50 distinct exact-origin roots. The response reports returned,
matching, and omitted root counts; accepted planned-terminal cells do not count
as matching or omitted work. Callers deduplicate returned cells by origin before
building a batch tool input.

The sub-agent model-visible result path uses a dedicated bounded compactor for
the three preflight tools. It keeps all returned compact work items (up to 200),
root counts, and the complete bounded `coverage_to_submit` array while omitting
large per-item diagnostics; the raw response remains in the transcript. Generic
large-JSON sampling must not truncate the coverage that the unchanged gate will
revalidate.

## Submit invariant

This change does not modify `submit_stage_deliverable` or the gate. After the
latest preflight returns `ready_to_submit=true`, the Enumerator must copy the
same terminal exception entries into `deliverable.coverage`. The gate then
re-checks exact-origin scope, registered technique, concrete note, and the
current non-terminal evidence rule. A new `error`/`partial` fact still blocks,
so preview cannot mint a pass token.

## Safety properties

- Read-only preview; no persistence or implicit completion.
- Active organization binding remains authoritative; explicit cross-org
  overrides are rejected before snapshot read.
- Exact-origin and registered-technique matching prevents guessed or foreign
  coverage.
- Atomic validation prevents a mixed valid/invalid array from partially
  changing readiness.
- `error`/`partial` always outrank model-declared terminal exceptions.
