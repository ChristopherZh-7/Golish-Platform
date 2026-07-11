# JS endpoint scope exclusion terminality implementation plan

1. Add failing tests that require a loopback absolute endpoint to be classified as scope-excluded while a malformed endpoint remains unresolved.
2. Replace the ambiguous resolver-only `Option` decision with `Resolved`, `ScopeExcluded`, and `Unresolved` dispositions while preserving the compatibility helper used by existing tests.
3. Count scope exclusions separately in progress, audit metadata, evidence summaries, single-target output, and bounded batch output. Do not include them in persistence-error or completion-error totals.
4. Run the focused endpoint and full `js_extract_apis` test groups, format/clippy checks, and the existing combined backend regression.
5. Rebuild the CLI and rerun Test1 Enumeration from a fresh stage attempt. Confirm the loopback candidate remains absent from `api_endpoints`, `moresec.cn` closes JSAPI/PARAM terminally, and the deterministic gate passes.
