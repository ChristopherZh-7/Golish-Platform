# Enumeration producer recovery-exhausted blocked

**Status:** Implemented
**Date:** 2026-07-11
**Scope:** Enumeration terminal projection and its agent-facing contract

## Problem

Enumeration previously treated only transport preflight as authoritative for
`blocked`. The direct browser and route producers now have bounded, persisted
recovery breakers. Once a breaker proves that repeating the same authorized
attempt cannot make progress, leaving the cell as `error` or `partial` causes the
worklist to schedule an infinite retry even though the backend has already
exhausted its safe recovery path.

The fix must not turn ordinary failures into completion. It must admit only the
producer's own recovery-exhausted evidence, only on that producer's axes, while
keeping `found` / `empty` ownership and every target/evidence guard unchanged.

## Terminal authority

`found` and `empty` remain unchanged:

| Axis | Authoritative source |
|---|---|
| `GOLISH-ENUM-JS` | `browser_collect_js_api` |
| `GOLISH-ENUM-DIR` | `route_probe_paths` |
| `GOLISH-ENUM-JSAPI` | `js_extract_apis` |
| `GOLISH-ENUM-PARAM` | `js_extract_apis` |

`blocked` uses a separate exact matrix:

| Outcome source / audit tool | Required audit kind | Allowed axes |
|---|---|---|
| `enum_preflight_web_origins` | `enumeration_transport_blocked` | JS, DIR, JSAPI, PARAM |
| `route_probe_paths` | `dir_probe_recovery_exhausted` | DIR only |
| `browser_collect_js_api` | `enumeration_collection_recovery_exhausted` | JS, JSAPI, PARAM only |

No wildcard source, kind, or axis matching is allowed. In particular,
`route_probe_paths` cannot block JS/JSAPI/PARAM, and browser collection cannot
block DIR.

## Evidence requirements

Every terminal projection still requires a positive evidence id from fresh
audit truth for the current run. The app bridge validates that the audit row:

- belongs to the current organization and current in-scope target owner;
- remains authorized for the exact canonical Web Origin and project;
- was created after the current stage freshness cutoff;
- exactly matches origin, technique, outcome, evidence id, producer tool, and
  required evidence kind.

The kit gate then independently requires the source/axis pair and the matching
`(origin, technique, evidence_id)` blocked fact. Stale, foreign-org,
target-less, owner-drifted, wrong-kind, wrong-source, and wrong-axis rows fail
closed in the worklist, UI read model, submit preview, and final org gate.

## Producer recovery semantics

`route_probe_paths` checkpoint v8 keeps per-candidate failure fingerprints,
pending verified directory writes, and terminal-publication recovery state under
the current operation/generation witness. The first network failure stays
retryable; two stable identical failures or three total failures exhaust a
candidate. Repeated directory-write or terminal-publication failures stop
automatic retry and require the explicit post-repair `retry_exhausted_*` action;
they remain `partial` and never manufacture `blocked`. DIR becomes `blocked` only after the declared
queue closes with at least one exhausted candidate, at least one candidate
request, and no persistence or verification incompleteness. The evidence kind
is `dir_probe_recovery_exhausted`.

`browser_collect_js_api` requires a guarded attempt and an actually applied
same-provenance checkpoint resume. At least one collection-blocking failure
signature must have reached count two, recovery must be exhausted with
automatic retry disabled, and persistence must be clean. It then atomically
publishes JS/JSAPI/PARAM `blocked` siblings with
`enumeration_collection_recovery_exhausted`. API-body-only diagnostics,
same-invocation duplicates, and persistence failures remain non-terminal.

Ordinary `partial` and `error` outcomes remain unfinished. A persisted producer
`blocked` result with `recovery_exhausted=true` is terminal, so agent prompts and
worklist hints must not schedule the same cell again.

## Non-goals and compatibility

- No route or browser recovery algorithm is changed by this design; it only
  admits their already persisted terminal evidence into the shared gate/read
  model.
- No database schema or migration is required.
- Model-authored coverage and non-empty `terminal_exceptions` remain forbidden;
  Enumeration submits `coverage: []`.
- Business rows such as `directory_entries`, `api_endpoints`, and
  `js_analysis_results` remain discovery context and cannot terminalize a cell.
- `not_applicable` authority is unchanged.

## Verification

Tests cover the positive matrix and fail-closed cases at all three seams:

1. app evidence projection validates exact tool/kind/axis ownership;
2. kit org gate validates exact source/axis and matching blocked evidence;
3. StageAssetCoverage projection reuses the same strict evidence-id helper.

The relevant stage spec, methodology, compact worklist text, Enumerator prompt,
and module cards state the same matrix and stop-retry rule.
