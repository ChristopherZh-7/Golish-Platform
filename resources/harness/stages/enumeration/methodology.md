**Goal:** enumerate CONTENT on the services EAS already mapped — JavaScript/API
endpoints, directories/paths, and parameters. Port/service discovery is already
done in EAS; do NOT re-port-scan here. The units you enumerate (endpoints, params)
become the coverage denominator for `vuln_triage`.

**Recommended sequence (only on live web services from EAS):**

1. Load the stage-local worklist — start with `stage_worklist_status`. If
   `ready_to_submit=false`, call
   `stage_worklist_next(prefer=["pending","error","partial"])` and treat its
   `items` as the exact plan for this loop. Each item is one
   asset×technique cell with a `work_item_id`, `target_id`, `asset`,
   `technique`, `state`, and `suggested_tools`. Work only the named cells, then
   re-query the worklist after tools land DB truth; do not choose from a full
   target list when the worklist already named gaps. `terminal_exceptions` is a
   deprecated compatibility input: omit it or pass `[]`; any non-empty array is
   rejected and cannot turn pending work into terminal truth.
2. Run the trusted transport preflight once for the distinct roots on this page:
   `enum_preflight_web_origins(origins=[{target_id,target_url}, ...])`. The
   backend performs fixed-timeout HEAD then only-if-needed GET Range requests,
   disables redirects, tries direct transport and the configured/environment
   proxy when present, and revalidates target ownership before and after I/O.
   Before I/O it atomically refreshes all four cells to non-terminal `partial`
   attempt markers, so a recovered origin cannot inherit stale `blocked`. Any
   HTTP response means reachable and writes no terminal coverage. Only when every
   available transport/TLS attempt fails does the backend append four fresh
   target-bound evidence rows and atomically publish JS/DIR/PARAM/JSAPI as
   `blocked`. Never reproduce this decision in deliverable prose. Remove trusted
   blocked roots from crawler/browser/JS/route inputs; continue normal producers
   for roots reported reachable or still pending/inconclusive.
3. Load EAS-confirmed web-root context — use
   `list_enumeration_web_roots(include_coverage=true)`. Each returned root already
   carries one exact `root_url` (`scheme://host:port/`, plus `scheme`/`port`), so
   feed those URLs straight to the content tools. Do NOT call `query_target_data`
   per target just to reconstruct a scheme/port to build the URL — the worklist
   already did that. Only drill into `query_target_data` for genuinely extra
   detail (e.g. a specific discovered API path). Keep the `target_id`, `root_url`,
   and org boundary attached to every downstream action. One target may yield
   multiple HTTP/HTTPS/non-default-port origins; treat each as a distinct row.
   Rootless alive hosts, unknown TCP services, closed/filtered ports, and any
   target whose scheme+port cannot be proved are not Enumeration work items at
   all; do not guess 80/443 or recreate them from a full target list. Do not
   enumerate passive candidates EAS did not prove as exact HTTP(S) origins.
4. URL seed discovery — run
   `enum_crawl_same_origin_urls(target_urls=[...])` ONCE over the same web-root
   list to harvest browser-worthy page routes, JS URLs, and endpoint hints. The wrapper owns the bounded Katana
   list-file recipe (`-list ... -jc -silent -dr -d N -cos <dangerous-route> -cs <exact-origin-union>`), which constrains actual requests to the authorized
   scheme+host+port set; do NOT call `katana` or `pentest_run` directly. Before
   writing the list, the wrapper replaces every accepted URL with its explicit-
   port exact-origin root `scheme://host:port/`: caller path/query/fragment is
   discarded, while the bound `target_id` remains attached to the returned
   `browser_seed`. The fixed `-cos` rejects dangerous path/query tokens in
   literal, percent-encoded, and double-percent-encoded forms before discovered
   links are requested; a broad `%25` marker deny also closes triple/deeper and
   mixed encodings. It uses the same Rust
   `ENUMERATION_DANGEROUS_ROUTE_TOKENS` as recipe sanitization and route probing,
   including builtin-wordlist actions such as shutdown/restart/refund/activate,
   so switching producer paths cannot weaken the read-only policy. The wrapper accepts only confirmed-open origins,
   disables redirects, and fixes Katana's rate, concurrency, parallelism,
   per-origin page, crawl-duration, and response-size bounds; do not try to
   weaken those limits. Treat Katana output as seed material, not the final
   collector result: use `browser_seed.target_urls` from the wrapper as the
   preferred next input to `browser_collect_js_api`. Third-party links are crawler
   context, not new targets.
