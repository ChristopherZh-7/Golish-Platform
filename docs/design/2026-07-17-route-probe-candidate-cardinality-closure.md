# Route Probe / Candidate Cardinality Closure

## 1. Problem

Live operation `2b2c2271-b8ea-4196-897b-799f144bb9ee` reached
`attack_candidate` for organization `905b143b-3ef0-4df6-9b9b-5a533bcd27a7`,
then failed before provider dispatch with:

```text
typed Candidate manifest exceeds the frozen Wave policy
```

The upstream facts are complete, but two contracts compose incorrectly:

- `route_probe_paths` saw two exact origins return HTTP 200 with an empty body
  for every wordlist path. Candidate and random-baseline signatures had the
  same status, content type, zero length, empty-body hash, and template hash.
  `response_signatures_are_uniform` nevertheless required `body_len > 0`
  before comparing those hashes, so 1,866 rows per origin were persisted as
  `verified_positive`.
- Candidate correctly aggregates 210 formulaic Vuln cells into 21
  `surface_analysis_v1` observations, but then expands every positive
  `directory_entries` row into an independent work item. The live shape became
  21 surfaces plus 3,735 directory work items, exceeding the final
  `MAX_ATTACK_MANIFEST_ITEMS=100` policy.

The failed seed transaction left one queued Candidate StageRunUnit and no
`attack_wave_runs`, `attack_wave_units`, `attack_candidate_seeds`,
`attack_candidate_work_items`, or V2 Candidate rows. Retrying `stage_run`
recomputes the same oversized manifest and cannot converge.

## 2. Decision

Fix both ends of the contract without deleting evidence, widening the 100-item
policy, or adding a schema migration.

### 2.1 Empty-body uniform response

Two same-status, zero-body signatures are uniform when all available
representation fields agree:

- `body_len == 0` for both;
- body and normalized-template hashes match;
- declared content lengths match;
- content types match case-insensitively.

The request URL is deliberately not compared: the candidate and random
baseline must use different paths. A status, content-length, content-type, or
hash difference remains eligible for ordinary positive classification. This
closes the exact live false-positive without treating every zero-byte response
as uniform.

### 2.2 Directory observations are per exact origin, not per row

Candidate will preserve every authorized directory row in the relational DB,
but materialize at most one `directory_entry_set_v1` observation per exact Web
Origin. The set observation contains:

```json
{
  "schema": "directory_entry_set_v1",
  "target_id": "<current target UUID>",
  "origin": "https://example.test:443",
  "entry_count": 1866,
  "entry_set_sha256": "sha256:<complete sorted row-set digest>",
  "entries_preview": [],
  "preview_count": 0,
  "preview_truncated": true,
  "source_evidence_ids": [31163]
}
```

The digest covers every sorted canonical row projection:
`id,target_id,url,status_code,content_length,content_type,tool`. The preview is
bounded to 32 rows and selected deterministically by security-relevance rank,
then URL and row id. The complete set count/hash—not the preview—is frozen into
the observation hash. Response-loss replay therefore detects relational drift
without putting thousands of rows in the model prompt.

One set uses the origin target snapshot, technique `WSTG-INFO`, allowed
techniques `[WSTG-INFO]`, and the exact sorted source evidence IDs. Existing
surface observations keep those same support evidence IDs. Foreign target,
project, origin, time window, tool, non-2xx, root-path, missing length, or
unlinked evidence rows remain excluded exactly as before.

The final 100-item policy is still applied after aggregation. A genuinely large
number of distinct surface/set/scanner observations continues to fail closed;
this change does not silently sample or enlarge Candidate fuel.

## 3. Existing-operation recovery

No historical rows are deleted or rewritten. After the new binary is active,
the queued generation-0 Candidate StageRunUnit can replay its seed transaction:

1. 210 Vuln cells re-attest and aggregate to 21 surfaces.
2. 3,735 exact directory rows re-attest and aggregate to five origin sets.
3. The final manifest contains 26 observations because this operation has no
   positive scanner leads.
4. Wave/seed/work-item rows are created atomically and the analyst may dispatch.

The route-probe classifier fix prevents the same empty-body flood in future
Enumeration runs. The set aggregation is independently required because a
legitimate origin may still expose more than 100 real paths.

## 4. Worklist/UI boundary

The generic `stage_worklist_status` coverage projection is not Candidate
manifest truth. This repair does not treat its zero cells or
`ready_to_submit=true` as evidence that Candidate ran. Provider dispatch and
the frozen relational manifest remain authoritative. A separate UI/read-model
change is not required to unblock the operation, but the pre-dispatch error
must continue to be returned verbatim rather than translated into completion.

## 5. Safety and compatibility

- No migration, table, column, generated IPC type, or external service call.
- No historical evidence or directory row deletion.
- No transaction performs HTTP or other long work.
- Existing single-row directory observations change shape only for manifests
  not yet frozen. Already frozen WaveUnits remain immutable and replay their
  existing manifest.
- Exact operation/org/scope/project/target/origin/freshness guards remain.
- The 100-item final policy and 64-evidence-id per observation policy remain.

## 6. Acceptance

- Identical zero-body candidate and baseline signatures classify as uniform.
- A meaningful zero-body signature difference remains non-uniform.
- 101+ exact positive directory rows for one origin produce one typed set whose
  complete count/hash changes when any member changes.
- The set preview is deterministic, bounded, and reports truncation.
- Owner/origin/time/tool/status guards still fail closed or exclude invalid
  rows.
- The live 21-surface + 3,735-row shape materializes to 26 final observations,
  below but without weakening the 100-item policy.
- Focused tests, affected-crate Clippy, rustfmt, full `just precommit`, JSON and
  diff checks pass before the feature is marked `passing`.
