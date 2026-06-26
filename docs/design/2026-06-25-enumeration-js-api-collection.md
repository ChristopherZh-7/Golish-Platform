# Enumeration JS/API Collection

> Date: 2026-06-25
> Status: browser JS acquisition + endpoint persistence + deterministic JS signal/rule-match candidate analysis landed (2026-06-26); model-side classification tool / WebRoot materialization still deferred
> Related: `2026-06-24-intel-to-eas-handoff.md`, `2026-06-23-technique-outcomes-provenance.md`, `2026-06-23-source-query-log.md`, `resources/harness/stages/enumeration/spec.json`
> Invariants: AGENTS.md I7 evidence-backed stage delivery, I8 checked_empty != unchecked, I9 no external long-running work inside transactions, deterministic gate remains final judge

## 1. Problem

The `enumeration` stage is supposed to turn EAS-confirmed live web services into concrete testable units for `vuln_triage`: directories, parameters, and JS/API endpoints.

The JS/API slice is currently under-specified and partially disconnected:

1. `js_collect` is a strong static collector, but it is still a `reqwest` HTML/manifest/chunk scanner. It does not execute a browser, click menus, follow SPA routes, scroll, wait for lazy chunks, or observe runtime XHR/fetch traffic.
2. `js_extract_apis` persists extracted endpoints into `js_analysis_results`, but `coverage_truth` marks `GOLISH-ENUM-JSAPI` found from `api_endpoints.source IN ('js_analysis','crawler')`. The current JS path therefore does not reliably satisfy the gate's DB truth projection.
3. The `browser` sub-agent is named like a browser specialist, but its tools are still `js_collect`, `web_fetch`, `web_search`, file tools, and `record_finding`. It has no DOM interaction or network-listener tool.
4. `enumeration` should not crawl every passive-intel asset. It should consume the concrete web entries EAS proved reachable. Today the common tools expose target rows with `value`, `http_status`, `ports`, etc., but there is no first-class `confirmed_web_root` contract for `scheme + host + port + final_url + redirect_chain`.

The user-facing goal is not "run a bigger crawler". The goal is:

```text
For each EAS-confirmed live web root:
  load the real page like a browser,
  discover lazy JS and runtime APIs,
  extract API endpoints and sensitive candidates,
  let AI classify and de-noise,
  persist verified facts so the gate and vuln stages can consume them.
```

## 2. Stage Boundary

JS/API collection belongs to `enumeration`, not `external_attack_surface`.

```text
target_intel
  -> discovers candidate org assets without touching targets

external_attack_surface
  -> proves which entries are live web services
  -> records liveness, scheme/port/fingerprint/redirect evidence
  -> DOES NOT fetch or analyze JS

enumeration
  -> consumes EAS-confirmed web roots
  -> collects JS/API/params/dirs as content enumeration
  -> persists testable units for vuln_triage
```

This keeps EAS fast and bounded. EAS is the "which doors exist?" stage; enumeration is the "what content and APIs are behind each door?" stage.

## 3. Goals

1. Keep JS/API collection inside `enumeration`.
2. Run JS/API collection only against EAS-confirmed live web roots.
3. Preserve the existing static JS collector because its manifest/chunk-map logic is valuable.
4. Add a browser-driven collector for lazy chunks, SPA routes, DOM-triggered network traffic, XHR/fetch, GraphQL, JSON, WebSocket URLs, source maps, and runtime-loaded JS.
5. Make AI responsible for navigation strategy and semantic classification, not for gate truth.
6. Persist endpoint facts into `api_endpoints` and JS analysis facts into `js_analysis_results`.
7. Persist per-root coverage outcome via evidence-backed DB facts, preferably `technique_outcomes`.
8. Keep collection non-exploitative: no brute force, no destructive clicks, no vulnerability scanners, no credential stuffing.

## 4. Non-Goals

- Directory scanning redesign. Sensitive path / directory probing is a later design.
- Authenticated exploration beyond existing user-provided session/cookies. Login handling can be added later through the credential capture engine.
- Vulnerability testing. `auth_probe`, IDOR checks, XSS/SQLi scans, and exploit validation remain downstream.
- Model-based PASS/BLOCK. AI can advise and classify, but the gate only accepts persisted evidence-backed facts.
- Full schema rewrite in the first slice. P0 should work with current `targets`, `target_assets`, `js_analysis_results`, `api_endpoints`, audit/evidence, and `technique_outcomes`.