5. Batch JS/API collection — run `browser_collect_js_api` in BATCH for the
   current worklist page, preferably with `target_urls=<enum_crawl_same_origin_urls.browser_seed.target_urls>`
   so each root receives its Katana-discovered `recipe.routes` / `recipe.script_urls`.
   If the crawler found no browser seed, fall back to the returned worklist roots.
   Use `crawl_mode="standard"`, `ai_assist=false` for deterministic broad capture.
   It opens each page plus seeded
   routes, triggers runtime chunks, saves loaded JS, and persists observed XHR/fetch.
   This is a strictly read-only browser pass: only GET/HEAD may leave the
   process, click actions are disabled, and dangerous/state-changing routes are
   denied. Write-shaped same-origin API attempts may appear as blocked
   observations, but are never sent. Foreign subresources that are successfully
   aborted before dispatch are reported as `scope_exclusions`; every WebSocket,
   including same-origin sockets, is recorded then closed before upgrade because
   Enumeration has no read-only message contract. These exclusions do not alone
   make the authorized crawl partial. A foreign top-level
   navigation, recursive-fetch escape, or failure to enforce exact origin is a
   scope violation and remains non-terminal.
   A complete root may terminally close only its own GOLISH-ENUM-JS axis. The
   browser still persists runtime API/parameter discoveries, but it deliberately
   leaves GOLISH-ENUM-JSAPI and GOLISH-ENUM-PARAM as `partial` with
   `static_extraction_pending=true`, even when runtime requests were found or the
   root served zero JS. Always run `js_extract_apis` over the same page/root set;
   only that exact-origin static pass may publish the final JSAPI/PARAM
   `found` / `empty` outcomes. A root that truly serves no JS is recorded as JS
   `empty` by the browser (I8), while the extractor independently proves the
   JSAPI/PARAM empty result. `closure_partial`,
   `timeout_partial`, or `closure_complete=false` keeps raw rows but writes
   `partial` for all three axes; re-run that root with a bounded recipe.
   Before any network/AI/cancellable work, the tool atomically replaces all
   three current-run cells with `partial` attempt markers; if that sibling write
   fails, collection does not start. At close it first prepares evidence and
   outcome writes for all three JS/JSAPI/PARAM siblings, then publishes the
   entire group in one DB transaction. If any prepare fails, all three publish
   as `partial`; if publication fails, the initial partial group remains, so a
   mixed terminal/partial result is never accepted. The collector obtains
   `operation_id` from trusted `AgentToolContext` and records it with run/session,
   `captured_at`, and current `producer_stage_started_at` in the manifest. The manifest is rebuilt only from scripts
   observed or successfully revalidated in this run. Old capture files may be
   cache candidates but cannot appear as fresh manifest evidence by themselves.
   Rust and Node both parse and double-decode path+query before accepting
   `recipe.manifest_paths`, `recipe.script_urls`, or `recipe.routes`; clicks are
   disabled. If a valid `%HH` escape remains after two passes, the route is
   rejected as deeper/mixed encoding. Node replaces valid `%HH` escapes one occurrence at a time and
   leaves malformed neighbors such as `%ZZ` untouched, so malformed text cannot
   suppress recognition of an adjacent single/double-encoded dangerous token.
   All direct manifest/recursive fetches share `fetchExactOrigin`,
   which checks exact origin and the dangerous-route policy before the initial
   request and before every manually followed redirect hop. This also covers
   auto-inferred manifests and recursive JS references, not only recipe inputs.
   Playwright navigation uses `maxRedirects=0`; every later hop returns through
   the same request guard before dispatch.
   When `max_pages` leaves a real page queue, re-run the same root in the same
   producer operation/stage-attempt/run/session: resume requires parseable
   `producer_stage_started_at == operation_state.stage_started_at` and
   `captured_at >= stage_started_at`, then restores persisted `pending_pages` and
   cumulative scripts/API observations. Resume is forbidden across operations,
   across stage attempts even under the same operation, across runs/sessions, or
   when any prior incomplete reason is not exactly `page_queue_remaining`; real
   link truncation stays partial.
   Each exact-origin helper also has a finite wall-clock deadline: default
   `hard_timeout_ms=120000`, bounded to 10000..600000; legacy `0` is normalized
   to the default rather than unlimited. This deadline covers response body
   reads, recursive fetches, browser protocol work, and close. The first bounded
   failure remains retryable `partial`/`error` and persists guarded recovery
   state. After a genuine same-operation/stage/run/session checkpoint resume,
   the backend may publish JS/JSAPI/PARAM as `blocked` only when a collection-
   blocking failure fingerprint has repeated at least twice, recovery is marked
   exhausted with automatic retry disabled, and evidence persistence itself is
   clean. API-body-only diagnostics, same-invocation duplicate errors, and any
   persistence failure remain non-terminal. Therefore a timeout never becomes
   checked-empty; when the tool returns a persisted `blocked` outcome with
   `recovery_exhausted=true`, do not retry those three cells. The optional
   internal AI recipe call is likewise bounded to 60000 ms by default and
   degrades to the last deterministic result. Pure `page_queue_remaining`
   checkpoints continue to resume under the same operation/stage/run/session.
   Do NOT loop one URL at a time.
