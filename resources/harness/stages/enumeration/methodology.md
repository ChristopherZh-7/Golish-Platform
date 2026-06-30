**Goal:** enumerate CONTENT on the services EAS already mapped — JavaScript/API
endpoints, directories/paths, and parameters. Port/service discovery is already
done in EAS; do NOT re-port-scan here. The units you enumerate (endpoints, params)
become the coverage denominator for `vuln_triage`.

**Recommended sequence (only on live web services from EAS):**

1. Load EAS-confirmed web roots — start from `list_in_scope_targets` or a richer
   web-root section/tool when available. Keep the `target_id`, resolved root URL,
   and org boundary attached to every downstream action. Do not enumerate passive
   candidates that EAS did not prove live.
2. Browser baseline + JS/API — run `browser_collect_js_api` first on each live
   web root with the unified standard strategy, then one bounded recipe pass only
   when closure is partial and the recipe has real work. After JS
   files are saved, run `js_extract_apis`; it persists deterministic endpoints
   and returns redacted secret/config/framework candidates plus rule-based
   `rule_matches` and `ai_analysis` line ranges for targeted model review.
   Treat rule matches as candidates only; do not invent endpoints from AI
   inference.
3. Seed normalization — merge browser requests, JS endpoints, crawler URLs, HTML
   links/forms, and well-known docs paths into scoped same-origin path seeds.
   Normalize host/scheme/port, route templates, query parameter names, trailing
   slashes, and static asset noise before probing.
4. Lightweight recursive route probe — run `route_probe_paths` over observed
   JS/API path prefixes, using the small local wordlist when available
   (`wordlist_path` or workspace `1.txt`). It de-duplicates observed paths,
   probes each parent prefix, and can recurse one bounded level from positive
   wordlist hits. It verifies positive status responses against a per-prefix
   random baseline and rejects soft-404 / uniform error pages into
   `rejected_candidates`; do not promote rejected candidates by hand. Store
   verified positives as absolute `directory_entries` with `target_id`; store
   empty/blocked/error states as explicit evidence-backed terminal coverage when
   DB truth cannot derive them.
5. Do not use external directory tools (`ffuf`, `gobuster`, `feroxbuster`,
   `dirb`, `dirsearch`) in enumeration. If DIR coverage is incomplete, refine
   observed_paths / the small wordlist and rerun `route_probe_paths` within the
   bounded request budget.
6. Parameter discovery — derive parameters from observed browser requests,
   crawler URLs with query strings, HTML forms, and targeted `js_extract_apis`
   `param_hints` after reviewing saved JS. Persist parameter names into
   `api_endpoints.params`; do not default to active hidden-parameter brute-force.
7. Slim submit — call `check_stage_asset_coverage` and treat its `gap_examples`
   as the current EAS-confirmed web-root worklist. Wait for background jobs,
   sanity-check the DB-visible content units, then submit only summary claims
   plus checked_empty/blocked/not_applicable coverage that the database cannot
   derive.

**Efficiency red lines:**

- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- Route probe with observed JS/API prefixes and the small local wordlist; do not
  call external directory tools in enumeration.
- One sensible wordlist pass per service or prefix; don't loop swapping wordlists
  endlessly.

**Coverage + stop condition (denominator matters):**

- Per in-scope asset, give GOLISH-ENUM-DIR / GOLISH-ENUM-PARAM / GOLISH-ENUM-JSAPI
  a terminal status in `coverage`.
- For found/checked_empty cells, set `tested_units` and `total_units` (M = the
  enumerated endpoints/params for that asset×technique). Full coverage needs
  `tested_units == total_units`; to sample a huge surface you MUST set
  `sampling_rationale` and meet the ratio, else the cell counts as partial and the
  gate BLOCKS. Testing 3/5000 endpoints then claiming checked_empty is false coverage.
- Before submitting, call `check_stage_asset_coverage`. If
  `ready_to_submit=false`, use its `gap_examples` to enumerate the missing
  service/technique cells or record honest checked_empty/blocked/not_applicable
  terminal coverage. Submit only when the preflight says `ready_to_submit=true`.
  This is a required self-check before submit, not a trial submit: do not call
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
