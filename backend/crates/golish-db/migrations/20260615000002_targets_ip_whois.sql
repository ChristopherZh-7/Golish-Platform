-- Host-aware coverage 2c-3 (design 2026-06-15-host-aware-coverage-2c3-ip-native):
-- per-IP RIR/netblock WHOIS (RDAP /ip/), the store backing the IP-native
-- GOLISH-INTEL-IPWHOIS coverage cell. Distinct from organizations.whois (domain
-- RDAP, org-level) and organizations.asns (IP->ASN, org-level).
--
-- Expand-first / backward-compatible (AGENTS.md I10): nullable, no backfill.
-- Reads treat NULL / 'null' / '{}' as empty (shape-agnostic, like has_whois).
-- Suggested shape: { netname, org, country, cidr, abuse, source, raw_ref }
ALTER TABLE targets ADD COLUMN IF NOT EXISTS ip_whois JSONB;