6. Batch API/param extraction — after JS is saved, run `js_extract_apis` in
   BATCH (`target_urls=[...]`) over the same set. It persists deterministic
   endpoints, folds observed params, and returns redacted
   secret/config/framework candidates plus rule-based `rule_matches` and
   `ai_analysis` line ranges for targeted model review. Treat rule matches as
   candidates only; do not invent endpoints from AI inference. A complete root
   lands GOLISH-ENUM-JSAPI / GOLISH-ENUM-PARAM; any read error or skipped JS file
   keeps extracted rows but writes `partial` for both axes.
   `browser_collect_js_api.static_extraction_pending_targets > 0` is an explicit
   handoff, not an optional hint: feed every corresponding exact origin into
   this same-set batch before route probing, refreshing the worklist, or
   considering submit. Runtime-found API/params remain useful raw rows, but do
   not waive static extraction.
   Extraction atomically writes both attempt markers before reading files and
   reads only the current manifest's workspace-confined file allowlist; it must
   not recursively promote orphaned JS from an older capture. The extractor
   takes its operation id only from trusted `AgentToolContext`, loads that exact
   `operation_state`, and accepts the manifest only when the operation exists,
   is not superseded, is currently in `enumeration`, matches
   `producer_operation_id`, and the RFC3339 `captured_at` is not earlier than
   `operation_state.stage_started_at`; run/session must also match. Missing or
   stale provenance remains non-terminal. At close it prepares both JSAPI/PARAM
   sibling evidence/outcome writes before publishing them in one transaction;
   any prepare failure publishes both as `partial`, while a transaction failure
   leaves the initial partial pair authoritative. An unresolved endpoint
   candidate is incomplete, not an empty terminal result.