## 5. Terminology

| Term | Meaning |
|---|---|
| WebRoot | One concrete EAS-confirmed entry to load: scheme, host, port, root URL, final URL, status, redirect chain, target id, org id, evidence ids. |
| Static collector | Existing `js_collect`: HTML scripts, inline scripts, build manifests, recursive JS reference extraction. |
| Dynamic collector | New browser-backed tool that executes the page and records runtime network/DOM observations. |
| Endpoint candidate | A possible API endpoint from JS static analysis, browser network capture, crawler output, or AI proposal. |
| Verified endpoint | An endpoint whose existence came from real JS/network/crawler/HTTP evidence and was persisted to `api_endpoints`. |
| AI navigation plan | Bounded list of safe page interactions or route visits proposed by the model. |
| AI classification | Model judgment over candidate endpoints/secrets: likely real API, noise, sensitive, auth-required, IDOR candidate, etc. |

## 6. Current Implementation Summary

### 6.1 Existing static JS collector

`JsCollectTool` currently does four static discovery passes:

1. HTML `<script src>`.
2. Inline `<script>` bodies for webpack runtime/chunk maps/public path.
3. Well-known manifests such as `asset-manifest.json`, Vite manifests, and Next build manifests.
4. Recursive scan inside downloaded JS for more JS references.

It downloads with bounded concurrency and a hard file cap, saves files under:

```text
.golish/captures/{host}/{port}/js/
```

It also records `js_collected` audit metadata and merges saved JS into the SiteMap store.

This is a good static foundation. It should remain.

### 6.2 Existing JS endpoint extraction

`js_extract_apis` reads the JS capture directory and runs `golish-js-analyzer`, which extracts `fetch`, `axios`, `$.ajax`, `new Request`, concat URLs, and template literal patterns. It stores aggregated results in `js_analysis_results`.

2026-06-25 implementation note: this gap is fixed for resolvable endpoints. The tool now also writes `api_endpoints(source='js_analysis')`, counts duplicate `(target_id,url,method)` rows without failing, and recursively scans nested `.js` / `.mjs` captures so Next/Vite-style chunk paths are not missed.

### 6.3 Existing enumeration prompt/spec

The enumerator prompt already says:

```text
js_collect -> js_extract_apis -> api_endpoints
```

but the actual tool persistence does not match that contract. Fixing this mismatch is the first implementation slice.

2026-06-25 implementation note: the prompt/tool contract now includes `browser_collect_js_api` for SPA/lazy-loaded pages, and the Enumerator/Browser tool allowlists expose it.

### 6.4 First dynamic browser collector

The first browser-backed collector is implemented as:

