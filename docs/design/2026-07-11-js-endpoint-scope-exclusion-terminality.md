# JS endpoint scope exclusion terminality

## Context

The Test1 Enumeration run `stage-run-312239cf-f712-4f82-8f95-3a7037485f5d` completed a fresh `js_extract_apis` pass for `https://moresec.cn:443/`: all 120 manifest files were read, eight same-origin endpoints were persisted, and no read or database write failed. One deterministic HaE candidate was the absolute URL `http://127.0.0.1:4002/api/v1/mcdp`.

The exact-origin guard correctly refused to write that loopback URL under the `moresec.cn` target. The old resolver nevertheless returned the same `None` value for both an intentional scope rejection and a genuinely unparseable candidate. The persistence loop counted both as `unresolved_endpoint_rows`, so the clean pass remained `partial` forever and an identical retry could never close the JSAPI/PARAM cells.

## Decision

Endpoint projection has three explicit dispositions:

1. `Resolved`: a valid HTTP(S) endpoint on the target's exact origin. Persist it through the guarded API endpoint upsert.
2. `ScopeExcluded`: a syntactically valid endpoint that is outside the exact origin or uses an unsupported scheme. Keep it in `js_analysis_results` as source evidence, increment `scope_excluded_endpoint_rows`, and do not write it to `api_endpoints`. This is a deterministic checked decision and does not make the extraction incomplete.
3. `Unresolved`: a candidate whose target base or endpoint value cannot be parsed/resolved. Increment `unresolved_endpoint_rows`; this remains non-terminal `partial`.

The distinction is diagnostic only and does not weaken the exact-origin boundary. A foreign, loopback, protocol-relative foreign, or non-HTTP candidate is never promoted into the target's API table.

## Output contract

Single-target audit metadata and responses, plus bounded batch summaries, expose `scope_excluded_endpoint_rows`. Existing `unresolved_endpoint_rows` retains its fail-closed meaning. Raw endpoint evidence remains available in the per-file JS analysis rows.

## Verification

- A loopback absolute candidate is classified `ScopeExcluded`, is not resolved for persistence, and does not prevent a clean pass from becoming terminal.
- A malformed candidate is still `Unresolved` and keeps the pass `partial`.
- Existing external/non-HTTP rejection tests continue to prove that no out-of-scope endpoint is returned for persistence.
- Focused `js_extract_apis` tests, clippy, the combined backend regression, and a fresh Test1 live run must pass before the Enumeration feature can be marked `passing`.
