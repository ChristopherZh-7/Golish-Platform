**Goal:** enumerate CONTENT on the services EAS already mapped — JavaScript/API
endpoints, directories/paths, and parameters. Port/service discovery is already
done in EAS; do NOT re-port-scan here. The units you enumerate (endpoints, params)
become the coverage denominator for `vuln_triage`.

**Recommended sequence (only on live web services from EAS):**

1. Crawl + JS/API — `katana` to crawl, then `js_collect` + `js_extract_apis` to
   pull JS bundles and extract API endpoint call-sites (cheap, high-signal).
2. Directory/path discovery — `ffuf` / `gobuster` against the live web roots with
   a sensible wordlist. Scope to confirmed-live services, not the whole subnet.
3. Parameter discovery — `arjun` (or equivalent) on the discovered endpoints.

**Efficiency red lines:**

- Do NOT re-scan ports or re-fingerprint services — reuse EAS's evidence.
- Enumerate only the live services EAS confirmed; don't fuzz dead hosts.
- One sensible wordlist pass per service; don't loop swapping wordlists endlessly.

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
