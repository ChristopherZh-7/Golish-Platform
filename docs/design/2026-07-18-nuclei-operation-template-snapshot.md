# Nuclei Operation Template Snapshot and Proof Reuse

## Status

Accepted for the active `enumeration-surface-manifest-vuln-applicability-2026-07-17`
feature after the 2026-07-18 Vuln Triage run exposed a deterministic convergence
failure.

## Problem

The managed `adysec/nuclei_poc` source was attached to both Nuclei modes and was
refreshed by every wrapper invocation. Every exact-origin call then performed a
fresh offline `nuclei -tl` proof over the operator template root plus the managed
PoC root. The general mode simultaneously excluded the `cve` tag, so traversing
the CVE-oriented adysec source could not contribute a runnable general template.

On the observed template supply this made each proof traverse roughly 100,000
YAML paths, reach its 60-second bound without target traffic, publish a partial
error, and repeat for the next exact origin. The same work also repeated across
worker repair generations. The stage therefore consumed CPU without advancing
its evidence-backed coverage matrix.

## Decisions

### 1. Split template sources by capability semantics

- `vuln_nuclei_general` uses only the canonical operator ProjectDiscovery
  template root. Its planner excludes `cve`; it must neither refresh nor attach
  the managed adysec CVE PoC source.
- `vuln_nuclei_fingerprint_targeted` is the only Vuln wrapper that refreshes and
  attaches `adysec/nuclei_poc:poc_gold_13`. It continues to execute only the
  exact server-selected CVE template ids justified by exact-origin
  fingerprints; it never widens to a tag or full-tree scan.
- A managed snapshot may not be attached to a General plan. The adapter rejects
  that combination so a future caller cannot silently recreate this regression.

This keeps XSS/SQLi/command-injection and baseline WSTG checks on the maintained
general template taxonomy, while the third-party PoC feed serves its intended
fingerprint-to-CVE N-day path.

### 2. Check remote freshness at most once every seven days

The first fingerprint-targeted invocation resolves the local managed checkout.
If its persisted successful-refresh stamp matches the current commit and is
less than seven days old, no remote Git command is issued. At seven days or
older, the next targeted invocation fetches the repository and writes a new
stamp only after a successful refresh. A failed refresh may use the verified
last-known-good checkout for that operation, while a later operation may retry.
Every targeted invocation in one `operation_id` reuses the same resolved commit.

The initial checkout is a shallow sparse partial clone. A due update uses an
incremental shallow partial fetch and checks out the fetched commit in the same
dedicated managed repository: upstream additions and modifications are applied,
upstream tracked deletions are removed, and the operator's separate template
tree is never replaced or recloned.

The operation cache is process-local and bounded. It stores the operation's
refresh result, including a hard failure, so sibling URLs cannot repeat the same
Git timeout. A new operation gets a new cache key and may recover. The existing
last-known-good fallback remains explicit through `stale` and `diagnostic`.
The shared checkout is protected by a read/write lock while a targeted proof or
active process is using it, so another operation cannot change files under a
launch. If a newer operation advanced the checkout between two calls of an
older operation, the older cached commit fails revalidation before Nuclei starts
instead of reporting the newer files as the older snapshot.

This boundary prevents target order from changing the template version used
inside one evidence set, avoids contacting GitHub on every scan, and caps the
normal update check at once per seven-day window.

### 3. Reuse offline template proof per operation and selection

Template proof is supply validation, not a target request. Its cache key is
therefore independent of `target_id`, exact origin, and URL. It contains:

- operation id;
- Nuclei mode;
- canonical operator template root;
- managed commit when present;
- normalized requested WSTG techniques for General, or normalized exact
  template ids for fingerprint-targeted mode.

Both complete and incomplete proofs are cached in a bounded process-local cache.
Reusing an incomplete proof is deliberate: repeating the same supply validation
against another URL cannot turn it into target evidence and was the source of
the observed loop. Each active Nuclei execution still captures and revalidates
local path witnesses immediately before process launch, so proof reuse does not
remove the final filesystem guard.

The wrapper result reports whether proof was reused. When a cached proof is
incomplete it also reports a snapshot-scoped, non-retryable blocker so the stage
worker must stop dispatching sibling URLs for the same proof key, refresh the
DB-derived worklist once, and submit its blocker instead of spinning.

### 4. Evidence and failure semantics remain fail-closed

- A proof result is never target evidence and never sets `network_attempted`.
- Incomplete proof never becomes `checked_empty` or `not_applicable`.
- Existing generated attempt markers and partial error landing remain intact;
  the new retry metadata controls orchestration without inventing coverage.
- A complete cached proof permits the normal guarded target scan. The target
  authorization and local path witnesses are still revalidated for every
  active execution.

## Non-goals

- Do not raise the proof timeout to hide repeated work.
- Do not merge or delete the operator's legacy template tree in this change.
- Do not expose arbitrary Nuclei flags or template paths to the model.
- Do not make the cache durable authority or persist it in the database.
- Do not run an active scan during implementation verification.

## Verification

Focused tests must prove that General cannot attach the managed source, targeted
plans still can, operation snapshot reuse is scoped by operation id, proof keys
ignore target URL but separate operation/source/selection changes, and cached
incomplete proof is marked non-retryable. Run the affected crate tests and
strict scoped Clippy after `just space-guard`; do not run init, precommit, or
full-workspace gates without explicit user authorization.