- Rust bridge tool: `BrowserCollectJsApiTool` / `browser_collect_js_api`.
- Node helper: `scripts/browser_collect_js_api.mjs`.
- Runtime behavior: open target URL with Playwright, fall back to system Chrome if bundled Chromium is missing, wait/scroll, perform a bounded number of safe non-submit clicks, save JavaScript responses into the existing captures tree, recursively parse loaded JS for nested chunk references (`import()`, literal paths, Vite `__vite__mapDeps`, Webpack/Next/Rspack runtime chunk maps), and record same-origin XHR/fetch requests.
- Closure behavior: `crawl_mode="fast"` is the first-pass default; `crawl_mode="deep"` increases the default recursive script limit, page/action budget, and hard timeout for sites with hundreds of nested chunks. Results now report `closure_complete`, `recursive_queue_remaining`, `recursive_limit_hit`, and `closure_incomplete_reasons`; a non-timeout incomplete crawl returns `status="closure_partial"` so the outer agent knows to retry with deep mode, bounded higher limits, or an AI recipe.
- AI fallback behavior: the helper can return `ai_assist.context` when deterministic collection is thin or anomalous. The model may propose a bounded second-pass `recipe` (`manifest_paths`, `script_urls`, `routes`, `click_texts`, `public_path` + `chunk_pairs`), but the helper still enforces same-origin/request limits and only persisted files/network observations count as facts.
- Persistence: observed XHR/fetch requests write to `api_endpoints(source='crawler')` when a `target_id` can be resolved or provided.
- Timeout behavior: browser collection is evidence-first and bounded. `hard_timeout_ms` caps the whole helper; individual response bodies, manifest/chunk fetches, pending response waits, and context/browser close all have smaller caps. If the cap is hit, the helper returns `status="timeout_partial"` with whatever JS/API evidence was already saved, plus timeout diagnostics for the outer agent. The helper exits explicitly after writing JSON so Playwright or system Chrome dangling handles cannot keep the Rust wrapper waiting.
- Noise control: default collection blocks images, fonts, media, stylesheets, and common analytics/tracker URLs so JS/API enumeration is not held hostage by non-evidence resources.
- Verification snapshot: `react.dev` saved 13-14 Next.js chunks depending on runtime route timing, observed 8 `_next/data/...` fetch requests, and separated non-existent recursive candidates into `recursive_errors`; `example.com` returned a clean empty result.
  A local recipe-only fixture with no page `<script>` successfully fetched `/manifest.json` and then saved `/assets/hidden-chunk.js` via the bounded recipe path.
  A follow-up timeout pass against `react.dev` with `hard_timeout_ms=10000` returned `timeout_partial` with 10 saved JS files and 7 observed fetch requests instead of hanging.
  A multi-site smoke pass (`react.dev`, `vite.dev`, `vuejs.org`, `docs.astro.build`, `svelte.dev/docs`) returned without external kill under `hard_timeout_ms=25000`; `svelte.dev/docs` saved 27 JS files, 2 recursive chunks, and 1 fetch request.
  A second smoke pass (`nextjs.org/docs`, `nuxt.com/docs`, `tailwindcss.com/docs/installation`, `angular.dev/overview`, `developer.mozilla.org/.../JavaScript`, `nodejs.org/en/learn`) exposed a close-time dangling-handle case on MDN; after the explicit-exit fix, MDN returned in 18.9s with 30 saved JS files and 2 runtime fetch requests.
  A closure-mode comparison showed `angular.dev/overview` fast mode saved 47 JS files and returned `closure_partial`; deep mode with `max_recursive_scripts=120` saved 165 JS files. MDN fast mode saved 30 JS files with 448 queued candidates; deep mode saved 42 JS files before a 90s hard deadline, correctly returning `timeout_partial` instead of claiming closure. A local nested fixture (`main.js -> chunk-1.js -> ... -> chunk-8.js`) returned `closure_complete=true` and captured the final `/api/final` fetch.
  A follow-up deep smoke found `tailwindcss.com/docs/installation` closed cleanly (`status=ok`, 18 JS, 117-194 runtime requests depending on run timing), while `nextjs.org/docs`, `nuxt.com/docs`, and `svelte.dev/docs` returned partial states with explicit queue/deadline/body-timeout diagnostics rather than hanging or pretending to be complete.

2026-06-26 implementation note: the "JS files are already landed" analysis step now has a deterministic first layer. `golish-js-analyzer` exposes `analyze_signals_from_files`, which extracts redacted secret/sensitive candidates (JWT/API key/token/private key/internal URL), runtime config/API base candidates, common framework/library signals, and curated rule-based signal hits with `source_file + line + preview + sha256`. `js_extract_apis` persists those into `js_analysis_results.frameworks/libraries/secrets_found/raw_analysis`, returns `rule_matches`, and returns an `ai_analysis` object with candidate files and suggested line ranges for targeted `read_file` review. The rule preset lives in `resources/js-analysis/js-signal-rules.yml`; the model review guidance lives in `resources/skills/js_extract_apis/rule-assisted-analysis.md`. This is not yet a model-calling classifier; the outer agent consumes the structured hints.

Not yet implemented in this slice: AI navigation plan, endpoint de-noising/classification as a separate model-backed tool, and dedicated secrets table/report UI.

## 7. Proposed Architecture

