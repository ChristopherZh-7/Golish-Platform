**Goal:** actively map the attack surface of the APPROVED hosts — for every
in-scope asset establish (1) liveness, (2) open ports, (3) service/version
fingerprint. Subdomains are INHERITED from `target_intel`; do NOT re-enumerate
them. This is the first stage that touches the target, gated by `active_scan`
approval.

**Recommended sequence:**

1. Pull the inherited host list (target_intel subdomains + roots). Work from that
   set — `list_in_scope_targets` / `query_target_data` give ids and details.
2. Liveness + HTTP fingerprint — `httpx` over the host list in ONE batch (it
   resolves, probes, and fingerprints tech in a single pass). This replaces
   per-host `dig`.
3. Port discovery — `naabu` (fast) and/or `nmap` for the live hosts.
4. Service/version fingerprint — `nmap -sV` on discovered ports; `whatweb` for web
   tech. NEVER assume a service from the port number alone (8080 ≠ Tomcat).
5. (Optional) `gowitness` screenshots of live web services for the record.

**If a tool is missing or errors:**

- Record it in `skipped_checks` with the reason and use a fallback (e.g. if `httpx`
  is unavailable, use `nmap -sV` / `nmap -Pn -p- --open` for liveness+service).
- Do NOT install tools, spawn extra sub-agents, or retry a blocked/missing tool in
  a loop. Note it and move on — "checked_empty" is NOT "unchecked".

**Coverage + stop condition:**

- Per in-scope asset, give GOLISH-EAS-LIVENESS / GOLISH-EAS-PORT /
  GOLISH-EAS-SERVICE-FINGERPRINT a terminal status with evidence in `coverage`.
- Once every in-scope host has liveness + ports + service mapped (or an honest
  skip), `submit_stage_deliverable`. Do not jump into content enumeration or
  vuln scanning — the harness advances to `enumeration` for you.
