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
   web root, then one bounded deep/recipe pass when closure is partial. After JS
   files are saved, run `js_extract_apis`; it persists deterministic endpoints
   and returns redacted secret/config/framework candidates plus rule-based
   `rule_matches` and `ai_analysis` line ranges for targeted model review.
   Treat rule matches as candidates only; do not invent endpoints from AI
   inference.
3. Seed normalization — merge browser requests, JS endpoints, crawler URLs, HTML
   links/forms, and well-known docs paths into scoped same-origin path seeds.
   Normalize host/scheme/port, route templates, query parameter names, trailing
   slashes, and static asset noise before probing.
4. Lightweight recursive route probe — run `route_probe_paths` over observed path prefixes with a small
   curated rule set (docs/debug/admin/backup/upload/source-map style checks)
   before broad dictionary fuzzing. Store positives as absolute
   `directory_entries` with `target_id`; store empty/blocked/error states as
   explicit evidence-backed terminal coverage when DB truth cannot derive them.
5. Bounded directory/path discovery — use `ffuf` / `gobuster` only as a small
   scoped backfill against live roots or high-value prefixes. Do not treat CLI
   output as coverage unless it lands with an absolute URL and non-null
   `target_id`.
6. Parameter discovery — run `arjun` (or equivalent) on discovered endpoints and
   forms, not blindly across the whole host. Persist parameter names into
   `api_endpoints.params`.
7. Slim submit — wait for background jobs, sanity-check the DB-visible content
   units, then submit only summary claims plus checked_empty/blocked/not_applicable
   coverage that the database cannot derive.

**Efficiency red lines:**

- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- Route probe before dictionary fuzzing; dictionary scans are bounded backfill.
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
- Once each live service has dir + param + JS/API enumerated (or an honest skip),
  `submit_stage_deliverable`.
- Do not hand-write `found` cells that DB truth can derive from
  `directory_entries` or `api_endpoints`. Submitted coverage is for
  checked_empty, blocked, or not_applicable terminal states that still need notes
  or evidence.
