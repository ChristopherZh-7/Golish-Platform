**Goal:** actively map the attack surface of the APPROVED hosts — establish
liveness for inherited domain/URL/vhost assets, and establish open ports plus
service/version fingerprints only for concrete IP/CIDR host assets. Subdomains
are INHERITED from `target_intel`; do NOT re-enumerate them. This is the first
stage that touches the target, gated by `active_scan` approval.

**Coverage-driven strategy (IP-first for concrete hosts):**

1. Pull the inherited seed list. Prefer `list_attack_surface_seeds`; use
   `list_in_scope_targets` only as the lean fallback. Work from that set, not from
   fresh passive enumeration.
2. Before launching broad tools, inspect the current truth: call
   `check_stage_asset_coverage` and/or `query_target_data` to understand which
   asset × technique cells are already terminal, which are still pending, and
   which assets are `domain` / `ip` / `url` / `cidr`. Decide the next batch from
   that state; do not submit just to learn what is missing.
3. Classify each seed:
   - `ip`: run port discovery first. Fresh open-port evidence proves liveness;
     a terminal no-open-port result closes the IP liveness/port work without
     falling back to generic HTTP probing.
   - `domain`: establish liveness only. Do not port-scan a domain string. If
     `target_intel` has a concrete `real_ip` / IP target, scan that IP once for
     PORT/SERVICE. If there is no concrete IP to scan, treat PORT/SERVICE as not
     applicable to the domain and surface the missing IP as a target_intel/DNS
     landing gap rather than making EAS invent a host.
   - `url`: probe URL liveness; do not assign PORT/SERVICE to the path URL.
   - `cidr`: treat as a range. Get approval when required, sweep it, register
     discovered live IPs as concrete targets, then scan each IP.
   - wildcard: do not brute-force here; concrete inherited hosts carry the work.
4. Liveness + HTTP probe — for concrete IP/CIDR assets, prefer
   `eas_discover_ports` first; do not use `eas_probe_http_liveness` as a generic
   IP-alive test. Use `eas_probe_http_liveness` for inherited domain/URL/vhost
   assets, or for confirmed/likely web origins where HTTP metadata is the next
   missing fact.
   The wrapper owns the fixed `httpx` recipe and batching; call it with
   `targets=[...]` instead of building raw `httpx` / `pentest_run` args. DNS was
   already done in `target_intel`; reuse inherited DNS instead of re-running
   `dig`.
   If a liveness probe is terminal-empty, mark only LIVENESS checked_empty with
   evidence; do not claim domain PORT/SERVICE work because those cells belong to
   concrete IP/CIDR hosts.
5. Port discovery — call `eas_discover_ports` for every concrete IP/CIDR target
   that is still runnable.
   Default to `scanner="naabu"`; use `scanner="masscan"` for larger approved
   ranges when appropriate and `scanner="nmap"` as fallback/verification. The
   wrapper owns the list-file recipe and rejects domains/URLs. Every
   IP/CIDR-discovered host must have a fresh port-scan terminal result.
6. Service/version fingerprint — call `eas_fingerprint_services` only for
   concrete IP targets with confirmed open ports. The wrapper owns the
   `nmap -sV -Pn -iL ... -p ... -T3` recipe and rejects raw domains, URL strings,
   and CIDR ranges. Every confirmed open port, including ports discovered later
   in this stage, must get one fingerprint attempt unless a concrete blocker
   makes it impossible; if new ports are added after the first fingerprint pass,
   run the wrapper again for only those new ports. Close blocked concrete-host
   SERVICE cells with a concrete note. A port-scoped nmap terminal result such
   as `tcpwrapped` closes that port for coverage even though it is not a strong
   service identity; do not keep rerunning it without a new reason. Bare DNS/53
   does not block SERVICE-FINGERPRINT for a multi-service host. NEVER assume a
   service from the port number alone (8080 is not proof of Tomcat), and never
   treat HTTP liveness alone as PORT/SERVICE coverage.
7. Web technology fingerprint — after httpx or nmap confirms an HTTP(S)
   endpoint, call `eas_fingerprint_web_stack` with absolute HTTP(S) URLs. The
   wrapper owns the WhatWeb batch recipe and writes web-stack fingerprints into
   the same `fingerprints` / `web_origins` surface the Target UI reads. This is a
   required WEB-FINGERPRINT coverage cell for every confirmed web origin, not a
   substitute for IP:port SERVICE-FINGERPRINT. Never use it for
   SSH/MySQL/SMTP/non-HTTP SERVICE gaps, and do not call raw `whatweb` /
   `pentest_run`. If several domains/vhosts share one IP:port, nmap the IP:port
   once for service/version, but run WhatWeb once per confirmed web origin
   (`scheme://host:port`) because Host/SNI can expose different stacks.

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
  GOLISH-EAS-PORT / GOLISH-EAS-SERVICE-FINGERPRINT /
  GOLISH-EAS-WEB-FINGERPRINT cells. `eas_discover_ports` credits port truth from
  `targets.ports` and also closes concrete-IP LIVENESS from the same scan;
  `eas_probe_http_liveness` credits domain/URL/web-origin liveness from
  `targets` / web-origin observations; `eas_fingerprint_services` credits
  SERVICE-FINGERPRINT only when the confirmed-open ports have port-level service
  surface in `targets.ports`, or a matching `source=nmap` fingerprint exists for
  the same target and port; `eas_fingerprint_web_stack` / WhatWeb credits
  WEB-FINGERPRINT and lands Target/Fingerprints/web-origin rows. SERVICE-FINGERPRINT
  found is strict at the port level: every SERVICE-applicable confirmed-open
  `targets.ports[]` entry for the IP must carry
  service/version/product/banner/webserver/technologies-style data or a
  port-scoped nmap fingerprint. Weak names like `tcpwrapped` are not strong
  service identity, but the matching nmap row is a terminal attempt for that
  port; bare DNS/53 is ignored on multi-service hosts and DNS-only hosts can
  converge as not_applicable. WhatWeb fingerprints are not a substitute for nmap
  service/version on the IP:port surface.
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