```text
Enumerator
  -> list_enumeration_web_roots
      -> DB-derived EAS-confirmed roots
  -> for each WebRoot:
      -> browser_collect_js_api
          -> static js_collect pass
          -> dynamic Playwright/Crawlee pass
          -> runtime network capture
          -> saved JS + request/response evidence
      -> js_extract_apis
          -> writes js_analysis_results
          -> writes api_endpoints(source='js_analysis')
      -> api_analyze_candidates
          -> AI de-noise / classify / prioritize
          -> updates endpoint metadata / raw analysis
      -> technique_outcomes upsert:
          GOLISH-ENUM-JSAPI found | empty | error | blocked
  -> submit_stage_deliverable
      -> deterministic gate reads DB/evidence
```

The model may call tools and provide classification, but it should not hand-write `found` coverage cells. Found/empty/error must come from the tool layer and DB projection.

## 8. WebRoot Input Contract

### 8.1 Desired shape

```json
{
  "web_root_id": "stable string or uuid",
  "target_id": "uuid",
  "organization_id": "uuid",
  "root_url": "https://app.example.com:8443/",
  "scheme": "https",
  "host": "app.example.com",
  "port": 8443,
  "status": 200,
  "final_url": "https://app.example.com:8443/app/",
  "redirect_chain": [
    "http://app.example.com/",
    "https://app.example.com/app/"
  ],
  "title": "Example Console",
  "fingerprints": ["nginx", "React"],
  "evidence_ids": [123],
  "source": "external_attack_surface"
}
```

### 8.2 P0 derivation

P0 should avoid a migration. Add a deterministic helper/tool:

```text
list_enumeration_web_roots
```

It derives roots from the existing EAS-visible rows:

- `targets.value` when `target_type=url`;
- `targets.http_status` and `content_type` when the row is already known web-capable;
- `targets.ports` entries whose service looks web-like (`http`, `https`, `http-alt`, `ssl/http`, webserver hints);
- `fingerprints` with webserver/technology evidence;
- redirects and final URLs if they are already present in audit/tool output.

If exact scheme/final URL is missing, the tool should return a root with `confidence` and a `needs_probe` note rather than forcing the agent to guess.

### 8.3 P1 materialization

Once stable, EAS should materialize confirmed roots as `target_assets` rows:

```text
asset_type = "web_root"
value      = final_url or root_url
protocol   = http|https
port       = port
service    = web/http
metadata   = {status, final_url, redirect_chain, title, fingerprints, evidence_ids}
```

This reuses the existing `target_assets` table and avoids a new `web_roots` table unless later reporting/query performance proves a dedicated table is worth it.

## 9. Dynamic Collection Tool

### 9.1 Tool name

Preferred:

```text
browser_collect_js_api
```

Alternative:

```text
js_collect_dynamic
```

`browser_collect_js_api` is clearer because the output is not only JS files; it includes runtime API requests and sensitive candidates.

### 9.2 Engine choice

Use a Playwright-backed Node helper for the first slice.

Reasons:

- Playwright provides stable network events, request/response inspection, DOM actions, route navigation, screenshots if needed, and browser-like behavior.
- Rust-native browser automation options are thinner and would add more risk.
- Golish already has a frontend/tooling ecosystem where a Node helper is acceptable if wrapped by a Rust `Tool`.

Crawlee can be evaluated as a later wrapper if queue management becomes complex. P0 should use Playwright directly for predictable control.

### 9.3 Inputs

```json
{
  "web_root": { "...": "WebRoot" },
  "target_id": "uuid",
  "max_pages": 20,
  "max_actions": 40,
  "max_duration_seconds": 120,
  "same_origin": true,
  "ignore_https_errors": true,
  "include_source_maps": true,
  "ai_actions": [
    {"kind": "click", "selector_hint": "text=API Docs"},
    {"kind": "goto", "url": "https://app.example.com/#/settings"}
  ]
}
```

### 9.4 Deterministic actions

Before involving AI, the tool should do a safe baseline:

