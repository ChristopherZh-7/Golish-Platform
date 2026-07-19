# Vuln Outcome Set Final Seal

## 1. Problem

The live operation `607e2a51-06dd-412f-b9d2-cfca23f5c776` produced a complete
Vuln Triage matrix for organization `acca4a29-3ac7-4a41-95e3-dfaf85d54f21`:
36 exact web origins multiplied by 10 formulaic techniques, yielding 360 unique,
terminal, evidence-backed `technique_outcomes` rows. The deterministic org Gate
passed, but the V2 final seal failed after the Gate because the handoff catalog
kept at most 256 individual canonical keys and the Vuln closeout then required
one included key per terminal cell.

This is not a producer, network, repair, or missing-evidence failure. Re-running
the workers deterministically recreates the same 360 rows and the same 256-row
truncation.

Two adjacent contracts are also incorrectly coupled to raw coverage size:

- Reporting currently requires every operation-scoped TechniqueOutcome row to
  have an individual sealed reference.
- Candidate correctly aggregates ten formulaic cells into one asset-level
  `surface_analysis`, but an earlier attestation rejects raw terminal-cell counts
  above `MAX_ATTACK_MANIFEST_ITEMS=100` before aggregation happens.

## 2. Decision

Vuln Triage final seals will use one virtual canonical `TechniqueOutcomeSet`
reference for the complete `(organization, operation run, stage)` outcome set.
No new table or migration is required. The existing immutable StageHandoff JSON
can serialize the new tagged canonical key, and the DB resolver will re-read and
lock the underlying `technique_outcomes` rows before accepting it.

The key is:

```rust
TechniqueOutcomeSet {
    organization_id: Uuid,
    run_id: String,
    stage: String,
    terminal_cell_count: u32,
    outcome_set_sha256: String,
}
```

`outcome_set_sha256` hashes the sorted terminal identity tuples
`(asset, technique, normalized_state)`. `empty` is normalized to
`checked_empty`. Evidence IDs and mutable row metadata are intentionally not in
this identity digest: the resolver independently hashes the full sorted DB row
content into `CanonicalFactRef.content_sha256` and returns the exact sorted union
of evidence IDs. The identity digest proves that the Gate snapshot and DB set
contain the same cells; the content digest proves that the sealed DB rows have
not changed.

The virtual set is valid only for `stage="vuln_triage"` and only when `run_id` is
the exact operation UUID. A non-empty set requires every row to be terminal, unique by
`(asset, technique)`, organization-owned, project-owned, fresh, and backed by
positive evidence IDs. An empty set is valid only after the existing
authoritative zero-denominator checks have proven `total_assets=0` and
`assets=[]`; its observed time is the seal ceiling. A count, identity digest,
content digest, ownership,
freshness, or evidence mismatch fails closed.

## 3. Final-seal behavior

For Vuln Triage:

1. Runtime reads the authoritative coverage snapshot and the raw exact-run
   outcome projection.
2. Runtime compares the complete normalized tuple sets before any catalog
   bounding.
3. Runtime emits one `TechniqueOutcomeSet` key plus any independent Finding
   keys. Finding counts are not mixed into terminal outcome counts.
4. The DB final-seal transaction re-locks every member row, recomputes the set
   count/digest and full content hash, validates the union evidence, and writes
   the resolved set reference into the immutable StageHandoff.
5. Response-loss replay re-resolves the same set key in the original
   `[Unit.started_at, handoff.gate_passed_at]` window. It therefore detects
   removal or drift of sealed members without letting later writes change the
   replay set. A row added after the seal is outside that replay set; Reporting
   still rejects it as an unsealed operation-scoped row.

Other information stages keep their existing individual-reference behavior.
This change is deliberately narrow to the operation-scoped Vuln set that caused
the cardinality contradiction.

## 4. Reporting behavior

Reporting remains fail closed and complete. It accepts both:

- legacy individual `TechniqueOutcome` refs; and
- one current `TechniqueOutcomeSet` ref for an exact operation/org.

For a set ref, Reporting re-reads all exact operation rows in its repeatable-read
transaction, recomputes the identity and full-content attestations, and requires
an exact match with the sealed reference. It then emits every underlying
TechniqueOutcome as an individual report source. The set reference reduces
handoff cardinality; it does not hide, sample, or discard report facts.

## 5. Candidate behavior

Candidate continues to read and validate the complete Vuln outcome set. Raw
terminal cells are coverage context, not Candidate work items. The invalid
pre-aggregation check `terminal_cells <= MAX_ATTACK_MANIFEST_ITEMS` is removed.
The existing post-aggregation limit remains authoritative:

```text
complete terminal cells
  -> one surface_analysis per exact origin
  -> zero or more typed positive scanner leads
  -> enforce final observations/work-items <= 100
```

`empty`, `blocked`, and `not_applicable` remain in the asset-level coverage
context and in Reporting. They do not become 360 independent Candidate tasks.
Positive typed observations remain separate actionable leads.

## 6. Failure classification

A deterministic org Gate BLOCK remains eligible for bounded coverage repair.
An error after Gate PASS while assembling, resolving, or committing the final
seal is a finalization failure, not a coverage gap. Stage Run must return a typed
`COMPANY_CONTROLLER_FINAL_SEAL_FAILED` gap, preserve the same durable Controller
and submission, halt the current request, and instruct a separate continuation
after the code/storage condition is fixed. It must not dispatch producer repair
or rescan assets.

## 7. Compatibility and safety

- No schema or migration change.
- Existing StageHandoff JSON and legacy individual keys remain readable.
- FactDelta subjects remain individual facts; `TechniqueOutcomeSet` is rejected
  as a FactDelta subject.
- The existing 256-key and 256 KiB payload bounds remain. The set reference
  removes outcome-cardinality growth without expanding either bound.
- The existing 1024 evidence-ID bound remains an explicit policy boundary. It
  must fail with a capacity-specific error rather than silently truncate.
- Assets are never deleted or silently excluded by this change.
- No scan, exploit, external API, or provider call is part of implementation
  verification.

## 8. Acceptance

- A 360-cell Vuln matrix produces one set key, no truncation, and a valid final
  seal.
- Removing one row or changing one state makes set resolution fail.
- Adding a real Finding does not corrupt the outcome-count check.
- Final-seal replay detects sealed-member removal and evidence/content drift;
  Reporting rejects post-seal unsealed additions.
- Reporting accepts the set and still emits all 360 TechniqueOutcome sources;
  missing or changed rows fail closed.
- More than 100 raw cells can seed Candidate when aggregation yields at most 100
  observations; more than 100 final observations still fails.
- A post-Gate final-seal error is not classified as coverage repair.
