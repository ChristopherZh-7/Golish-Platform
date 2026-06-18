**Goal:** build the passive, ZERO-TOUCH intelligence picture of the in-scope
roots — asset inventory, subdomains, DNS/whois/ASN/CT, historical URLs — WITHOUT
sending any packet that touches the target's own hosts. Liveness/port/service
checks are NOT done here; they belong to `external_attack_surface` (EAS).

You run for ONE organization — the `stage_run` fan-out dispatches one Recon per
org, so collect only THIS org's footprint and register its assets as in-scope
targets bound to this `organization_id`.

**Recommended sequence (provider survey first, then WHOIS; CLI tools fill the rest):**

1. `recon_map_assets` first AND as the main path — ASM/intel providers
   (quake / 0.zone / fofa / hunter / shodan / enscan) return org, ICP, subdomains,
   ASN, certificates and asset fields in one shot. Since the passive-intel-closure
   (Phase A/B), it also pairs each discovered domain with its surveyed IP,
   scope-filters it, and lands it as an in-scope target carrying that `real_ip` —
   discovery becomes landing without a second tool. It writes the data the gate
   reads: subdomains → `target_assets`, ASN → `organizations.asns`, certificates →
   `organizations.certificates`, OSINT → `organizations.intel`. This is the
   cheapest, richest source; do it before any CLI tool. **OSINT is a REQUIRED
   coverage technique** (`GOLISH-INTEL-OSINT`) — confirm the survey produced OSINT
   data for this org; if a technique genuinely has no data (no provider/credential),
   record it `blocked+note` — never silently skip or fabricate.
2. `recon_lookup_whois` — RDAP WHOIS, ONCE per org across its registrable domains,
   lands `organizations.whois` (the `GOLISH-INTEL-WHOIS` cell). Fast and zero-touch.
3. CLI tools fill the cells the survey did NOT land (zero-touch) — reach for them
   ONLY for an empty coverage cell, at most ONCE per root: SUBDOMAIN →
   `subfinder -all` / `amass enum -passive`; CT → `ctfr -d <root>`; ASN →
   `asnmap -d <root>`; DNS → `dig` (per in-scope domain with no record yet). These
   query crt.sh / the RIRs / resolvers — not the target's own hosts — so they stay
   in-scope here, and their output auto-projects to the matching coverage cell. If a
   tool still returns nothing, submit that cell as `checked_empty+evidence` or
   `blocked+note` — do NOT retry the same tool, and do NOT install a missing tool
   mid-stage (record it `blocked+note`).
4. URL history (`gau` / `waybackurls`) — optional, for historical endpoints; it is
   orthogonal to the survey and is NOT a completeness-gate requirement.

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
