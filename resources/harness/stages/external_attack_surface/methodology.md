**Goal:** actively map the attack surface of the APPROVED hosts — for every
in-scope asset establish (1) liveness, (2) open ports, (3) service/version
fingerprint. Subdomains are INHERITED from `target_intel`; do NOT re-enumerate
them. This is the first stage that touches the target, gated by `active_scan`
approval.

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
   - `domain` / `ip`: scan ports and establish liveness.
   - `url`: probe URL liveness; do not assign PORT/SERVICE to the path URL.
   - `cidr`: treat as a range. Get approval when required, sweep it, register
     discovered live IPs as concrete targets, then scan each IP.
   - wildcard: do not brute-force here; concrete inherited hosts carry the work.
4. Liveness + HTTP fingerprint — batch host/url targets through `httpx` early
   when it will reduce uncertainty or give you HTTP evidence. This is normally the
   cheapest way to separate live web assets from dead leads, but it is not a hard
   ordering rule: if the DB already has fresh liveness or a targeted port batch is
   clearly the right next gap, use judgment.
   Prefer one JSONL run per org/worklist chunk: call `pentest_run` with
   `tool_name="httpx"`, `args="-json -sc -title -td -server -silent"`, and
   newline-separated targets in `input_lines` / `stdin` instead of one `httpx -u`
   call per host. DNS was already done in `target_intel`; reuse inherited DNS
   instead of re-running `dig`.
   If a liveness probe is terminal-empty, mark only LIVENESS checked_empty with
   evidence; do not automatically claim PORT/SERVICE is complete unless you have
   run or intentionally closed those applicable cells.
5. Port discovery — batch concrete host/IP/CIDR targets with `naabu` / `masscan`
   / `nmap` where the tool accepts list input. With `pentest_run`, put
   `{{input_file}}` in `args` and pass the actual targets via `input_lines`:
   `naabu -list {{input_file}} ...`, `masscan -iL {{input_file}} ...`, or
   `nmap -iL {{input_file}} ...`. Every IP or host must have a fresh port-scan
   terminal result. Do not feed URL strings such as `https://1.2.3.4/path` to
   `nmap -iL`; normalize to concrete host/IP targets or close URL-only
   PORT/SERVICE cells as not_applicable with a note.
6. Service/version fingerprint — prefer `nmap -sV` only for confirmed open
   ports. Group hosts that share the same confirmed port set with
   `-iL {{input_file}}` + `-p <confirmed-open-ports>` instead of launching one
   foreground command per host/port. Do not run `nmap -sV -iL` over the raw
   in-scope domain/IP list, unresolved hosts, or assets that have no open-port
   evidence; close those SERVICE cells as `not_applicable` / `blocked` with a
   concrete note. Use `whatweb --input-file={{input_file}}` only for HTTP(S)
   services when its Ruby runtime is ready.
   If `whatweb` returns a runtime/SSL/opening error, record the failed attempt,
   do not retry it on the same host, and continue with `nmap -sV` / `httpx`
   evidence. NEVER assume a service from the port number alone (8080 is not
   proof of Tomcat), and never treat HTTP liveness alone as PORT/SERVICE
   coverage.
7. (Optional) `gowitness file -f {{input_file}}` screenshots of live web
   services for the record.

**If a tool is missing or errors:**

- Record it in `skipped_checks` with the reason and use a fallback (e.g. if `httpx`
  is unavailable, use `nmap -sV` / `nmap -Pn -p- --open` for liveness+service).
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
- Before submitting, call `check_stage_asset_coverage`. If
  `ready_to_submit=false`, use its `gap_examples` to close the missing
  asset-technique cells by grouping gaps with the same technique/tool into batch
  probes where possible. For list-file tools use `{{input_file}}` in
  `pentest_run.args` and provide targets through `input_lines`.
  `pentest_list_tools.params` is the parameter catalog; skills are examples, not
  fixed call signatures. Use honest
  checked_empty/blocked/not_applicable coverage when a tool cannot run. Submit
  only when the preflight says
  `ready_to_submit=true`. This is a required self-check before submit, not a
  trial submit: do not call `submit_stage_deliverable` just to discover missing
  cells.
- Do not jump into content enumeration or vuln scanning — the harness advances
  to `enumeration` for you.
