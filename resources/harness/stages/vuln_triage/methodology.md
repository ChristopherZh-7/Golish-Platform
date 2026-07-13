**Goal:** run the FORMULAIC vulnerability scan over every testable unit
`enumeration` produced. This stage is a *broad, mechanical sweep* of the
technique classes a tool + dictionary/template can cover with a relatively
objective verdict — not creative exploitation. Anything that needs reasoning
about business semantics (SSRF chains, SSTI, LFI, auth-bypass logic, business
logic) is deliberately NOT here; it is synthesized in the next stage
(`attack_candidate`). Even a formulaic `found` result is an evidence-backed
observation seed here, not yet a retained Candidate or Finding. Candidate V2
creates neither row until the next stage reasons over the exact final-sealed
manifest and its final Gate PASS transaction accepts the result.

The exact final-sealed `vuln_triage` handoff may enter the initial Wave only.
Every follow-on Wave entry must come from an immutable, accepted FactDelta
consolidation. Never reuse the initial handoff for a later Wave and never
fabricate a follow-on manifest from prose, process-local memory, or verifier
output that consolidation has not accepted.

**Recommended sequence (only on live services / endpoints from enumeration):**

1. Load the stage-local worklist first — `stage_worklist_status`. If
   `ready_to_submit=false`, call `stage_worklist_next(prefer=["pending","error"])`
   and treat its `items` as the exact plan: each item is one asset×technique cell
   with a `work_item_id`, `asset`, `technique`, `state`, and `suggested_tools`.
   Work only the named cells, then re-query after tools land DB truth.
2. Batch the formulaic classes per asset, reusing enumeration's endpoints/params
   as the denominator:
   - `GOLISH-NDAY` — run nuclei with the fingerprint-matched template set (CVE /
     n-day PoCs); do not hand-write CVE findings the scanner did not confirm.
   - `WSTG-CONF-05` — sensitive dir/config sweep over the enumerated directory
     surface (reuse `directory_entries`; do not re-fuzz what enumeration mapped).
   - `WSTG-ATHN-02` — weak-credential / default-login checks against discovered
     auth endpoints with a bounded dictionary.
   - `WSTG-CRYP-03` — TLS posture (testssl-class) on each live HTTPS service.
   - `WSTG-INFO` — version/banner/error-message info leak from fingerprints.
   - `WSTG-SESS-02` — cookie/session attribute + CSRF-token presence checks.
   - `WSTG-ATHZ-04` — SHALLOW IDOR: mechanical id substitution on parameterized
     endpoints (deep object-relationship abuse belongs to `attack_candidate`).
   - `WSTG-INPV-05 / -01 / -12` — SQLi / XSS / command-injection TOOL sweeps
     (sqlmap / dalfox / injection scanners) to catch low-hanging results; hand
     the suspicious-but-unconfirmed points to `attack_candidate`, do not manually
     craft deep payloads here.
3. Land results as DB truth. Every terminal formulaic cell (`found`,
   `checked_empty`, `blocked`, or `not_applicable`) must be a durable
   `technique_outcomes` row grounded by real evidence. Do not write a Candidate
   or Finding and do not hand-fabricate `found` cells the database can derive.
4. Slim submit — call `stage_worklist_status` + `check_stage_asset_coverage`.
   Only submit when `ready_to_submit=true`, adding summary claims + the
   checked_empty/blocked/not_applicable coverage that DB truth cannot derive.

**Efficiency red lines:**

- This is a formulaic sweep, NOT bespoke exploitation. Do not spend turns
  hand-crafting a single deep exploit here — record the lead and let
  `attack_candidate` reason about it.
- Reuse enumeration's endpoints/params/dirs as the denominator; do not
  re-enumerate content or re-port-scan.
- One template/dictionary pass per asset×technique; if a scan is backgrounded,
  wait for its completion note instead of re-running the same command.

**Coverage + stop condition (denominator matters):**

- Per in-scope asset, give each of the 10 formulaic techniques a terminal status
  in `coverage`: found / checked_empty / blocked|not_applicable
  +note. A MISSING (asset × technique) cell counts as not_attempted and FAILS the
  gate — "omitted" is not "checked-empty" (I8).
- For found/checked_empty cells set `tested_units`/`total_units` (M = enumerated
  units for that technique). Full coverage needs `tested_units == total_units`;
  to sample a huge surface set `sampling_rationale` and meet the ratio, else the
  cell counts as partial and the gate BLOCKS.
- Submit `findings=[]` and legacy `candidates=[]`. Formulaic found and
  checked-empty outcomes are both retained observation facts; the exact sealed
  set becomes the initial Wave's immutable reasoning manifest. A later Wave may
  contain only members selected by the accepted FactDelta consolidation for
  that Wave entry.
