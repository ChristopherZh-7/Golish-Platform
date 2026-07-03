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
3. Batch JS/API collection — run `browser_collect_js_api` in BATCH: pass
   `target_urls=[...]` with the whole EAS-confirmed live web-root set (up to 50)
   in ONE call, `crawl_mode="standard"`, `ai_assist=true`. It opens each page,
   triggers runtime chunks, saves loaded JS, and persists observed XHR/fetch.
   Each root lands its own GOLISH-ENUM-JS (JS assets) and GOLISH-ENUM-JSAPI
   (observed API) terminal coverage — a root that truly serves no JS lands
   `checked_empty`, not unchecked (I8). Only re-run a single root with a bounded
   recipe when it returned `closure_partial` / `timeout_partial` /
   `ai_assist.recommended=true`. Do NOT loop one URL at a time.
4. katana supplement (recommended) — run
   `pentest_run(tool_name="katana", args="-list <urls-file> -jc -silent ...")`
   ONCE over the same web-root list to harvest extra URLs/endpoints. katana
   output lands in `api_endpoints(source='crawler')` and MERGES/dedupes with
   browser + js_extract rows automatically (unique on `target_id`+`url`+`method`).
   katana is a SUPPLEMENT — it does not replace the browser closure crawl.
5. Batch API/param extraction — after JS is saved, run `js_extract_apis` in
   BATCH (`target_urls=[...]`) over the same set. It persists deterministic
   endpoints, folds observed params, and returns redacted
   secret/config/framework candidates plus rule-based `rule_matches` and
   `ai_analysis` line ranges for targeted model review. Treat rule matches as
   candidates only; do not invent endpoints from AI inference. Each root lands
   GOLISH-ENUM-JSAPI / GOLISH-ENUM-PARAM terminal coverage.
6. Batch route probe — run `route_probe_paths` in BATCH
   (`targets=[{target_id, base_url}, ...]`) over the same set. Each entry reads
   target-bound `api_endpoints`/`directory_entries` DB seeds, runs the full
   de-duplicated local/built-in wordlist and recursive queue, verifies positives
   against a per-prefix random baseline, rejects soft-404/uniform pages into
   `rejected_candidates` (do not promote by hand), and lands verified positives
   as absolute `directory_entries` + GOLISH-ENUM-DIR terminal coverage per root.
   `queue_completed=true` means the queue drained. Do NOT use external directory
   tools (`ffuf`, `gobuster`, `feroxbuster`, `dirb`, `dirsearch`); rerun a single
   root only when it errored, did not complete its queue, or got materially new
   DB seeds.
7. Parameter discovery — derive parameters from observed browser requests,
   crawler (browser + katana) URLs with query strings, HTML forms, and targeted
   `js_extract_apis` `param_hints` after reviewing saved JS. Persist parameter
   names into `api_endpoints.params`; do not default to active hidden-parameter
   brute-force.
8. Slim submit — call `stage_worklist_status` and `check_stage_asset_coverage`.
   If either says `ready_to_submit=false`, call `stage_worklist_next` again and
   close only the named cells. Wait for background jobs, sanity-check the
   DB-visible content units, then submit only summary claims plus
   checked_empty/blocked/not_applicable coverage that the database cannot derive.

**Efficiency red lines:**

- BATCH, don't loop: pass the whole web-root set via `target_urls` / `targets`
  in ONE call per tool. Do NOT fire one `browser_collect_js_api` /
  `js_extract_apis` / `route_probe_paths` per URL — that is the slow, token-
  burning pattern that used to stall the stage.
- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- Route probe once per service after JS/API landing; let it read DB seeds and
  run the full local/built-in wordlist and recursive queue.
- katana is a supplement (extra URL corpus), run once over the list; it does not
  replace the browser closure crawl.
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
- An IP/CIDR target that EAS/httpx proved serves HTTP (`targets.http_status` is
  set) is a web root even with no domain: enumerate it for all four content axes
  (JS/DIR/PARAM/JSAPI) exactly like a domain/URL. A bare IP with no HTTP evidence
  stays `not_applicable` for content enumeration — do NOT fuzz it.
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