1. Load `root_url` or `final_url`.
2. Wait for DOM content and bounded network idle.
3. Record every request and response metadata.
4. Save JS response bodies.
5. Scroll top-to-bottom once.
6. Visit same-origin anchors and router links up to `max_pages`.
7. Click safe navigation-like elements: tabs, menus, accordions, sidebars, "more", "settings", "docs", "dashboard".
8. Avoid destructive/action elements: delete, submit, save, pay, upload, send, logout, reset, deploy, run.

### 9.5 AI-guided actions

AI should get a sanitized page summary:

- visible text snippets;
- anchor hrefs;
- buttons and ARIA labels;
- route-like strings from JS/DOM;
- already observed request paths;
- blocked/noisy actions.

AI returns a bounded navigation plan. The tool validates the plan before execution:

- same origin unless explicitly allowed;
- no form submission by default;
- no destructive labels;
- max actions/pages/time enforced;
- no credential brute force or payload injection.

### 9.6 Outputs

```json
{
  "status": "ok|empty|partial|blocked|error",
  "web_root": "...",
  "js_files": [
    {"url": "...", "path": ".golish/captures/.../app.js", "size": 12345, "sha256": "..."}
  ],
  "runtime_requests": [
    {"method": "GET", "url": ".../api/me", "resource_type": "xhr", "status": 200}
  ],
  "endpoint_candidates": [
    {"method": "GET", "url": ".../api/me", "source": "browser_network", "confidence": 1.0}
  ],
  "secret_candidates": [
    {"kind": "token|api_key|credential|config", "source": "js", "confidence": 0.72, "preview": "..."}
  ],
  "navigation_trace": [
    {"action": "goto", "url": "...", "requests_seen": 12}
  ],
  "evidence_ids": [123],
  "quality_warnings": []
}
```

## 10. Endpoint Persistence Contract

### 10.1 Sources

Use `api_endpoints.source` consistently:

| Source | Meaning | Counts for JSAPI found |
|---|---|---|
| `js_analysis` | Endpoint extracted from captured JS. | yes |
| `crawler` | Endpoint observed by katana/browser network/runtime crawl. | yes |
| `proxy` | Future proxy/capture ingestion. | likely yes after verified |
| `ai` | AI-proposed candidate not yet verified. | no |
| `manual` | User/manual entry. | no by default |

P0 should not expand `coverage_truth` to count raw `ai` source. AI proposals must be verified by browser/network/HTTP evidence before they count.

### 10.2 `js_extract_apis` writer

`js_extract_apis` should, when not dry-run:

1. Continue writing grouped rows to `js_analysis_results`.
2. For each extracted endpoint, resolve it against the web root / target URL.
3. Insert into `api_endpoints` with:
   - `source='js_analysis'`;
   - `method`;
   - absolute `url`;
   - normalized `path`;
   - `params` from query/path/template hints when available;
   - `auth_type` from `AuthHint`;
   - `risk_level='unknown'` initially;
   - `capture_path` when the JS file path is known.
4. Upsert idempotently on `(target_id, url, method)`.

This closes the current mismatch between prompt/spec and DB truth.

### 10.3 Browser network writer

`browser_collect_js_api` should write runtime-observed API requests directly to `api_endpoints` with `source='crawler'`.

Filtering rules:

- Include XHR/fetch, GraphQL, JSON, WebSocket URL, and same-origin API-like requests.
- Exclude static assets by default: `.js`, `.css`, images, fonts, maps, analytics beacons unless configured.
- Keep cross-origin requests only when they are clearly first-party API/CDN or when `same_origin=false`.
- Normalize query parameter names into `params`.

## 11. Sensitive Candidate Handling

Enumeration should collect sensitive candidates but not immediately turn them into vulnerability findings.

Store them as:

- `js_analysis_results.secrets_found`;
- audit/evidence payload on the collection run;
- later, optionally a dedicated secrets table.

2026-06-26 implementation note: P0 storage uses existing `js_analysis_results`.
Secret values are not stored verbatim by the analyzer output; candidates carry
`value_preview`, `value_sha256`, `source_file`, `line`, `confidence`,
`is_likely_test_value`, and a redacted context line. Runtime config candidates
are stored in `raw_analysis.config_candidates`, while frameworks/libraries use
their existing JSONB columns.

AI classification should produce:

