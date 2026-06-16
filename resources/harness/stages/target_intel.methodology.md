**Goal:** build the passive, ZERO-TOUCH intelligence picture of the in-scope
roots — asset inventory, subdomains, DNS/whois/ASN/CT, historical URLs — WITHOUT
sending any packet that touches the target's own hosts. Liveness/port/service
checks are NOT done here; they belong to `external_attack_surface` (EAS).

You run for ONE organization — the `stage_run` fan-out dispatches one Recon per
org, so collect only THIS org's footprint and register its assets as in-scope
targets bound to this `organization_id`.

**Recommended sequence (run each technique ONCE per in-scope root):**

1. `recon_enrich_assets` first — ASM/intel providers (quake / 0.zone / enscan)
   return org, ICP, subdomains, and asset fields in one shot. This is the cheapest,
   richest source; do it before any CLI tool.
2. Passive subdomain enumeration — `subfinder -all -recursive` and/or
   `amass enum -passive`. Run each ONCE on the root domain. Merge + dedupe results.
3. URL history — `gau` / `waybackurls` on the root for historical endpoints.
4. OSINT / WHOIS / ASN / CT / DNS / SUBDOMAIN land via `recon_enrich_assets`
   (step 1): one call writes the data the gate reads — subdomains →
   `target_assets`, DNS A/AAAA → `dns_records`, ASN/CT/WHOIS →
   `organizations.asns/.certificates/.whois` (CT/WHOIS have a crt.sh/RDAP fallback
   when providers return nothing), OSINT → `organizations.intel`. **OSINT is a
   REQUIRED coverage technique** (`GOLISH-INTEL-OSINT`), not optional — confirm the
   enrich actually produced OSINT data for this org. If a technique genuinely has no
   data (no provider/credential, nothing in CT/RDAP), record it `blocked+note` with
   the reason; never silently skip it and never fabricate it.
5. CT / ASN fallback CLIs (zero-touch) — if `recon_enrich_assets` did NOT land CT or
   ASN for a root (its crt.sh / RDAP fallback returned nothing, or no provider was
   available), run the dedicated passive CLI as a backstop so the cell is not left
   not_attempted: `ctfr -d <root>` (certificate-transparency subdomains →
   `GOLISH-INTEL-CT`) and/or `asnmap -d <root>` (ASN + netblock ranges →
   `GOLISH-INTEL-ASN`). Both query crt.sh / the RIRs — not the target's own hosts —
   so they are in-scope in this passive stage, and their output auto-projects to the
   CT / ASN coverage cell. Run each ONCE per root, and only when that cell would
   otherwise stay empty. If the CLI is not installed, record the cell `blocked+note`
   — do NOT try to install it mid-stage.

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
- Do NOT pipe tool output through `| head` / `| tail` or otherwise truncate it —
  truncated output cannot be parsed and will NOT land in the database the gate reads.
- Do NOT reuse one technique's evidence for another cell. Each coverage cell must
  cite evidence produced by THAT technique's own run (DNS evidence backs only the
  DNS cell, CT evidence only CT, …). Citing the same evidence_id across DNS / ASN /
  CT / OSINT is fabricated coverage and the gate's corroboration check rejects it —
  this is the #1 cause of repeated `needs_fix`.

**Coverage + submission (this stage reads coverage from the DATABASE):**

- target_intel coverage is adjudicated from DB truth. Once a technique actually RAN
  and its data LANDED (subdomains → `target_assets`, DNS → `dns_records`,
  ASN/CT/WHOIS → `organizations.asns/.certificates/.whois`, OSINT →
  `organizations.intel`), the gate marks that (asset × technique) cell `found` on its
  own. You do NOT need to hand-write `found` cells or cite their evidence_ids — the
  platform reads them from the DB. Your job is to make each technique truly run/land.
- `submit_stage_deliverable` is therefore a thin checkpoint. Put in `coverage` ONLY
  the cells the DB cannot derive:
  - `checked_empty` + evidence_refs — you actually ran the technique and it returned
    nothing (NOT "unchecked"; this is the I8 distinction and still needs the probe
    evidence id).
  - `blocked` / `not_applicable` + note — no provider/credential, or it does not apply.
  Leave `found` cells out (the DB supplies them); `claims` may be empty; put real
  vulnerabilities (rare in passive intel) in `findings`.
- Stop condition: once providers + one pass of passive subdomain + url-history have
  run AND every expected technique has either landed in the DB or been recorded as
  checked_empty / blocked / not_applicable, call `submit_stage_deliverable` ONCE. Do
  not loop hand-building a big matrix — the gate reads the DB, not your self-report.
