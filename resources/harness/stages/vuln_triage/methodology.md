**Goal:** run the three server-owned formulaic capabilities over only the
applicable exact-origin surfaces that `enumeration` produced. SQLi/XSS/command
injection require operation-bound GET query parameters; five root-origin WSTG
cells use controlled Nuclei; `GOLISH-NDAY` uses exact-origin fingerprint-selected Nuclei,
and `WSTG-ATHN-04` uses a bounded anonymous-access probe over server-owned
endpoint ids. This stage produces evidence-backed observations and terminal
coverage; it does not expose raw request material, Nuclei arguments, manually
select templates, exploit a result, or write a Candidate/Finding. Reasoning
over observations belongs to `attack_candidate`.

The exact final-sealed `vuln_triage` handoff may enter the initial Wave only.
Every follow-on Wave entry must come from an immutable, accepted FactDelta
consolidation. Never reuse the initial handoff for a later Wave and never
fabricate a follow-on manifest from prose, process-local memory, or verifier
output that consolidation has not accepted.

**Recommended sequence (only on exact origins inherited from the final-sealed
`enumeration` handoff):**

1. Load the stage-local worklist first — `stage_worklist_status`. If
   `ready_to_submit=false`, call `stage_worklist_next(prefer=["pending","error"])`
   and treat its `items` as the exact plan: each item is one asset×technique cell
   with a `work_item_id`, `asset`, `technique`, `state`, and `suggested_tools`.
   Work only the named cells, then re-query after tools land DB truth. Cells
   marked `not_applicable` with source `enumeration_surface_manifest` are already
   closed by deterministic backend context; do not dispatch them or recreate
   them in deliverable coverage.
2. For a Nuclei work item, obtain its server-side target id and exact absolute
   `target_url` from the worklist or `query_target_data`; never invent an id,
   origin, path, port, or scheme.
   - For `WSTG-INPV-05`, `WSTG-INPV-01`, and `WSTG-INPV-12`, call
     `vuln_nuclei_general` only with injection cells in that call. The backend
     loads current-operation exact-origin GET query inputs, removes every
     captured value, inserts inert placeholders, and runs `-dast -fa low` under
     single-concurrency/low-rate/zero-retry bounds. Never mix these injection
     techniques with root-origin baseline techniques in one call.
   - For the other five general-Nuclei `WSTG-*` cells call
     `vuln_nuclei_general(target_id=..., target_url=..., techniques=[...])`.
     The backend owns the safe tag profile and all Nuclei flags.
   - For `GOLISH-NDAY` call
     `vuln_nuclei_fingerprint_targeted(target_id=..., target_url=...,
     techniques=["GOLISH-NDAY"])`. The backend selects the exact template-id set
     from fingerprints bound to this exact origin and the local PoC knowledge base. No matched
     templates is an explicit result and never falls back to the general scan.
   - General Nuclei never loads the managed adysec CVE feed. Fingerprint-targeted
     N-day uses a dedicated sparse checkout of `adysec/nuclei_poc`
     `poc_gold_13`. A matching valid checkout whose last successful refresh is
     less than seven days old is reused without remote access; at seven days or
     older the next targeted call fetches once. All targeted calls in one Vuln
     operation pin the same commit. A valid previous snapshot may be used only
     with an explicit stale diagnostic when refresh fails; with no valid
     snapshot the targeted wrapper fails before target traffic. The managed
     checkout never mutates the operator's existing template tree.
   - Offline template proof is operation/snapshot scoped and may be reused across
     exact origins. If a wrapper returns `automatic_retry_allowed=false` with
     `retry_scope=template_snapshot`, stop all sibling Nuclei dispatch for that
     proof scope. Refresh the worklist once and submit the backend-reported
     blocker accurately; do not try the same template supply on another URL or
     repair generation. This blocker is not checked-empty target evidence.