7. Batch route probe — run `route_probe_paths` in BATCH
   (`targets=[{target_id, base_url}, ...]`) over the same page, with explicit
   bounded concurrency such as `batch_concurrency=4` unless the worklist is
   tiny. For a completion run, omit both `max_runtime_ms` and `max_requests` so
   the declared queue can drain under the completion default and still return
   early when done; a 60-second runtime/request slice is only for an intentionally
   non-terminal diagnostic sample. In batch mode,
   `max_runtime_ms` is the per-target budget. Leave `batch_max_runtime_ms`
   omitted for completion so every bounded root is scheduled. If explicitly
   set, it is only a scheduling-start ceiling: roots not yet started are skipped,
   while already-started roots finish under their own per-target budget. Each entry reads
   target-bound `api_endpoints`/`directory_entries` DB seeds, runs the full
   de-duplicated local/built-in wordlist once at the exact-origin root, and runs
   observed-path/curated probes under the derived parent prefixes. The completion
   default is `wordlist_recursion_depth=0`: omit that argument so a positive
   directory does not repeat the full wordlist below itself. Explicit values
   `1..6` opt into bounded positive-hit recursion and may still end `partial` if
   candidate generation reaches its hard limit. The tool verifies positives
   against a per-prefix random baseline, rejects soft-404/uniform pages into
   `rejected_candidates` (do not promote by hand), and lands verified positives
   as absolute `directory_entries` + GOLISH-ENUM-DIR terminal coverage per root.
   `queue_completed=true` means the queue drained; `timeout_partial` /
   `request_limited_partial` / candidate-generation truncation preserve sampled
   directory rows but write a non-terminal DIR `partial`, so the same exact
   origin must be resumed. Checkpoint schema v8 carries per-candidate recovery
   counters, pending verified directory writes, and a zero-HTTP terminal-publication
   cursor under the exact operation/generation witness. The first network failure
   stays pending, while two stable failures with the same fingerprint or three
   total failures exhaust that candidate. Stable directory-write or terminal-publication
   failure also stops automatic retry after its bounded budget without minting DIR
   `blocked`; repair the reported persistence error, then retry only that root with
   the named `retry_exhausted_persistence` or `retry_exhausted_terminalization`
   manual flag. If the full
   queue then closes with at least one exhausted candidate and no persistence or
   verification incompleteness, the backend writes DIR `blocked` with
   `dir_probe_recovery_exhausted`; `recovery_exhausted=true` and a persisted
   blocked outcome mean no further automatic retry for that DIR cell. Do NOT use external directory
   tools (`ffuf`, `gobuster`, `feroxbuster`, `dirb`, `dirsearch`). Ordinary
   single-root retry is allowed only when `automatic_retry_allowed=true` (or the
   compact result says `retry.recommended=true`); `automatic_retry_allowed=false`
   always returns a `manual_repair_reason` and executable `recovery_action`; that
   action overrides error/queue heuristics until its stated repair or refresh is
   complete.
   Custom `wordlist_path` values must resolve to canonical regular files at
   `<workspace>/1.txt` or below `<workspace>/.golish/wordlists/`; arbitrary
   workspace source/config/secret files, absolute/external paths, parent
   traversal, symlinks, and symlink escapes are rejected. Dangerous/state-changing candidate paths are
   excluded before queueing. The DIR attempt is marked `partial` before work,
   and any wordlist-entry cap remains candidate-generation truncation rather
   than terminal coverage.
8. Parameter discovery — derive parameters from observed browser requests,
   crawler (browser + enum_crawl_same_origin_urls) URLs with query strings, HTML forms, and targeted
   `js_extract_apis` `param_hints` after reviewing saved JS. Persist parameter
   names into `api_endpoints.params`; do not default to active hidden-parameter
   brute-force.
9. Slim submit — call `stage_worklist_status` and `check_stage_asset_coverage`.
   If either says `ready_to_submit=false`, call `stage_worklist_next` again and
   close only the named cells. Once the newest previews say
   `ready_to_submit=true`, wait for background jobs, sanity-check the DB-visible
   content units, then submit summary claims, `findings: []`, and `coverage: []`.
   Fresh exact-origin producer evidence owns `found` / `empty`; trusted blocked
   evidence is limited to transport preflight for all four axes, route recovery
   exhaustion for DIR, and browser collection recovery exhaustion for
   JS/JSAPI/PARAM. Deterministic gate context owns `not_applicable`. Deliverable
   prose cannot duplicate or override any of them.

**Efficiency red lines:**

- BATCH, don't loop: pass the current worklist page via `target_urls` /
  `targets` in ONE call per tool, then refresh the worklist for the next page.
  `stage_worklist_next` returns at most 200 cells across at most 50 distinct
  exact-origin roots; deduplicate `items` by `asset` before building tool inputs
  and never send more than the returned 50 roots in one batch.
  Do NOT fire one `browser_collect_js_api` / `js_extract_apis` /
  `route_probe_paths` per URL, and do NOT request hundreds of roots for one
  model loop — both patterns stall the stage.
- Batch input is never silently truncated. Browser collection reports rejected,
  skipped, and over-cap rows in bounded `omissions`; an authorized browser
  over-cap root gets a current-run `partial` marker. JS extraction and route probe
  both reject any raw batch longer than 50 before a sibling performs target work
  or writes an attempt marker. Never interpret an absent per-root result as
  success; rejected over-limit extraction/probe inputs leave every root pending.
- Every active producer snapshots target id + organization + workspace + scope +
  exact origin before network I/O and revalidates that same binding before
  persistence. A moved/deleted/de-scoped target revokes the write; do not retry
  it under a guessed replacement target.
- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- Route probe once per service after JS/API landing; let it read DB seeds, run
  observed/curated parent-prefix probes, and run the local/built-in root wordlist
  once inside the foreground runtime budget. Omit `wordlist_recursion_depth` for
  the finite completion default; explicit `1..6` recursion is opt-in. A
  completion run must let its declared queue drain; request/candidate-generation
  limited samples remain `partial`.
- `enum_crawl_same_origin_urls` is a browser seed discovery pass, run once
  over the list before the browser closure crawl; its `browser_seed.target_urls`
  feeds Playwright routes/scripts, but Katana itself is not the final collector.
- Do not call external directory tools in enumeration, and don't loop swapping
  wordlists endlessly.

**Coverage + stop condition (denominator matters):**

- Per EAS-confirmed exact Web Origin, close GOLISH-ENUM-JS / GOLISH-ENUM-DIR /
  GOLISH-ENUM-PARAM / GOLISH-ENUM-JSAPI. The denominator is normalized
  `scheme://host:port × technique`, not host × technique. GOLISH-ENUM-JS (JS asset
  collection) is closed only by the current run's fresh exact-origin outcome from
  `browser_collect_js_api`; `js_analysis_results` remains discovery data. A
  complete JS-less run writes JS `empty` itself (I8), but still hands the same
  origin to `js_extract_apis`. GOLISH-ENUM-JSAPI is narrower: completion is owned
  by a fresh `js_extract_apis` outcome, not by browser runtime observation.
  GOLISH-ENUM-PARAM follows the same default closeout path; browser observations
  are retained as input, while current browser collection intentionally remains
  partial for PARAM until extraction finishes.
- An IP/CIDR target that EAS/httpx proved serves HTTP is a web root even with no
  domain, but it still enters Enumeration as its exact confirmed origin(s), not a
  guessed bare IP URL. Enumerate each for all four content axes. A bare IP with no HTTP evidence
  stays `not_applicable` for content enumeration — do NOT fuzz it.
- Before submitting, call `stage_worklist_status` and `check_stage_asset_coverage`.
  If `ready_to_submit=false`, use `stage_worklist_next.items` first and
  `gap_examples` as supporting context to enumerate the missing service/technique
  cells. For pending roots, run `enum_preflight_web_origins` before content
  producers. Transport preflight may block all four axes; after bounded producer
  recovery, `route_probe_paths` may block DIR only and `browser_collect_js_api`
  may block JS/JSAPI/PARAM only. A persisted producer `blocked` outcome with
  `recovery_exhausted=true` is terminal and must not be retried.
  Submit only when the latest preflight says `ready_to_submit=true`, with
  `coverage: []`. This is a
  required self-check before submit, not a trial submit: do not call
  `submit_stage_deliverable` just to discover missing cells.
- Do not hand-write `found`, `empty`, `blocked`, or `not_applicable`.
  `directory_entries`, `api_endpoints`, and
  `js_analysis_results` retain discovery truth, but only the current stage-run's
  fresh exact-origin `technique_outcomes` plus matching evidence close
  Enumeration cells. For `blocked`, the evidence producer/kind/axis must be one
  of: `enum_preflight_web_origins` + `enumeration_transport_blocked` on any of
  the four axes; `route_probe_paths` + `dir_probe_recovery_exhausted` on DIR;
  `browser_collect_js_api` + `enumeration_collection_recovery_exhausted` on
  JS/JSAPI/PARAM. `error` and `partial` remain unfinished and cannot produce a
  pass token. Trusted context alone supplies `not_applicable`.
- A terminal `found`/`empty`/`blocked` also requires real evidence from the same current
  session, organization, authorized target, exact-origin asset, technique, and
  outcome, created after the stage freshness cutoff. Evidence is appended to an
  atomically serialized per-operation hash chain; stale, foreign-org,
  target-less, mismatched, or failed evidence cannot be borrowed to close a cell.
- The stage deliverable has `findings: []`. Summarize discoveries as claims with
  kinds like `web_root_enumerated`, `directories_discovered`,
  `api_endpoints_discovered`, `params_discovered`, and
  `js_candidates_reviewed`. Do not invent evidence ids, and do not use
  `record_finding` in enumeration.