```json
{
  "kind": "api_key|jwt|bearer_token|private_key|cloud_key|internal_url|config",
  "confidence": 0.0,
  "is_likely_test_value": false,
  "reason": "short explanation",
  "recommended_next_stage": "vuln_triage|reporting|ignore"
}
```

Do not emit findings in `enumeration`. The existing stage spec has `findings_allowed=false`.

The P0 model handoff is the `ai_analysis` object returned by `js_extract_apis`.
When `ai_analysis.recommended=true`, the agent should read only the suggested
line ranges and classify candidates as real/test/noise/needs_followup. Full
bundle reading is not the default path. `rule_matches` are first-pass candidates:
the `rule_name` / `source_rule` explains why a line is suspicious, but proof
still comes from local source context and deterministic tool evidence.

## 12. AI Analysis Contract

AI is useful in two places:

1. Navigation: decide which safe routes/clicks are likely to load more JS/API.
2. Semantic analysis: classify endpoint/secret candidates.

2026-06-26 status: semantic analysis now has a deterministic candidate feed and
line-range handoff, including curated rule-based signal rules inspired by
gh0stkey/HaE, but no in-tool model invocation. The model intervention is currently the outer
enumerator/browser agent reading `ai_analysis`, using `read_file` for local
context, and deciding whether candidates need later triage.

AI must not:

- mark `GOLISH-ENUM-JSAPI` found without a persisted endpoint/evidence row;
- claim checked_empty without a tool outcome proving a real collection attempt;
- invent endpoints as verified facts;
- convert secrets into findings during enumeration;
- run exploit or injection payloads.

AI may propose candidate endpoints. Those candidates must be stored as `source='ai'` or kept in raw analysis until a deterministic browser/HTTP probe confirms them.

## 13. Coverage Semantics

For each WebRoot, `GOLISH-ENUM-JSAPI` must end in one of:

| Status | Source of truth |
|---|---|
| `found` | `api_endpoints` contains fresh `js_analysis` or `crawler` rows for the target/web root, or `technique_outcomes.outcome='found'` with evidence. |
| `checked_empty` | Browser/static collection ran, no JS/API endpoints were found, and a real empty outcome was recorded with evidence. |
| `blocked` | Collection could not run due to auth wall, transport error, WAF, cert failure, browser failure, or budget exhaustion, with evidence/note. |
| `not_applicable` | The EAS-confirmed target is not actually web-content-capable for JS/API purposes, with note/evidence. |

P0 should use `technique_outcomes` to represent empty/error outcomes because `coverage_truth` only projects found facts from business tables. This prevents "no row" from being mistaken for checked_empty.

## 14. Integration With Enumeration

Enumerator methodology should become:

1. Call `list_enumeration_web_roots`.
2. For each WebRoot:
   - run `browser_collect_js_api`;
   - run/merge `js_collect` static collection if the browser collector did not already run it internally;
   - run `js_extract_apis`;
   - run `api_analyze_candidates` if candidates exist or if quality warnings indicate ambiguity.
3. Wait for background jobs if used.
4. Submit deliverable with only DB-unavailable terminal cells. Do not hand-write found cells already backed by DB truth.

`enumerator` and `browser` tool lists should include the new collection/analysis tools. If `discover_apis` remains as a manual persistence tool, add it to both lists; however, the preferred implementation is direct persistence inside collection/extraction tools so the agent cannot forget to save.

## 15. Phased Implementation

### P0 - Close the existing JSAPI persistence gap

- `js_extract_apis` writes extracted endpoints to `api_endpoints(source='js_analysis')`.
- Add tests proving the endpoint rows satisfy `coverage_truth` JSAPI projection.
- Update enumerator prompt/spec comments so they match the actual behavior.
- Add `discover_apis` to enumerator/browser only if direct persistence is not implemented in the same slice.

### P1 - First-class WebRoot listing

- Add `list_enumeration_web_roots`.
- Derive roots from existing EAS target/ports/fingerprint fields.
- Include evidence ids and confidence/needs_probe diagnostics.
- Keep JS collection in enumeration; do not move it to EAS.

### P2 - Browser dynamic collector