3. For a `WSTG-ATHN-04` work item, call
   `query_target_data(target_id=..., sections=["endpoints"])` before probing.
   Filter the returned metadata to the exact origin. The wrapper then intersects
   it with the current operation manifest and returns the exact eligible ids if
   the review set differs. Review that operation-bound universe, select a small
   evidence-driven subset, then call
   `vuln_probe_anonymous_access(target_id=..., target_url=...,
   reviewed_endpoint_ids=[...], selected_probes=[{"endpoint_id":...,
   "query_values":{...},"rationale":...}])`. `reviewed_endpoint_ids` must be
   the complete eligible id set returned for the exact origin;
   `selected_probes` is the AI-selected subset (maximum 16). An empty selected
   subset is valid only after the complete review and records not-applicable.
   Do not blindly probe every endpoint. Do not pass per-endpoint URLs, methods,
   headers, cookies, tokens, bodies, redirect controls, or CLI arguments; the
   top-level `target_url` is only the exact authorized-origin witness and the
   backend reloads the current endpoint rows and owns request construction.
   - True IDOR/BOLA (`WSTG-ATHZ-04`) is not part of this stage; it requires
     later role/object comparison through Candidate verification.
   - All three capabilities are guarded foreground calls. Do not use background
     job controls, legacy manual credential/ID-substitution probes, raw
     `nuclei`, `pentest_run`, shell,
     template paths, CLI args, or caller-authored HTTP material.
4. Land results as DB truth. Every terminal formulaic cell (`found`,
   `checked_empty`, `blocked`, or `not_applicable`) must be a durable
   operation-scoped `technique_outcomes` row grounded by matching current-owner,
   org-bound evidence from the exact wrapper. Legacy shell evidence,
   `source_query`, and hand-written deliverable coverage never close a cell. Do
   not write a Candidate or Finding and do not hand-fabricate a terminal state. Malformed,
   truncated, timed-out, non-zero, foreign-origin, or otherwise inconclusive
   output remains partial/error.
5. Slim submit — call `stage_worklist_status` + `check_stage_asset_coverage`.
   Only submit when `ready_to_submit=true`, with `coverage=[]`. Coverage is
   reconstructed from current operation-scoped DB truth; model-authored
   `found`, `checked_empty`, `blocked`, or `not_applicable` rows are never gate
   authority.

**Efficiency red lines:**

- This is a formulaic observation stage, not bespoke exploitation. Do not craft
  payloads or request material, choose a template path/id, or run a manual
  scanner/probe.
- Reuse enumeration's endpoints/params/dirs as the denominator; do not
  re-enumerate content or re-port-scan.
- One guarded foreground call per exact target×surface-class group; never mix
  parameter DAST and root-origin baseline cells. For anonymous
  access, one complete review plus bounded AI-selected probe subset per target. Refresh the
  worklist after the call instead of launching a duplicate.
- A template-snapshot-scoped non-retryable result applies to every sibling URL
  sharing that proof key. Re-dispatch cannot produce target evidence and is a
  worker protocol violation.

**Coverage + stop condition (denominator matters):**

- The backend must derive a terminal outcome for every inherited exact-origin ×
  technique cell. A missing DB outcome is `not_attempted` and FAILS the gate;
  omission is never `checked_empty` (I8). `blocked` requires wrapper evidence,
  and `not_applicable` requires deterministic backend context.
- Wrapper outcomes own their tested/total surface accounting. A partial,
  malformed, timed-out, or error result remains non-terminal and cannot be
  converted into a deliverable-side terminal status. Retry it through the
  worklist only when the wrapper permits automatic retry; a
  `template_snapshot` non-retryable blocker must instead stop sibling dispatch
  and be submitted as an unresolved backend blocker.
- Submit `findings=[]` and legacy `candidates=[]`. Formulaic found and
  checked-empty outcomes are both retained observation facts; the exact sealed
  set becomes the initial Wave's immutable reasoning manifest. A later Wave may
  contain only members selected by the accepted FactDelta consolidation for
  that Wave entry.
