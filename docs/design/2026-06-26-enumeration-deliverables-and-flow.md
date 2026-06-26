# Enumeration Deliverables And Flow

> Date: 2026-06-26
> Status: implementation started — Phase 0 contract alignment + Phase 1 `query_target_data` read-model slice + Phase 2 P0 `route_probe_paths` bridge and DIR `technique_outcomes` write landed 2026-06-26; JSAPI `technique_outcomes` write wired for both `js_extract_apis` and `browser_collect_js_api` (found/empty/error, browser timeout + helper-failure → error) 2026-06-26; Phase 3 partial — ffuf/gobuster `directory_entry_add` URL absolutization (relative token + command `-u` base → absolute URL → non-null `target_id`) landed 2026-06-26 so dictionary scans count for GOLISH-ENUM-DIR; AI-assisted JS param recipe landed 2026-06-26 — `js_extract_apis` takes `param_hints` (AI-read body/form params), merges them into matching api_endpoints via `api_endpoints_upsert_merge_params` (set-union, no query-param loss), and writes GOLISH-ENUM-PARAM found/empty/error only when a recipe ran; arjun/paramspider/x8 active-discovery PARAM outcome landed 2026-06-26 — `output_store::maybe_detect_and_store_via` records GOLISH-ENUM-PARAM found/empty (I8 checked_empty) for `subcategory==param` tools via `OutputStore::store_param_outcome`, keyed on `current_agent_session()` run id (foreground runs only; background jobs have no session); soft-404 filtering / YAML rule resources / arjun param-name landing (toolsconfig parses only count) / authoritative_found still pending
> Related: `resources/harness/stages/enumeration/spec.json`, `resources/harness/stages/enumeration/methodology.md`, `docs/design/2026-06-25-enumeration-js-api-collection.md`, `docs/design/2026-06-13-stage-run-fanout-design.md`, `docs/design/2026-06-15-db-truth-single-source-deliverable.md`
> Reference: [F6JO/RouteVulScan](https://github.com/F6JO/RouteVulScan) as a workflow reference, not as a dependency
> Invariants: AGENTS.md I7 evidence-backed stage delivery, I8 checked_empty != unchecked, I9 no external long work in transactions, findings do not belong to enumeration

## 0. One Sentence

`enumeration` should not be a one-click black-box scan. It should consume EAS-confirmed live web services, run a staged content-enumeration loop per web root, persist concrete testable units into DB truth tables, and submit only the small amount of coverage state the database cannot derive.

The stage-level contract should be:

```text
EAS proves live web services.
Enumeration turns each live web service into evidence-backed testable units:
  JS/API endpoints, directories/routes, parameters, forms/auth surfaces, and route-probe exposure candidates.
Vuln triage consumes those units and produces findings.
```

## 1. Why This Design Exists

The current stage boundaries are mostly right:

- `external_attack_surface` maps liveness, ports, and service fingerprints.
- `enumeration` has `specialist = "enumerator"` and coverage axis `DIR / PARAM / JSAPI`.
- JS/API collection was recently closed enough that browser-loaded JS and static JS analysis now persist into `api_endpoints`.

The missing piece is not "more tools". The missing piece is a deterministic deliverable model and a non-black-box execution order inside `enumeration`.

Today, the prompt says "browser JS/API, directory/path, parameter discovery", but the stage still behaves too much like a general one-shot content scan:

1. It does not define the exact deliverable buckets that feed `vuln_triage`.
2. It does not formalize "web roots from EAS" as the enumeration input contract.
3. It does not distinguish DB-derived found facts from manually submitted terminal exceptions.
4. It does not include the RouteVulScan-style recursive route/path probe between JS/API discovery and dictionary scanning.
5. It still has a `min_invocations` hard floor named `http_probe`, even though HTTP probing belongs to EAS and found content facts are already DB-projected.

This design fixes those contracts.

## 2. Current Code Reality

### 2.1 Stage Spec

`resources/harness/stages/enumeration/spec.json` already declares the important parts:

- `allowed_tool_types = ["recon/http", "recon/crawler", "web/fuzzer", "web/param"]`.
- `findings_allowed = false`.
- `coverage_complete` uses `derive_from_evidence = true`.
- Expected techniques are `GOLISH-ENUM-DIR`, `GOLISH-ENUM-PARAM`, `GOLISH-ENUM-JSAPI`.
- `specialist = "enumerator"` and `coverage_axis = ["DIR", "PARAM", "JSAPI"]`.
- `freshness_window = true`, so DB truth only counts rows collected after the stage start.

Important mismatch:

```json
"min_invocations": {
  "http_probe": 1
}
```

`min_invocations_check` only checks `deliverable.required_checks_done`. It does not read actual tool evidence. For enumeration this hard floor is stale: liveness probing was moved to EAS, and enumeration found facts are supposed to come from `directory_entries` and `api_endpoints`.

### 2.2 Stage Deliverable Shape

`StageDeliverable` is generic:

- `claims[]` with `evidence_ids`.
- top-level `evidence_refs`.
- `findings[]`.
- `required_checks_done[]`.
- `coverage[]` with `(asset, technique, status, evidence_refs, note, tested_units, total_units, sampling_rationale)`.

For enumeration, `findings` should remain empty or be dropped, because this is a facts-only discovery stage. The real result is the content inventory that lands in business tables.

### 2.3 DB Truth For Enumeration

`coverage_truth.rs` already maps enumeration techniques to DB facts:

| Technique | Current DB truth source | Meaning |
|---|---|---|
| `GOLISH-ENUM-DIR` | `directory_entries` joined by `target_id` | At least one directory/path entry landed for this target during this run |
| `GOLISH-ENUM-PARAM` | `api_endpoints.params` non-empty | At least one endpoint with parameters landed |
| `GOLISH-ENUM-JSAPI` | `api_endpoints.source IN ('js_analysis', 'crawler')` | JS analyzer or browser/crawler persisted endpoints |

`js_analysis_results` is not the direct gate source for JSAPI. It is the rich analysis artifact store. The gate source is the projected `api_endpoints` rows.

### 2.4 JS/API Tooling Is Now Mostly On The Right Side

`browser_collect_js_api`:

- opens the target with Playwright,
- saves runtime-loaded JS files under `.golish/captures/{host}/{port}/js/`,
- persists observed XHR/fetch into `api_endpoints(source='crawler')`.

`js_extract_apis`:

- reads captured JS,
- runs `golish-js-analyzer`,
- persists per-file analysis into `js_analysis_results`,
- projects resolvable endpoint call-sites into `api_endpoints(source='js_analysis')`,
- returns redacted secret/config/framework/library/rule-match candidates for model review.

This is the correct pattern: rich artifact table plus DB projection for gate truth.

Both tools now also upsert `technique_outcomes(GOLISH-ENUM-JSAPI)` (source `js_extract_apis` / `browser_collect_js_api`): `found` when endpoints/API requests were observed or persisted, `empty` when the tool ran clean with zero results, and `error` on read/persist errors or — for `browser_collect_js_api` — a hard timeout / helper non-zero exit. `found` is still independently projected from `api_endpoints` business truth, so these provenance rows mainly record the `empty`/`error` terminal states DB business rows cannot express (I8). They share the `(run_id, asset, technique)` upsert key (last-writer-wins), and the business-table `found` projection prevents a later `empty` from erasing a real hit.

### 2.5 Directory And Endpoint Landing Still Need Tightening

`output_store` routes:

- `directory_entry_add` to `store_directory_entry`.
- `endpoint_add` to `store_endpoint`.

Important details:

- `store_directory_entry` only binds `target_id` when the parsed `url` is absolute `http(s)`.
- Relative outputs from ffuf/gobuster can insert rows without `target_id`, which means `coverage_truth` will not count them for `GOLISH-ENUM-DIR`.
- `store_endpoint` rejects non-absolute URLs instead of guessing a host.
- `fields_with_command_target` currently enriches missing targets only for `target_update_recon`, not for `directory_entry_add` or `endpoint_add`.

Therefore, any implementation that relies on CLI text output must guarantee absolute URLs or pass a `base_url/target_id` through a wrapper. Otherwise the tool can run and still fail to satisfy DB truth.

> ✅ Resolved (2026-06-26, Phase 3 — Option B): `fields_with_command_target` now absolutizes `directory_entry_add` URLs. For a relative ffuf/gobuster token it recovers the command's `-u`/`--url` base and either replaces `FUZZ` (ffuf) or `Url::join`s the origin (gobuster), so `store_directory_entry` resolves a host → non-null `target_id` and the row counts for `GOLISH-ENUM-DIR`. No-base / already-absolute cases keep legacy behavior. `endpoint_add` absolutization is still out of scope (arjun/katana already emit absolute URLs in practice).

### 2.6 Sub-Agent And Stage Run

The enumerator already exists:

- It is registered as `enumerator`.
- It has `list_in_scope_targets`, `query_target_data`, `manage_targets`, `pentest_run`, `wait_for_background_jobs`, `browser_collect_js_api`, `js_collect`, `js_extract_apis`, and `submit_stage_deliverable`.
- It does not expose passive intel tools or exploitation tools.

`stage_run`:

- runs the current stage's `specialist` per organization,
- gates each org independently,
- retries blocked orgs with gate feedback,
- returns a pass token when every org passes.

The code currently runs per-org units serially. The `concurrency` parameter is reserved for future K-parallel fan-out. So this design should treat `stage_run` as the org-level runner and avoid adding another "one-click full enumeration" layer below it.

## 3. Design Principles

1. **Stage run is org-level, not scan-level.**
   `stage_run` should fan out one Enumerator per org. Inside the Enumerator, enumeration remains a visible sequence over web roots.

2. **Only enumerate live web services from EAS.**
   Enumeration should not rediscover subdomains, scan ports, or fuzz dead hosts.

3. **JS/API comes before route/path probing.**
   Runtime JS and network requests give high-signal route prefixes, API bases, forms, and auth hints.

4. **Route/path probing comes before dictionary brute force.**
   The RouteVulScan idea is to recursively probe every observed path layer with small, accurate payload sets. That is different from large wordlist fuzzing.

5. **DB facts define found.**
   `found` for DIR/PARAM/JSAPI should come from `directory_entries`, `api_endpoints`, and `technique_outcomes`, not from a model-written claim.

6. **StageDeliverable is slim.**
   The model submits negative/blocked/not_applicable states and high-level claims. It does not hand-copy every discovered path or endpoint into the final JSON.

7. **Enumeration does not produce vulnerabilities.**
   It may produce exposure candidates and triage work items, but vulnerability confirmation belongs to `vuln_triage`.

## 4. Deliverable Model

The stage has four deliverable layers. Only the last layer is the literal `StageDeliverable`.

### 4.1 Input Deliverable From EAS: WebRoot

Enumeration needs a stable web-root input contract. P0 can derive it from existing `targets`, `ports`, `http_status`, and `fingerprints`; P1 can materialize it if needed.

Desired shape:

```json
{
  "web_root_id": "derived:target_id:scheme:port",
  "target_id": "uuid",
  "organization_id": "uuid",
  "root_url": "https://app.example.com:8443/",
  "final_url": "https://app.example.com/app/",
  "scheme": "https",
  "host": "app.example.com",
  "port": 8443,
  "status": 200,
  "title": "Example Console",
  "fingerprints": ["nginx", "React"],
  "auth_state": "anonymous|authenticated|unknown",
  "evidence_ids": [123],
  "source_stage": "external_attack_surface"
}
```

P0 implementation recommendation:

- Add `list_enumeration_web_roots` as a read-only security-analysis tool, or extend `list_in_scope_targets` with a `web_roots` section.
- Derive web roots from:
  - `targets.target_type = url`,
  - `targets.http_status` / `content_type`,
  - `targets.ports` web-like services,
  - `fingerprints` that indicate HTTP technology.
- Return `needs_probe` only when scheme/final URL is ambiguous. Do not force the model to guess.

### 4.2 Persisted Facts: Content Discovery Units

Enumeration's real output is the set of persisted content units:

| Unit | Existing store | Required fields | Coverage role |
|---|---|---|---|
| JS artifact | file under `.golish/captures/.../js` plus `js_analysis_results` | `target_id`, URL/file path/hash, analysis summary, redacted signal candidates | Rich artifact, supports JSAPI |
| API endpoint | `api_endpoints` | `target_id`, absolute `url`, `method`, normalized `path`, `params`, `source`, `auth_type`, evidence/capture refs | JSAPI and PARAM |
| Directory/route | `directory_entries` | `target_id`, absolute `url`, status, size, content type, tool/source | DIR |
| Route probe candidate | P0: `directory_entries(tool='route_probe')` + raw evidence; P1 optional table | `base_url`, `prefix`, `probe_path`, status/body hash/matched rule | DIR, triage seed |
| Form/action | P0: `api_endpoints(source='crawler')` or raw JS/crawler analysis; P1 optional metadata | action URL, method, field names, auth state | PARAM and triage seed |
| Source-map / config / secret candidate | `js_analysis_results.raw_analysis` | source file, line, redacted preview, sha256, rule id, confidence | Triage context, not coverage by itself |

The guiding rule:

```text
If it should become a vuln_triage denominator, it must land as a structured DB row.
If it is only context, it can stay in js_analysis_results.raw_analysis / evidence metadata.
```

### 4.3 Derived Worklist For Vuln Triage

`vuln_triage` should not parse an enumeration prose summary. It should consume a queryable worklist derived from:

- `api_endpoints` for API/auth/IDOR tests.
- `directory_entries` for sensitive path, backup, admin, docs, upload, debug, and exposure checks.
- `js_analysis_results.raw_analysis` for JS secret/config/source-map candidate review.
- Optional `technique_outcomes` for explicit empty/error/blocked terminal states.

Suggested derived fields:

```json
{
  "unit_id": "uuid-or-derived-key",
  "target_id": "uuid",
  "web_root": "https://app.example.com/",
  "kind": "api_endpoint|directory|route_probe|form|js_signal",
  "url": "https://app.example.com/api/users/{id}",
  "method": "GET",
  "params": ["id", "page"],
  "source": "js_analysis|crawler|route_probe|ffuf|gobuster|arjun|katana",
  "auth_hint": "bearer|cookie|none|unknown",
  "candidate_class": "admin|debug|swagger|source_map|backup|upload|idor_candidate|secret_candidate",
  "evidence_ids": [123],
  "confidence": "high|medium|low"
}
```

This worklist can be a query/view first. A new table is optional and should wait until reporting or sampling needs prove it.

### 4.4 Literal StageDeliverable

For `enumeration`, the literal `StageDeliverable` should become slim:

```json
{
  "stage_id": "enumeration",
  "claims": [
    {
      "kind": "enumeration_summary",
      "subject": "https://app.example.com/",
      "summary": "JS/API, route probe, light directory, and parameter discovery completed for this web root.",
      "evidence_ids": [123, 124, 125]
    }
  ],
  "evidence_refs": [123, 124, 125],
  "findings": [],
  "required_checks_done": [],
  "coverage": [
    {
      "asset": "https://static.example.com/",
      "technique": "GOLISH-ENUM-PARAM",
      "status": "not_applicable",
      "note": "Static content root; no forms, no query-bearing endpoints, no runtime API calls after browser collection."
    },
    {
      "asset": "https://app.example.com/admin/",
      "technique": "GOLISH-ENUM-DIR",
      "status": "blocked",
      "note": "Route probe was stopped by scope/auth boundary; do not continue without credentials or approval."
    }
  ]
}
```

Rules:

- Do not hand-write `found` coverage when DB truth already has rows.
- `checked_empty` needs real evidence of a run returning no applicable result.
- `blocked` and `not_applicable` need a note.
- `findings` stays empty.
- `required_checks_done` should not be the primary contract.

## 5. Desired Flow

### 5.1 High-Level Stage Order

```text
target_intel
  -> passive org/asset discovery

external_attack_surface
  -> liveness + ports + service fingerprint
  -> produces live web roots

enumeration
  -> browser visit + JS/API landing
  -> API/route seed normalization
  -> lightweight recursive route probe
  -> bounded dictionary path scan
  -> parameter discovery
  -> slim StageDeliverable

vuln_triage
  -> consumes testable units and decides findings
```

### 5.2 Per-WebRoot Enumerator Flow

For each WebRoot:

1. **Load WebRoot inventory**
   - Call `list_enumeration_web_roots` or P0 equivalent.
   - Skip roots that are not EAS-confirmed live web services.
   - Keep `target_id` and `root_url` with every downstream action.

2. **Baseline and soft-404 profile**
   - Establish redirect/final URL, auth state, content type, body-size/hash baseline, and fallback behavior.
   - This can be part of `browser_collect_js_api` or a new lightweight `web_root_baseline` helper.
   - Output is evidence, not a vulnerability.

3. **Browser JS/API collection**
   - Run `browser_collect_js_api(crawl_mode="fast", ai_assist=true)`.
   - If closure is partial, run one bounded `deep` or recipe pass.
   - Persist runtime XHR/fetch into `api_endpoints(source='crawler')`.

4. **Static JS analysis**
   - Run `js_extract_apis`.
   - Persist deterministic endpoint call-sites into `api_endpoints(source='js_analysis')`.
   - Persist redacted JS signal candidates into `js_analysis_results`.
   - Use `ai_analysis` only for targeted review, not for invented endpoints.

5. **Seed normalization**
   - Merge seeds from:
     - browser network requests,
     - JS endpoints,
     - crawler URLs,
     - HTML links/forms,
     - robots/sitemap/swagger/openapi/graphql well-known paths,
     - historical URLs only if already scoped and bounded.
   - Canonicalize:
     - host/scheme/port,
     - path template (`/api/user/123` -> `/api/user/{id}`),
     - query parameter names,
     - trailing slash and duplicate slashes,
     - static assets vs route paths.

6. **Lightweight recursive route probe**
   - For each observed path, derive parent prefixes.
   - Example: `/aaa/bbb/detail?id=1` probes `/`, `/aaa/`, `/aaa/bbb/`.
   - For each prefix, send a small curated set of probes:
     - docs/spec paths: `swagger`, `openapi`, `api-docs`,
     - framework/debug paths: `actuator`, `__debug`, `env`,
     - admin/upload/backup path variants,
     - source map/config candidates from JS analysis.
   - Use original safe headers when available, similar to RouteVulScan's "Head" mode, but never replay unsafe methods or request bodies by default.
   - Respect `max_depth`, `max_requests`, `rate_limit`, same-origin, auth/scope boundaries.
   - Store positives as `directory_entries(tool='route_probe')` with absolute URLs and evidence.
   - Store negatives or blocked/error states via `technique_outcomes` where possible.

7. **Bounded dictionary path scan**
   - Run only after route probe.
   - Use small wordlists and only on live roots or high-value prefixes.
   - Use recursion depth 1 or 2, not unbounded recursion.
   - Ensure the writer stores absolute URL + `target_id`, otherwise the result cannot satisfy `GOLISH-ENUM-DIR`.

8. **Parameter discovery**
   - Run `arjun` or equivalent on discovered endpoints/forms, not on the whole host blindly.
   - Prefer endpoints from JS/API/crawler/route probe.
   - Persist params into `api_endpoints.params`.

9. **Submit**
   - Wait for background jobs.
   - Query target data for a quick DB sanity check.
   - Submit a slim `StageDeliverable` with negative/blocked/not_applicable coverage only.

## 6. Route Probe Design

### 6.1 Why This Is Not Just ffuf Recursion

RouteVulScan's useful idea is:

- passive or semi-passive seed collection,
- split observed paths into every path layer,
- probe each layer with a small accurate rule set,
- match responses with rules,
- avoid large brute-force wordlists.

That is exactly where Golish should put the "lightweight recursive" action.

`ffuf -recursion` and `gobuster dir` are still useful, but they are dictionary brute force. They should be the bounded補盲 step after route probe, not the first content-enumeration action.

### 6.2 P0 Tool Recommendation

Add a first-class bridge tool:

```text
route_probe_paths
```

Suggested crate location:

```text
backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs
```

Register it in `create_pentest_bridge_tools` and expose it to the `enumerator`.

Suggested input:

```json
{
  "target_id": "uuid",
  "base_url": "https://app.example.com/",
  "observed_paths": ["/app/users/123", "/api/v1/orders?id=1"],
  "max_depth": 3,
  "max_requests": 200,
  "rate_limit_per_sec": 5,
  "same_origin": true,
  "carry_safe_headers": true,
  "rule_groups": ["docs", "debug", "admin", "backup", "upload", "source_map"]
}
```

Suggested output:

```json
{
  "success": true,
  "base_url": "https://app.example.com/",
  "prefixes_tested": ["/", "/app/", "/api/", "/api/v1/"],
  "requests_sent": 86,
  "matches": [
    {
      "url": "https://app.example.com/api/v1/swagger.json",
      "status": 200,
      "rule_id": "swagger-json",
      "candidate_class": "api_docs",
      "content_length": 42182,
      "evidence_id": 123
    }
  ],
  "persisted_directory_entries": 1,
  "outcome": "found|empty|error|blocked"
}
```

Why bridge tool instead of raw `pentest_run`:

- It can carry `target_id` directly.
- It can always persist absolute URLs.
- It can implement soft-404 filtering and response hashing consistently.
- It can book evidence and `technique_outcomes` without relying on CLI text parsing.
- It can enforce request budgets and safe headers.

### 6.3 Rule Set

Keep the first rule set small and auditable. Example groups:

| Group | Examples | Notes |
|---|---|---|
| `docs` | `swagger.json`, `openapi.json`, `api-docs`, `v3/api-docs`, `graphql` | Discovery only; no introspection query by default |
| `debug` | `actuator`, `actuator/env`, `__debug`, `.env` | Record exposure candidate, do not exploit |
| `admin` | `admin`, `manage`, `console`, `dashboard` | Candidate path, not vulnerability |
| `backup` | `.bak`, `.old`, `.zip`, `.tar.gz` variants for observed route names | Limit count aggressively |
| `upload` | `upload`, `files`, `attachments` | Candidate path |
| `source_map` | `*.js.map` for captured JS | Tied to actual JS artifacts |

Each rule needs:

- id,
- path template,
- allowed methods, default `GET` or `HEAD`,
- expected status ranges,
- response keyword/regex/hash heuristic,
- candidate class,
- max per-prefix expansion count.

Rules should live under `resources/` rather than in model prompt text.

## 7. Required Flow Changes

### 7.1 `enumeration/methodology.md`

Current recommended sequence:

```text
1. Crawl + JS/API
2. Directory/path discovery
3. Parameter discovery
```

Recommended new sequence:

```text
1. Load EAS-confirmed WebRoots.
2. Browser baseline + JS/API collection.
3. JS extraction + seed normalization.
4. Lightweight recursive route probe.
5. Small bounded directory/path scan.
6. Parameter discovery on discovered endpoints/forms.
7. Submit slim deliverable.
```

The methodology should explicitly say:

- route probe happens after JS/API and before ffuf/gobuster,
- dictionary scan is bounded補盲,
- all path results must land with `target_id`,
- found coverage is DB-derived.

### 7.2 `enumeration/spec.json`

Recommended changes:

1. Remove or empty the stale `min_invocations`.

```json
"min_invocations": {}
```

Rationale: `http_probe` belongs to EAS. Enumeration completeness is already covered by `coverage_complete` over DIR/PARAM/JSAPI.

2. Make found facts authoritative once route/path/param empty outcomes are wired.

```json
"facts_from_db_truth": true
```

and in `coverage_complete`:

```json
"authoritative_found": true,
"authoritative_techniques": [
  "GOLISH-ENUM-DIR",
  "GOLISH-ENUM-PARAM",
  "GOLISH-ENUM-JSAPI"
]
```

Rationale: the stage is facts-only. A model-written found cell should not pass without `directory_entries` / `api_endpoints` / `technique_outcomes`.

3. Consider:

```json
"require_note_for_other": true
```

for blocked/not_applicable cells, matching the newer gate direction.

### 7.3 `build_enumerator_prompt`

Update the Enumerator prompt so it no longer sounds like a one-shot scan:

- Start from `list_enumeration_web_roots` or explicitly derive web roots from `list_in_scope_targets`.
- Browser JS/API first.
- Route probe second.
- Small dictionary scan third.
- Parameter discovery last.
- Never submit findings.
- Never hand-write found cells.

### 7.4 Directory Writer / Tool Config

Before relying on `ffuf` or `gobuster` for gate truth, fix one of these:

Option A, recommended for P0:

- Use `route_probe_paths` and other bridge tools for DB-crediting path results.
- Keep ffuf/gobuster as supplemental evidence.

Option B:

- Extend `fields_with_command_target` or `store_directory_entry` so `directory_entry_add` can combine a relative CLI result with the command's base URL.
- Add tests proving `ffuf -u https://x/FUZZ` output `admin [Status: 200...]` lands as `https://x/admin` with a non-null `target_id`.

Option C:

- Change ffuf/gobuster skills to emit JSON or absolute URLs and update parsers accordingly.

Do not accept a path scan as complete unless the row is target-bound.

### 7.5 `query_target_data`

`query_target_data` currently returns assets, endpoints, fingerprints, JS analysis, and scan logs. It should add:

```text
sections: ["directories", "coverage", "web_roots"]
```

At minimum:

- `directories` lists `directory_entries` by target.
- `coverage` shows current DIR/PARAM/JSAPI projection for that target if available.
- `web_roots` shows EAS-derived root URLs for the target.

This lets repair mode close exact gaps without relisting or rescanning the whole org.

## 8. Stage Run Integration

### 8.1 What Stage Run Should Do

Inside an active `enumeration` stage, the primary agent should call:

```text
stage_run({ orgs: [...] })
```

`stage_run` should:

- read `specialist = enumerator`,
- pass the enumeration methodology into each per-org objective,
- keep org isolation via `active_org_id_override`,
- let each Enumerator operate over that org's WebRoots,
- gate each org independently,
- retry only blocked orgs.

This is already aligned with the current `stage_run_call.rs` architecture.

### 8.2 What Stage Run Should Not Do

`stage_run` should not become "one button scans everything" at the web-root/action level.

Do not add a hidden deterministic loop that silently runs browser collection, route probes, ffuf, arjun, and submit without model-visible tool calls. That would recreate the one-click behavior the design is trying to avoid.

The visible split should be:

```text
stage_run
  -> org-level fan-out and gate/retry
Enumerator
  -> visible per-WebRoot sequence and tool evidence
Tools
  -> bounded HTTP actions and DB persistence
```

### 8.3 Progress UI

Current `StageRunOrgProgress` emits org-level progress and `coverage_axis`, but the code emits an empty `coverage` list at progress time.

P0 can keep org-level progress only.

P1 should add per-org enumeration phases:

```text
web_roots_loaded
browser_js_done
js_extract_done
route_probe_done
dir_scan_done
param_discovery_done
submitted
```

These phases should be observational. They should not replace gate truth.

## 9. Stop Conditions

Per WebRoot:

- JSAPI terminal when:
  - `api_endpoints(source='crawler'|'js_analysis')` rows exist, or
  - browser + static JS collection ran and produced no endpoint evidence, with checked_empty evidence.

- DIR terminal when:
  - `directory_entries(target_id=...)` rows exist from route probe / ffuf / gobuster, or
  - route probe + bounded directory pass ran and returned no candidates, with checked_empty evidence.

- PARAM terminal when:
  - `api_endpoints.params` is non-empty for at least one endpoint, or
  - parameter discovery ran against the discovered endpoint set and returned no params, with checked_empty evidence, or
  - root is static and not_applicable has a concrete note.

Per org:

- All WebRoots have terminal DIR/PARAM/JSAPI states.
- Background jobs have settled.
- StageDeliverable has no findings.
- DB projection is fresh for this stage-run.

Per engagement:

- `stage_run` returns `passed = true`.
- Primary agent submits the pass-token claim if required by current stage-run closeout semantics.

## 10. Implementation Plan

### Phase 0: Contract-Only Alignment

Files:

- `resources/harness/stages/enumeration/methodology.md`
- `resources/harness/stages/enumeration/spec.json`
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- `backend/crates/golish-agent-kit/src/harness/gate/min_invocations_check.rs` tests

Tasks:

1. Update methodology sequence with route probe.
2. Remove `http_probe` min invocation from enumeration.
3. Add `facts_from_db_truth` and authoritative found for enum techniques after tests are ready.
4. Update Enumerator prompt.

Verification:

```bash
cd backend && cargo nextest run -p golish-agent-kit enumeration min_invocations coverage_complete --status-level fail
cd backend && cargo nextest run -p golish-sub-agents enumerator --status-level fail
```

### Phase 1: WebRoot Read Model

Files:

- `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/recon.rs`
- `backend/crates/golish-agent-kit/src/tool_executors/security.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs` if a new tool is exposed to primary agents

Tasks:

1. Add `list_enumeration_web_roots` or extend `list_in_scope_targets`.
2. Derive web roots from current EAS DB truth without a migration.
3. Add `directories` section to `query_target_data`.

Verification:

```bash
cd backend && cargo nextest run -p golish-agent-kit query_target_data list_in_scope --status-level fail
cd backend && cargo nextest run -p golish-agent-app query_target_data --status-level fail
```

### Phase 2: Route Probe Tool

Files:

- `backend/crates/golish-pentest-app/src/pentest_bridge/route_probe_paths.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`
- `backend/crates/golish-sub-agents/src/defaults/builder/{mod.rs,registry.rs}`
- `resources/route-probe/*.yml` or similar

Tasks:

1. Implement bounded recursive prefix generation.
2. Implement soft-404 baseline filtering.
3. Persist matches as absolute `directory_entries`.
4. Book evidence and optionally `technique_outcomes`.
5. Expose to Enumerator.

Verification:

```bash
cd backend && cargo nextest run -p golish-pentest-app route_probe_paths --status-level fail
cd backend && cargo nextest run -p golish-sub-agents enumerator --status-level fail
```

### Phase 3: CLI Path Landing Fix

Files:

- `backend/crates/golish-pentest/src/output_store/mod.rs`
- `backend/crates/golish-pentest/src/output_store/findings.rs`
- `resources/toolsconfig/ffuf.json`
- `resources/toolsconfig/gobuster.json`

Tasks:

1. ✅ Ensure directory CLI results land with non-null `target_id` (done 2026-06-26: `fields_with_command_target` absolutizes `directory_entry_add` URLs in `output_store/mod.rs`).
2. Add a light recursive ffuf skill if still needed (ffuf.json already exposes `-recursion`/`-recursion-depth`; deferred).
3. ✅ Add tests for relative output + base URL materialization (done: 4 unit tests in `output_store/mod.rs`).

Verification:

```bash
cd backend && cargo nextest run -p golish-pentest output_store --status-level fail
```

Done 2026-06-26: `cargo nextest run -p golish-pentest output_store` → 19 passed; `cargo clippy -p golish-pentest --all-targets` → 0 warnings.

### Phase 4: Live Stage Run Validation

Use a real workspace run:

```bash
python3 scripts/run_tree.py --workspace <ws> <session> --full --db
```

Expected evidence:

- `stage_run` dispatches `enumerator`.
- Each org lists WebRoots derived from EAS.
- Browser JS/API rows land in `api_endpoints`.
- Route probe positives land in `directory_entries`.
- Param discovery rows land in `api_endpoints.params`.
- `coverage_complete` passes from DB truth without hand-written found cells.

## 11. Open Decisions

1. **Route probe as bridge tool or pentest registry tool?**
   Recommendation: bridge tool first, because it needs `target_id`, baseline, DB persistence, and strict budgets.

2. **Materialize WebRoot now or derive it?**
   Recommendation: derive in P0, materialize later only if UI/reporting needs it.

3. **Turn on `authoritative_found` immediately?**
   Recommendation: turn it on after route probe and directory landing tests pass. JSAPI is already close; DIR is the risky axis.

4. **Should route-probe candidates become findings?**
   No. They become triage work items. `vuln_triage` decides findings.

5. **Should stage_run become K-parallel now?**
   No. Keep current serial per-org MVP until enumeration is correct. Hidden parallelism will make debugging path/DB truth gaps harder.

## 12. Final Contract

After this design is implemented, a successful enumeration run means:

```text
For every EAS-confirmed live web root:
  JS/API was collected or honestly terminal,
  route/path discovery was collected or honestly terminal,
  parameter discovery was collected or honestly terminal,
  all found units landed in DB with target_id and evidence,
  StageDeliverable contains only summary claims and explicit non-found terminal exceptions,
  vuln_triage can derive its worklist from DB without reading prose.
```

