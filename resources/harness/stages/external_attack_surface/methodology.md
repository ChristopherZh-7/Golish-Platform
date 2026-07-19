**Goal:** actively map the attack surface of the APPROVED hosts — establish
liveness for inherited domain/URL/vhost assets, and establish open ports plus
service/version fingerprints only for concrete IP host assets. CIDR rows own the
range sweep; guarded child IP rows own later SERVICE/WEB work. Subdomains are
INHERITED from `target_intel`; do NOT re-enumerate them. This is the first stage
that touches the target, gated by `active_scan` approval.

**Execution contract (`relation != authorization`):**

| Authorized identity / confirmed surface | Required work | Capability | Durable truth |
|---|---|---|---|
| Domain, URL, or vhost | HTTP(S) LIVENESS for the exact name/origin | `eas_probe_http_liveness` | compatibility fields in `targets`, exact `web_origins`, and `web_origin_observations` |
| Independently authorized IP | PORT discovery; the scan also establishes IP liveness | `eas_discover_ports` | `targets.ports` and IP:port `network_endpoints` |
| Independently authorized CIDR | Range LIVENESS + PORT only | `eas_discover_ports` | range outcome plus guarded in-range child IP targets/endpoints for a supplemental wave |
| Authorized concrete IP with confirmed open ports | SERVICE/version fingerprint per open port | `eas_fingerprint_services` | enriched `targets.ports`, `network_endpoints`, and nmap `fingerprints` |
| Confirmed exact `scheme://host:port` web origin | WEB-FINGERPRINT | `eas_fingerprint_web_stack` | `web_origins`, `web_origin_observations`, and WhatWeb `fingerprints` |

`dns_records` and `targets.real_ip` describe relationships/cache only. A DNS-only,
provider-observed, CDN, or shared IP must never be passed to
`eas_discover_ports` or `eas_fingerprint_services` unless an org-bound in-scope
IP/CIDR target independently authorizes it. Record each terminal attempt in the
evidence ledger / `technique_outcomes` for the exact asset and technique.

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
     an independently authorized IP/CIDR target exists, scan that concrete target
     once for its applicable work: IP owns PORT/SERVICE, while CIDR owns only
     range LIVENESS/PORT and emits child IPs for later SERVICE/WEB. A `real_ip` or
     `dns_records` edge alone is not scan
     authority. PORT/SERVICE are not applicable to the domain identity itself;
     never make EAS invent or promote an IP target from a relationship fact.
   - `url`: probe URL liveness; do not assign PORT/SERVICE to the path URL.
   - `cidr`: treat as a range. Get approval when required and sweep it through
     `eas_discover_ports`. The CIDR row itself closes only LIVENESS/PORT. The
     guarded output store creates only concrete in-range child IP targets with
     CIDR provenance; those children enter a supplemental wave and own later
     SERVICE/WEB work. Never fingerprint the CIDR string as a service host.
   - `wildcard`: passive authorization pattern only. The pattern row has all EAS
     techniques `not_applicable`; never resolve, probe, brute-force, or scan it.
     Concrete strict-child domain targets discovered in Intel carry their own work.
4. Liveness + HTTP probe — for concrete IP/CIDR assets, prefer
   `eas_discover_ports` first; do not use `eas_probe_http_liveness` as a generic
   IP-alive test. Use `eas_probe_http_liveness` for inherited domain/URL/vhost
   assets, or for confirmed/likely web origins where HTTP metadata is the next
   missing fact.
   The wrapper owns the fixed `httpx` recipe and batching; call it with
   `targets=[...]` instead of building raw `httpx` / `pentest_run` args. DNS was
   already done in `target_intel`; reuse inherited DNS as routing/context instead
   of re-running `dig`, but never as implicit IP scan authorization.
   If a liveness probe is terminal-empty, mark only LIVENESS checked_empty with
   evidence; do not claim domain PORT/SERVICE work because those cells belong to
   concrete IP/CIDR hosts.
5. Port discovery — call `eas_discover_ports` for every explicit IP/CIDR target
   that is still runnable.
   Its optional scanner enum only selects a backend-owned recipe; raw
   `naabu`/`masscan`/`nmap` tools and command arguments are never exposed to the
   model. The wrapper owns the list-file recipe and rejects domains/URLs. Every
   IP/CIDR range row must have a fresh port terminal result; discovered child IPs
   continue in their own supplemental wave.
