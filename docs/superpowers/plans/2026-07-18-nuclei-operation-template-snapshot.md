# Nuclei Operation Template Snapshot Implementation Plan

> Design: `docs/design/2026-07-18-nuclei-operation-template-snapshot.md`
>
> Active feature: `enumeration-surface-manifest-vuln-applicability-2026-07-17`

**Goal:** Stop Vuln Triage from repeating managed-template refresh and offline
proof for every exact URL while keeping fingerprint-selected N-day scans fresh,
exact, and evidence-safe.

**Architecture:** General Nuclei uses only the operator template root. Targeted
N-day resolves one managed adysec commit per operation. A bounded process-local
proof cache reuses supply validation by operation, source identity, and
technique/template selection; active target execution retains per-call DB and
filesystem guards.

**Tech Stack:** Rust 2021, Tokio, `golish-pentest-app`, harness Markdown/JSON
contracts, focused nextest and Clippy.

---

### Task 1: Lock source-mode semantics with failing tests

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`
- Test: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`

1. Add a test that attaching a managed CVE snapshot to a General plan is
   rejected.
2. Retain and strengthen the targeted-plan assertion that active and proof
   commands contain the managed root and exact template id.
3. Run only the named tests and record the expected RED result before changing
   implementation.

### Task 2: Refresh at most every seven days and pin per operation

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei_template_source.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`
- Test: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei_template_source.rs`

1. Persist a bounded refresh stamp under the managed repository's Git metadata;
   reuse a matching valid checkout without remote access while the last
   successful refresh is less than seven days old.
2. Keep the first install shallow+sparse, and make due updates use incremental
   shallow partial fetch so upstream add/modify/delete is applied without a
   reclone or any mutation of the operator template tree.
3. Add a bounded process-local cache keyed by `operation_id`, including hard
   failure, so sibling URLs cannot repeat a Git timeout.
4. Expose `managed_adysec_nuclei_poc_for_operation(operation_id)` and protect
   checkout refresh/use with a read/write guard plus commit revalidation.
5. Add deterministic tests for the seven-day boundary, incremental fetch
   contract, same-operation reuse, hard-failure memoization, operation
   isolation, and bounded eviction without contacting GitHub.

### Task 3: Route managed source only to targeted N-day

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/nuclei.rs`

1. Build General plans without resolving or attaching an adysec snapshot.
2. Resolve the operation-scoped managed snapshot only after targeted selection
   has produced a non-empty strict CVE template set.
3. Emit `managed_template_source` only for targeted results and include its
   operation scope.

### Task 4: Cache template proof and expose deterministic retry metadata

**Files:**
- Modify: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`
- Test: `backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`

1. Define a normalized proof key from operation id, mode, canonical template
   roots/managed commit, and requested techniques or exact ids.
2. Add a bounded cache for complete and incomplete `NucleiTemplateProof`
   values; return a cache-hit flag alongside the proof.
3. Add result fields for cache reuse and a snapshot-scoped non-retryable blocker
   when an incomplete cached proof is returned.
4. Test that URL/target changes do not change the key, while operation,
   selection, or managed commit changes do.

### Task 5: Teach the stage worker to stop sibling dispatch after a cached blocker

**Files:**
- Modify: `resources/harness/stages/vuln_triage/methodology.md`
- Modify: `resources/harness/stages/vuln_triage/spec.json`

1. Document that adysec refresh applies only to fingerprint-targeted N-day,
   checks the remote at most once every seven days, and pins one commit per Vuln
   operation.
2. Require a worker receiving `automatic_retry_allowed=false` with
   `retry_scope=template_snapshot` to stop sibling Nuclei dispatch, refresh the
   worklist once, and submit the backend blocker accurately.
3. Parse `spec.json` and check the diff.

### Task 6: Focused verification and records

**Files:**
- Modify: `docs/modules/backend/golish-pentest-app.md`
- Modify: `docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- Modify: `docs/modules/INDEX.md`
- Modify: `agent-progress.md`
- Modify: `feature_list.json`

1. Run `just space-guard` before Cargo.
2. Run the named adapter/source/cache tests, then the affected
   `golish-pentest-app` Vuln Nuclei test slice.
3. Run scoped `cargo clippy -p golish-pentest-app --all-targets -- -D warnings`,
   Rust formatting check, JSON parse, and `git diff --check` for touched files.
4. Record commands, exit codes, evidence, remaining risk, and the requirement to
   restart the currently running old binary. Do not run `./init.sh`,
   `just precommit`, or full-workspace gates.
