**Goal:** enumerate CONTENT on the services EAS already mapped — JavaScript/API
endpoints, directories/paths, and parameters. Port/service discovery is already
done in EAS; do NOT re-port-scan here. The units you enumerate (endpoints, params)
become the coverage denominator for `vuln_triage`.

**Recommended sequence (only on live web services from EAS):**

1. Load the stage-local worklist — start with `stage_worklist_status`. If
   `ready_to_submit=false`, call `stage_worklist_next(prefer=["pending","error"])`
   and treat its `items` as the exact plan for this loop. Each item is one
   asset×technique cell with a `work_item_id`, `target_id`, `asset`,
   `technique`, `state`, and `suggested_tools`. Work only the named cells, then
   re-query the worklist after tools land DB truth; do not choose from a full
   target list when the worklist already named gaps.
2. Load EAS-confirmed web-root context only when needed — use
   `list_enumeration_web_roots(include_coverage=true)` or `query_target_data` for
   details. Keep the `target_id`, resolved root URL, and org boundary attached to
   every downstream action. Do not enumerate passive candidates that EAS did not
   prove live.
3. Browser baseline + JS/API — run `browser_collect_js_api` first on each live
   web root with the unified standard strategy, then one bounded recipe pass only
   when closure is partial and the recipe has real work. After JS
   files are saved, run `js_extract_apis`; it persists deterministic endpoints
   and returns redacted secret/config/framework candidates plus rule-based
   `rule_matches` and `ai_analysis` line ranges for targeted model review.
   Treat rule matches as candidates only; do not invent endpoints from AI
   inference.
4. Seed normalization — merge browser requests, JS endpoints, crawler URLs, HTML
   links/forms, and well-known docs paths into scoped same-origin path seeds.
   Normalize host/scheme/port, route templates, query parameter names, trailing
   slashes, and static asset noise before probing.
5. Recursive route probe — after JS/API rows have landed, run
   `route_probe_paths` once per live web root. Pass `target_id` + `base_url`;
   observed paths are optional because the tool reads target-bound
   `api_endpoints` and existing `directory_entries` from DB by default. It
   uses `wordlist_path`, workspace `1.txt`, or the built-in fallback wordlist
   and loads the full de-duplicated list.
   The tool probes parent prefixes, recursively expands verified wordlist
   directory hits, verifies positive status responses against a per-prefix
   random baseline, and rejects soft-404 / uniform error pages into
   `rejected_candidates`; do not promote rejected candidates by hand. Store
   verified positives as absolute `directory_entries` with `target_id`; store
   empty/blocked/error states as explicit evidence-backed terminal coverage when
   DB truth cannot derive them. `queue_completed=true` means the generated and
   recursive queue drained.
6. Do not use external directory tools (`ffuf`, `gobuster`, `feroxbuster`,
   `dirb`, `dirsearch`) in enumeration. If DIR coverage is incomplete, inspect
   `seed_paths`, `wordlist`, `rejected_candidates`, errors, and
   `queue_completed`; rerun only when the previous run errored, did not complete
   its queue, or received materially new DB seeds.
7. Parameter discovery — derive parameters from observed browser requests,
   crawler URLs with query strings, HTML forms, and targeted `js_extract_apis`
   `param_hints` after reviewing saved JS. Persist parameter names into
   `api_endpoints.params`; do not default to active hidden-parameter brute-force.
8. Slim submit — call `stage_worklist_status` and `check_stage_asset_coverage`.
   If either says `ready_to_submit=false`, call `stage_worklist_next` again and
   close only the named cells. Wait for background jobs, sanity-check the
   DB-visible content units, then submit only summary claims plus
   checked_empty/blocked/not_applicable coverage that the database cannot derive.

**Efficiency red lines:**

- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- Route probe once per service after JS/API landing; let it read DB seeds and
  run the full local/built-in wordlist and recursive queue.
- Do not call external directory tools in enumeration, and don't loop swapping
  wordlists endlessly.

**Coverage + stop condition (denominator matters):**

- Per in-scope asset, give GOLISH-ENUM-JS / GOLISH-ENUM-DIR / GOLISH-ENUM-PARAM /
  GOLISH-ENUM-JSAPI a terminal status in `coverage`. GOLISH-ENUM-JS (JS asset
  collection) is satisfied when `browser_collect_js_api` lands JS into
  `js_analysis_results`; if a site truly serves no JS, record `checked_empty`
  with the browser-run evidence (a run that found 0 JS is checked-empty, not
  unchecked — I8). GOLISH-ENUM-JSAPI is now narrower: API endpoints extracted
  from JS/crawler via `js_extract_apis`.
- For found/checked_empty cells, set `tested_units` and `total_units` (M = the
  enumerated endpoints/params for that asset×technique). Full coverage needs
  `tested_units == total_units`; to sample a huge surface you MUST set
  `sampling_rationale` and meet the ratio, else the cell counts as partial and the
  gate BLOCKS. Testing 3/5000 endpoints then claiming checked_empty is false coverage.
- Before submitting, call `stage_worklist_status` and `check_stage_asset_coverage`.
  If `ready_to_submit=false`, use `stage_worklist_next.items` first and
  `gap_examples` as supporting context to enumerate the missing service/technique
  cells or record honest checked_empty/blocked/not_applicable terminal coverage.
  Submit only when the latest preflight says `ready_to_submit=true`. This is a
  required self-check before submit, not a trial submit: do not call
  `submit_stage_deliverable` just to discover missing cells.
- Do not hand-write `found` cells that DB truth can derive from
  `directory_entries` or `api_endpoints`. Submitted coverage is for
  checked_empty, blocked, or not_applicable terminal states that still need notes
  or evidence.
- The stage deliverable has `findings: []`. Summarize discoveries as claims with
  kinds like `web_root_enumerated`, `directories_discovered`,
  `api_endpoints_discovered`, `params_discovered`, and
  `js_candidates_reviewed`; each claim must cite real evidence ids. Do not use
  `record_finding` in enumeration.
