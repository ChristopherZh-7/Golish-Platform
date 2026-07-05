**Goal:** actively map the attack surface of the APPROVED hosts — establish
liveness for inherited domain/URL/vhost assets, and establish open ports plus
service/version fingerprints only for concrete IP/CIDR host assets. Subdomains
are INHERITED from `target_intel`; do NOT re-enumerate them. This is the first
stage that touches the target, gated by `active_scan` approval.

**Coverage-driven strategy (not a rigid pipeline):**

1. Pull the inherited seed list. Prefer `list_attack_surface_seeds`; use
   `list_in_scope_targets` only as the lean fallback. Work from that set, not from
   fresh passive enumeration.
2. Before launching broad tools, inspect the current truth: call
   `check_stage_asset_coverage` and/or `query_target_data` to understand which
   asset × technique cells are already terminal, which are still pending, and
   which assets are `domain` / `ip` / `url` / `cidr`. Decide the next batch from
   that state; do not submit just to learn what is missing.
3. Classify each seed:
   - `ip`: scan ports and establish liveness.
   - `domain`: establish liveness only. Do not port-scan a domain string. If
     `target_intel` has a concrete `real_ip` / IP target, scan that IP once for
     PORT/SERVICE. If there is no concrete IP to scan, treat PORT/SERVICE as not
     applicable to the domain and surface the missing IP as a target_intel/DNS
     landing gap rather than making EAS invent a host.
   - `url`: probe URL liveness; do not assign PORT/SERVICE to the path URL.
   - `cidr`: treat as a range. Get approval when required, sweep it, register
     discovered live IPs as concrete targets, then scan each IP.
   - wildcard: do not brute-force here; concrete inherited hosts carry the work.
4. Liveness + HTTP fingerprint — batch host/url targets through
   `eas_probe_http_liveness` early
   when it will reduce uncertainty or give you HTTP evidence. This is normally the
   cheapest way to separate live web assets from dead leads, but it is not a hard
   ordering rule: if the DB already has fresh liveness or a targeted port batch is
   clearly the right next gap, use judgment.
   The wrapper owns the fixed `httpx` recipe and batching; call it with
   `targets=[...]` instead of building raw `httpx` / `pentest_run` args. DNS was
   already done in `target_intel`; reuse inherited DNS instead of re-running
   `dig`.
   If a liveness probe is terminal-empty, mark only LIVENESS checked_empty with
   evidence; do not claim domain PORT/SERVICE work because those cells belong to
   concrete IP/CIDR hosts.
5. Port discovery — call `eas_discover_ports` for concrete IP/CIDR targets.
   Default to `scanner="naabu"`; use `scanner="masscan"` for larger approved
   ranges when appropriate and `scanner="nmap"` as fallback/verification. The
   wrapper owns the list-file recipe and rejects domains/URLs. Every
   IP/CIDR-discovered host must have a fresh port-scan terminal result.
6. Service/version fingerprint — call `eas_fingerprint_services` only for
   concrete IP targets with confirmed open ports. The wrapper owns the
   `nmap -sV -Pn -iL ... -p ... -T3` recipe and rejects raw domains, URL strings,
   and CIDR ranges. Close blocked concrete-host SERVICE cells with a concrete
   note. NEVER assume a service from the port number alone (8080 is not proof of
   Tomcat), and never treat HTTP liveness alone as PORT/SERVICE coverage.

**If a tool is missing or errors:**

- Record it in `skipped_checks` with the reason and use another EAS wrapper
  fallback where possible (for example, `eas_discover_ports(scanner="nmap")` for
  concrete IPs when `naabu` is unavailable), not unresolved domains.
- Do NOT install tools, spawn extra sub-agents, or retry a blocked/missing tool in
  a loop. Note it and move on — "checked_empty" is NOT "unchecked".
- If only one target in a batch fails, do not downgrade the whole batch. Re-run or
  mark only that target's terminal cell, and keep the successful batch evidence.
- If a backgrounded scan is still running after a visible wait, inspect it like
  Cursor/Codex would: use `check_job` once. If stdout/stderr is still moving and
  the batch is appropriate, wait again; if output is not moving or the batch is
  too broad, `kill_job` it and close the affected cells with a concrete
  blocked/error/not_applicable note or a narrower batch.

**Coverage + stop condition:**

- The gate reads database truth for found GOLISH-EAS-LIVENESS /
  GOLISH-EAS-PORT / GOLISH-EAS-SERVICE-FINGERPRINT cells. When
  `eas_probe_http_liveness`, `eas_discover_ports`, or
  `eas_fingerprint_services` data lands in
  `targets`, `targets.ports`, `fingerprints`, or `technique_outcomes`, the
  corresponding found coverage is credited automatically.
- Do NOT hand-write found coverage cells just to mirror the database. Add
  `coverage` only for terminal states the database cannot derive yet:
  `checked_empty` with the real scan/probe evidence for active negatives, or
  `blocked` / `not_applicable` with a concrete note.
- If there are no open ports, SERVICE-FINGERPRINT is `not_applicable` with a
  note, not a fabricated found service and not `checked_empty` with
  `total_units=0`. HTTP liveness alone is never PORT or SERVICE-FINGERPRINT.
- Before submitting, call `check_stage_asset_coverage`. If
  `ready_to_submit=false`, use its `gap_examples` to close the missing
  asset-technique cells by grouping gaps with the same technique/tool into batch
  wrapper calls where possible. Use honest
  checked_empty/blocked/not_applicable coverage when a tool cannot run. Submit
  only when the preflight says
  `ready_to_submit=true`. This is a required self-check before submit, not a
  trial submit: do not call `submit_stage_deliverable` just to discover missing
  cells.
- Do not jump into content enumeration or vuln scanning — the harness advances
  to `enumeration` for you.