- Add Playwright-backed `browser_collect_js_api` Rust tool wrapper + Node helper.
- Enforce budgets: max roots, pages, actions, duration, response bytes.
- Save JS bodies and runtime request summaries.
- Write runtime API rows to `api_endpoints(source='crawler')`.
- Record collection run audit/evidence.

### P3 - AI navigation and classification

- Add sanitized DOM/network summary for AI.
- Allow guided safe action plans.
- Add deterministic JS signal extraction for secrets/config/framework/library candidates and curated rule-based first-pass regex hits. **Done 2026-06-26 as P3a** via `analyze_signals_from_files` + `js_extract_apis.ai_analysis`.
- Add `api_analyze_candidates` for model-backed de-noise/classification. **Deferred**.
- Persist classification to endpoint notes/risk/auth metadata and `js_analysis_results.raw_analysis`.

### P4 - Coverage hardening

- Upsert `technique_outcomes` for `GOLISH-ENUM-JSAPI` found/empty/error/blocked per WebRoot.
- Gate consumes found from `api_endpoints`, empty/error from `technique_outcomes`.
- Only after empty facts are reliable, consider making enumeration found/empty more authoritative.

### P5 - Later, directory/sensitive path scanning

Directory scanning is intentionally separate. It should use JS/API/browser discoveries as seed hints instead of full blind wordlists.

## 16. Verification Strategy

1. Unit tests:
   - `js_extract_apis` resolves relative/template endpoints and inserts `api_endpoints`;
   - duplicate `(target_id, url, method)` does not create duplicates;
   - AI source candidates do not count as JSAPI found;
   - WebRoot derivation handles URL/domain/IP/ports without guessing unsafe schemes.
2. Fixture dynamic site:
   - initial HTML loads one entry JS;
   - route click loads lazy chunk;
   - lazy chunk triggers `/api/me`;
   - source map exposes additional route string;
   - collector must capture lazy JS and runtime API.
3. Harness tests:
   - enumeration JSAPI passes when endpoint rows land;
   - checked_empty requires a real empty `technique_outcomes` fact;
   - no endpoint/no outcome remains not_attempted and blocks.
4. Live E2E:
   - run `--stage-run --only enumeration` after a known EAS run;
   - verify `.golish/captures/.../js`, `js_analysis_results`, `api_endpoints`, `technique_outcomes`, and `run_tree.py --db`.

## 17. Risks And Controls

| Risk | Control |
|---|---|
| Browser collector becomes too heavy | Per-root budgets, queue caps, same-origin default, background jobs. |
| Destructive clicks | Safe action allowlist, destructive-label denylist, no form submit by default. |
| WAF/anti-bot noise | Browser-like UA, low concurrency, record blocked/error rather than looping. |
| Login walls | Mark blocked/checked_empty with evidence; authenticated capture is later. |
| AI hallucinated endpoints | Store as `source='ai'` only; do not count for coverage until verified. |
| Secret exposure in logs | Store previews/hashes by default; avoid dumping full secret values into prompts. |
| Cross-org leakage | WebRoot listing and endpoint writes must preserve `organization_id`, `target_id`, and `project_path` scoping. |
| Tool dependency drift | Playwright helper has versioned package/dependency check and fallback to static `js_collect`. |

## 18. Open Questions

1. Should dynamic collection live as a Node helper under `frontend` tooling, a backend-owned helper under `backend/tools`, or a separately packaged tool?
2. Should `target_assets(asset_type='web_root')` become the long-term WebRoot materialization, or do we eventually want a dedicated `web_roots` table?
3. How much authenticated browser state should enumeration consume from the credential capture engine in a later phase?
4. Should source maps be fetched by default or only when same-origin and below a size cap?
5. How should sensitive candidates be rendered in UI without leaking full values?

## 19. Rollback

- P0 direct endpoint persistence can be disabled by leaving `js_extract_apis` in `js_analysis_results`-only mode.
- P1 WebRoot listing is read-only and additive.
- P2 dynamic collection can be feature-flagged and fall back to static `js_collect`.
- P3 AI classification can be disabled without breaking persisted endpoint facts.
- P4 `technique_outcomes` projection is additive dual-read; if disabled, the gate falls back to existing DB truth and submitted coverage cells.
