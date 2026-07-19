# Enumeration Surface Manifest & Vuln Applicability Implementation Plan

> Follow TDD for every behavior change. Do not run `./init.sh` or `just precommit` in this session per the user's explicit instruction.

**Goal:** Turn Enumeration's final-sealed exact-origin output into the authoritative Vuln testing denominator, then run anonymous checks, injection DAST and fingerprint POCs only against operation-bound applicable surfaces.

**Architecture:** Add normalized ownership-safe manifest tables beside the existing endpoint/fingerprint tables. Producers publish operation/exact-origin relations transactionally. The final Enumeration handoff snapshot derives trusted applicability cells, and Vuln tools consume the same snapshot rows. A separate managed template-source component refreshes `adysec/nuclei_poc` before each Nuclei execution and records the exact snapshot.

**Tech Stack:** Rust 2021, sqlx/Postgres migrations, tokio process execution, Nuclei 3.8+, existing Golish evidence/gate contracts.

---

### Task 1: Register the feature and preserve the paused feature state

**Files:**
- Modify: `feature_list.json`
- Modify: `agent-progress.md`

1. Move `ai-event-restore-dispatch-visibility-2026-07-17` from `in_progress` to `blocked` with the explicit no-precommit reason.
2. Add `enumeration-surface-manifest-vuln-applicability-2026-07-17` as the only `in_progress` feature.
3. Record the approved DB/schema boundary and the no-init/no-precommit verification restriction.
4. Verify with `jq empty feature_list.json` and an exact one-in-progress query.

### Task 2: Add normalized Enumeration surface schema

**Files:**
- Create: `backend/crates/golish-db/migrations/20260717000003_enumeration_surface_manifest.sql`
- Create: `backend/crates/golish-db/tests/enumeration_surface_manifest.rs`
- Modify: `backend/crates/golish-db/src/repo/mod.rs`
- Create: `backend/crates/golish-db/src/repo/enumeration_surface_manifest.rs`

1. Write a failing fresh-migration integration test that queries the three new relations and their constraints.
2. Run `just space-guard`, then the single DB test and preserve the expected RED evidence.
3. Add the migration with ownership/FK/unique/location checks and useful operation/origin indexes.
4. Add typed rows plus guarded publish/list APIs. Lock and validate target, operation, origin, endpoint and fingerprint ownership inside one transaction.
5. Add tests for idempotent merge, exact-origin isolation, operation isolation, raw-value omission and cross-owner rejection.
6. Run the focused DB tests to GREEN.

### Task 3: Publish exact-origin fingerprint observations

**Files:**
- Modify: `backend/crates/golish-pentest/src/output_store/targets.rs`
- Modify: related focused tests in `backend/crates/golish-pentest/src/output_store/`

1. Add a failing transaction test proving a WhatWeb fingerprint is not queryable for another port/origin.
2. Make `store_active_http_surface_identity` return the upserted `web_origin_id`.
3. After each active fingerprint upsert, publish the origin observation in the same transaction.
4. Preserve target-global fingerprint rows for UI/backward compatibility.
5. Run focused output-store tests.

### Task 4: Publish Enumeration endpoint and parameter observations

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/browser_collect_js_api.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/js_extract_apis.rs`
- Modify: focused tests beside those modules

1. Add failing tests for query versus body/form location publication and manifest persistence failure propagation.
2. Resolve the exact `web_origin_id` from the already-authorized origin.
3. Upsert the existing `api_endpoints` row, then publish the operation-bound observation and normalized parameter names.
4. Never publish captured parameter values.
5. Add any manifest failure to the existing persistence error list so Enumeration cannot falsely terminalize JSAPI/PARAM.
6. Run focused bridge tests.

### Task 5: Derive the dynamic Vuln denominator

**Files:**
- Modify: `backend/crates/golish-db/src/repo/stage_coverage.rs`
- Modify: `backend/crates/golish-db/src/repo/org_gate.rs`
- Modify: relevant stage coverage/gate tests
- Modify: `resources/harness/stages/enumeration/{spec.json,methodology.md}`
- Modify: `resources/harness/stages/vuln_triage/{spec.json,methodology.md}`

1. Add failing tests for four snapshots: no surface, endpoint only, GET query parameter, and exact-origin fingerprint.
2. Join only the final-sealed Enumeration handoff operation to build per-origin surface counts and executable query inputs.
3. Emit trusted `not_applicable` cells with source `enumeration_surface_manifest` for inapplicable SQLi/XSS/Command Injection, Anonymous Access and N-day.
4. Make the gate import only those backend-generated cells into deterministic N/A context; reject model-authored equivalents.
5. Update stage instructions so Workers publish the manifest and the Controller groups work by surface class.
6. Run focused coverage/gate tests.

### Task 6: Bind anonymous and fingerprint-targeted tools to the manifest

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/anonymous_access.rs`
- Modify: `backend/crates/golish-scan-runner/src/lib.rs` or the current template-selection module
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`
- Modify: focused tests

1. Add RED tests showing an endpoint from another Enumeration operation and fingerprint from another port are excluded.
2. Load anonymous candidates only through `(operation_id, web_origin_id)` manifest rows.
3. Select Nuclei exact template IDs only from origin-bound fingerprints; remove target-global fallback.
4. Run focused scan-runner and pentest-app tests.

### Task 7: Add parameter-aware low-aggression Nuclei DAST

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`
- Modify: focused Nuclei adapter tests

1. Add failing planner/proof tests asserting injection techniques do not scan a root URL and require manifest query inputs.
2. Build canonical same-origin GET URLs with inert values from normalized parameter names.
3. For SQLi/XSS/Command Injection add `-dast -fa low`; do not exclude fuzz templates, and retain all existing protocol, redirect, OAST, rate and exact-origin guards.
4. Reject mixed injection/root-baseline technique groups and missing executable inputs before spawning Nuclei.
5. Make offline proof use the exact active DAST/tag/exclusion policy.
6. Run focused adapter tests and a loopback-only executable smoke test.

### Task 8: Refresh the managed adysec template source per scan

**Files:**
- Create: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei_template_source.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- Modify: focused tests

1. Add RED tests around a temporary local bare Git remote for first clone, later commit update, invalid update rollback and no-snapshot failure.
2. Implement a process-wide async mutex and bounded `git` subprocesses without shell interpolation.
3. Sparse-checkout only `poc_gold_13` into `~/.golish/nuclei-template-sources/adysec-nuclei_poc`.
4. Validate the fetched revision with offline Nuclei listing before marking it last-known-good.
5. Add the exact managed snapshot as a second template root and include commit/freshness in evidence/witness metadata.
6. On refresh failure, use last-known-good only with an explicit stale diagnostic; fail before target traffic when none exists.
7. Run the local-remote tests; optionally perform one user-authorized GitHub refresh verification without scanning a target.

### Task 9: Synchronize module cards and record targeted evidence

**Files:**
- Modify: `docs/modules/backend/golish-db.md`
- Modify: `docs/modules/backend/golish-db/repo.md`
- Modify: `docs/modules/backend/golish-pentest/output_store.md` if present
- Modify: `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `agent-progress.md`
- Modify: `feature_list.json`

1. Update responsibilities, public contracts, ownership rules, failure semantics and focused test entrypoints.
2. Run affected-crate `cargo check`, focused nextest suites, Clippy `-D warnings`, rustfmt check, JSON validation and `git diff --check`.
3. Do not run `./init.sh` or `just precommit`; therefore keep the feature `in_progress` and state exactly which full completion gates were skipped.
4. Record commands, exit codes and material output in `agent-progress.md` and the feature evidence field.
