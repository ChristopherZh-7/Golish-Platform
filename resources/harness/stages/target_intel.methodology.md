**Goal:** build the passive, ZERO-TOUCH intelligence picture of the in-scope
roots — asset inventory, subdomains, DNS/whois/ASN/CT, historical URLs — WITHOUT
sending any packet that touches the target's own hosts. Liveness/port/service
checks are NOT done here; they belong to `external_attack_surface` (EAS).

**Multi-org engagements — fan out with `stage_run`:** If this engagement has more
than one in-scope organization (a parent plus subsidiaries you built during
scoping), call `stage_run` with the full organization tree
(`orgs: [{ id: organization_id, name, ownership_percent }]`) instead of collecting
every org yourself or dispatching `sub_agent_recon` per org by hand. `stage_run`
runs the Recon specialist once per org — each isolated to its own
`organization_id` and gated on its own evidence — and returns `{ passed, gaps[] }`.
If `passed` is false, call `stage_run` again with `orgs` set to ONLY the blocked
org(s), and repeat until every org passes (the gate-closure loop); ask the human
only if an org keeps failing. Use the single-org sequence below directly only for a
one-organization engagement (or when `stage_run` is unavailable).

**Recommended sequence (run each technique ONCE per in-scope root):**

1. `recon_enrich_assets` first — ASM/intel providers (quake / 0.zone / enscan)
   return org, ICP, subdomains, and asset fields in one shot. This is the cheapest,
   richest source; do it before any CLI tool.
2. Passive subdomain enumeration — `subfinder -all -recursive` and/or
   `amass enum -passive`. Run each ONCE on the root domain. Merge + dedupe results.
3. URL history — `gau` / `waybackurls` on the root for historical endpoints.
4. OSINT / WHOIS / ASN / CT land via `recon_enrich_assets` (step 1): ENScan returns
   org records / contacts / social accounts / business systems (= OSINT);
   quake / 0.zone return ASN / CT / WHOIS. **OSINT is a REQUIRED coverage technique**
   (`GOLISH-INTEL-OSINT`, read from `organizations.intel`), not optional — confirm
   the enrich actually produced OSINT data for this org. If your providers returned
   no OSINT (no provider/credential), record OSINT `blocked+note` with the reason;
   never silently skip it.

**Efficiency red lines (these are the common failure modes):**

- Resume — skip already-done work: before collecting, call `list_in_scope_targets`
  and check the org's existing assets / ledger (`search_knowledge_base`). For any
  in-scope target already at `passive` or later (this stage already ran for it), do
  NOT re-collect — reuse the prior evidence. Re-run only the assets/techniques that
  still lack a terminal coverage status.
- Run each passive tool ONCE per root, then move on. Do NOT re-run subfinder/amass
  repeatedly with different flags hunting for more.
- Do NOT `dig` every discovered subdomain one-by-one. Per-host A-record resolution
  and liveness is EAS's job (httpx does it in one batch). Resolving 200 hosts with
  200 `dig` calls here is wasted work and is out of this stage's purpose.
- Do NOT run `nmap` / port scans / `httpx` live probing — those touch the target
  and are blocked here. If you feel the urge to "verify a host is up", STOP: that
  belongs to EAS, which inherits your subdomain evidence.

**Coverage + stop condition:**

- For each in-scope asset, give each expected intel technique (DNS / WHOIS / ASN /
  CT / SUBDOMAIN / OSINT) a terminal status in `coverage`: found+evidence_refs, or
  checked_empty+evidence_refs (you actually ran it and it was empty — NOT the same
  as "unchecked"), or blocked/not_applicable+note.
- Once providers + one pass of passive subdomain + url-history have run and the
  asset list is recorded, fill coverage and `submit_stage_deliverable`. Do not loop.
