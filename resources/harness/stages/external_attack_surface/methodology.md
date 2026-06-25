**Goal:** actively map the attack surface of the APPROVED hosts — for every
in-scope asset establish (1) liveness, (2) open ports, (3) service/version
fingerprint. Subdomains are INHERITED from `target_intel`; do NOT re-enumerate
them. This is the first stage that touches the target, gated by `active_scan`
approval.

**Recommended sequence:**

1. Pull the inherited seed list. Prefer `list_attack_surface_seeds`; use
   `list_in_scope_targets` only as the lean fallback. Work from that set, not from
   fresh passive enumeration.
2. Classify each seed:
   - `domain` / `ip`: scan ports and establish liveness.
   - `url`: probe URL liveness; do not assign PORT/SERVICE to the path URL.
   - `cidr`: treat as a range. Get approval when required, sweep it, register
     discovered live IPs as concrete targets, then scan each IP.
   - wildcard: do not brute-force here; concrete inherited hosts carry the work.
3. Liveness + HTTP fingerprint — `httpx` over host/url targets in batches. DNS was
   already done in `target_intel`; reuse inherited DNS instead of re-running `dig`.
4. Port discovery — `naabu` / `masscan` / `nmap` for concrete host/IP targets.
   Every IP or host must have a fresh port-scan terminal result.
5. Service/version fingerprint — prefer `nmap -sV` for every discovered open
   port; use `whatweb` only for HTTP(S) services when its Ruby runtime is ready.
   If `whatweb` returns a runtime/SSL/opening error, record the failed attempt,
   do not retry it on the same host, and continue with `nmap -sV` / `httpx`
   evidence. NEVER assume a service from the port number alone (8080 is not
   proof of Tomcat), and never treat HTTP liveness alone as PORT/SERVICE
   coverage.
6. (Optional) `gowitness` screenshots of live web services for the record.

**If a tool is missing or errors:**

- Record it in `skipped_checks` with the reason and use a fallback (e.g. if `httpx`
  is unavailable, use `nmap -sV` / `nmap -Pn -p- --open` for liveness+service).
- Do NOT install tools, spawn extra sub-agents, or retry a blocked/missing tool in
  a loop. Note it and move on — "checked_empty" is NOT "unchecked".

**Coverage + stop condition:**

- The gate reads database truth for found GOLISH-EAS-LIVENESS /
  GOLISH-EAS-PORT / GOLISH-EAS-SERVICE-FINGERPRINT cells. When `httpx`,
  `naabu`/`masscan`/`nmap`, `whatweb`, or `nmap -sV` data lands in
  `targets`, `targets.ports`, `fingerprints`, or `technique_outcomes`, the
  corresponding found coverage is credited automatically.
- Do NOT hand-write found coverage cells just to mirror the database. Add
  `coverage` only for terminal states the database cannot derive yet:
  `checked_empty` with the real scan/probe evidence for active negatives, or
  `blocked` / `not_applicable` with a concrete note.
- If there are no open ports, SERVICE-FINGERPRINT is `not_applicable` with a
  note, not a fabricated found service and not `checked_empty` with
  `total_units=0`. HTTP liveness alone is never PORT or SERVICE-FINGERPRINT.
- Once every in-scope host has liveness + ports + service mapped (or an honest
  skip), `submit_stage_deliverable`. Do not jump into content enumeration or
  vuln scanning — the harness advances to `enumeration` for you.