6. Service/version fingerprint — call `eas_fingerprint_services` only for
   concrete IP targets with confirmed open ports. Normally pass only `targets[]`:
   the wrapper reads the exact DB-owned pending ports for each IP, splits them
   into bounded chunks, runs small chunks concurrently, isolates slow IPs, and
   performs at most one smaller recovery pass for ports whose complete XML did
   not land. Optional `ports[]` can only narrow the pending set; callers cannot
   expand the scan surface or override server-owned deadlines. It rejects raw
   domains, URL strings, and CIDR ranges. Do not regroup IPs by matching port
   sets, raise a timeout, or blindly replay a timed-out batch. Close blocked concrete-host
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
   (`scheme://host:port`) with that origin's Host/SNI because each vhost can expose
   a different stack. Do not fold those observations into the shared IP origin.
   A normal WhatWeb response with no plugin tail is a real exact-origin
   `checked_empty`. By contrast, an ANSI-stripped
   `ERROR Opening: <exact-origin> - <reason>` increments the operation-epoch
   breaker only for the backend's narrow, target-attributed transport/TLS
   grammar. Attempts 1 and 2 publish guarded `error` outcomes and remain
   retryable. The third consecutive same-class failure seals only the WhatWeb
   producer for that exact target/origin and short-circuits later same-epoch
   WhatWeb calls before network launch. A new EAS epoch is required to reopen it.
   Ruby/runtime/configuration failures, unknown or unattributed stderr, missing
   batch members and truncated output never increment the breaker and remain
   unfinished; never describe those as checked-empty or blocked.
   For a WEB-FINGERPRINT coverage/worklist gap, copy
   `details.recommended_args.target_urls` directly when present. Otherwise pair
   the gap's `target_id` with each exact `details.missing_origins` string and pass
   `{target_id,target_url}` entries. Copy those exact origins unchanged: never
   guess, infer, or rewrite the scheme from the port number, including port 443.

**If a tool is missing or errors:**

- Record it in `skipped_checks` with the reason and use another EAS wrapper
  fallback where possible (for example, `eas_discover_ports(scanner="nmap")` for
  concrete IPs when `naabu` is unavailable), not unresolved domains.
- Do NOT install tools, spawn extra sub-agents, or retry a blocked/missing tool in
  a loop. Note it and move on — "checked_empty" is NOT "unchecked".
- If a WhatWeb batch has valid output or exactly attributed transport failures
  for every member, the wrapper keeps successful siblings and records each
  failed origin independently. Attempts 1/2 remain `error`; attempt 3 becomes
  producer-owned `blocked` only after its guarded evidence chain is durable.
  Any unrecognized stderr or unaccounted member keeps the batch non-terminal;
  do not hand-write a parent blocked exception.
- On attempt 3 the backend also runs its fixed, tool-independent direct/proxy
  HEAD then bounded GET-Range policy. Any HTTP response keeps the origin eligible
  for Enumeration even though WhatWeb remains sealed for this EAS epoch. Only
  when every available independent transport/TLS attempt fails does the backend
  publish the trusted exact-origin handoff that removes that origin from the next
  stage. Never delete the target/origin, rewrite an open port as closed, or
  suppress sibling Host/SNI origins.
- EAS wrappers are forced foreground and return only after guarded business rows
  and evidence land. Do not use background job controls or relaunch a wrapper
  because its prior result has not yet been interpreted; retry only a named
  partial/error cell with a bounded batch. The four wrappers bypass the generic
  sub-agent 300-second outer timeout and remain cancellable through User Stop;
  SERVICE fingerprinting defaults each underlying nmap batch to a bounded
  600-second command budget unless the caller explicitly supplies a smaller one.
- The four `eas_*` capabilities are the complete model-facing active tool surface.
  `httpx`, `naabu`, `masscan`, `nmap`, WhatWeb and `pentest_run` are internal
  engines/recipes, not tools the AI may call directly.

**Coverage + stop condition:**

- The gate reads database truth for found GOLISH-EAS-LIVENESS /
  GOLISH-EAS-PORT / GOLISH-EAS-SERVICE-FINGERPRINT /
  GOLISH-EAS-WEB-FINGERPRINT cells. `eas_discover_ports` credits port truth from
  `targets.ports`/guarded range outcomes and also closes the explicit IP/CIDR
  row's LIVENESS from the same scan;
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
- Before submitting, call `check_stage_asset_coverage`. If a real negative or
  blocker cannot be derived from DB truth, construct exact
  `terminal_exceptions` (`checked_empty` + exact-technique evidence, or
  `blocked/not_applicable` + concrete note) and pass the same array to the next
  preflight. Only when it returns `ready_to_submit=true`, copy
  `terminal_exceptions_preview.coverage_to_submit` unchanged into the final
  deliverable and submit once. `status=accepted` ends the stage immediately.
  This exception path does **not** apply to a parent
  `GOLISH-EAS-WEB-FINGERPRINT` cell while `details.missing_origins` is non-empty:
  preflight rejects that entry and keeps the exact origins actionable. Refresh
  `stage_worklist_next`, copy its DB-backed `{target_id,target_url}` entries
  unchanged, and close every remaining origin through
  `eas_fingerprint_web_stack`; an asset-level note cannot replace origin truth.
  After the authoritative per-org gate passes, blocked/not-applicable cells are
  materialized without overwriting producer-owned terminal truth.
- If `stage_run` returns `retry_budget_exhausted=true`, stop this top-level
  request after recording the deterministic BLOCK. Do not open another automatic
  repair/submit turn. A later explicit user continuation receives a fresh bounded
  budget and may resume the durable worker chain.
- Do not jump into content enumeration or vuln scanning — the harness advances
  to `enumeration` for you.
