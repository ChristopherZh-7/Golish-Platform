---
title: ACME Corp (acme.com) - Subsidiary & Org Structure Reconnaissance
category: products
tags:
  - recon
  - osint
  - red-team-scope
  - acme-corp
status: draft
---

# ACME Corp (acme.com) - Organizational Reconnaissance

## Overview

**Primary Domain:** acme.com
**Purpose:** Identify subsidiaries, business units, divisions, and affiliated organizations for red-team scoping.

> **⚠️ Status: DRAFT — Requires live OSINT.** This page was created without web search capabilities. Manual research is needed to populate findings.

---

## Research Methodology

### Phase 1: Public Website Enumeration

1. **Crawl acme.com** — Look for:
   - "About Us" / "Our Companies" / "Investor Relations" pages
   - Footer links to subsidiary sites
   - Press releases mentioning acquisitions
   - Careers pages listing different divisions/locations

2. **Sitemap & robots.txt** — Check `acme.com/sitemap.xml` and `acme.com/robots.txt` for subdomains/paths

### Phase 2: Corporate Filings & SEC Records

1. **SEC EDGAR** (https://www.sec.gov/cgi-bin/browse-edgar) — Search for:
   - 10-K annual reports (list all subsidiaries in Exhibit 21)
   - 8-K filings (acquisitions, mergers)
   - Proxy statements (DEF 14A) for organizational structure

2. **State corporate registries** — Search secretary of state databases for entities registered by/acquired by ACME Corp

3. **OpenCorporates** (https://opencorporates.com) — Search for "Acme" related entities

### Phase 3: Domain & DNS Reconnaissance

1. **Certificate Transparency Logs** — Search crt.sh for `acme.com` to find affiliated domains/subdomains
2. **DNS enumeration** — Look for MX records, NS records pointing to different providers
3. **WHOIS historical data** — DomainTools / WHOIS history for related registrations
4. **Reverse WHOIS** — Find domains registered with the same org/emails

Search queries to run:
```
crt.sh: %.acme.com
SecurityTrails: *.acme.com subdomains
Shodan: org:"ACME" ssl.cert.subject.cn
```

### Phase 4: OSINT & Business Intelligence

1. **LinkedIn** — Search for:
   - "ACME Corp" company page → check "Associated Pages" / subsidiaries
   - Employee titles with "Division", "Unit", "Subsidiary"
   - Employees listing different company names under ACME umbrella

2. **Crunchbase** — Search for ACME Corp → acquisitions, subsidiaries, investments

3. **Bloomberg / PitchBook** — Corporate structure and ownership

4. **Google Dorks**:
   ```
   site:acme.com "subsidiary" OR "division" OR "acquired"
   "acme corp" "subsidiary" filetype:pdf
   "acme" "wholly owned" OR "affiliate"
   ```

### Phase 5: Infrastructure Fingerprinting

1. **Shodan / Censys** — Search by org name, SSL certs, ASN
2. **ASN lookup** — Find IP ranges owned by ACME Corp → reverse DNS for other domains
3. **Cloud tenant enumeration** — Check for Azure/O365 tenant names

---

## Known Findings

*To be populated after live research:*

### Confirmed Subsidiaries
| Name | Domain | Relationship | Source |
|------|--------|-------------|--------|
| *(research needed)* | | | |

### Business Divisions/Units
| Division | Description | Source |
|----------|-------------|--------|
| *(research needed)* | | | |

### Affiliated Domains
| Domain | Purpose | Relationship |
|--------|---------|-------------|
| *(research needed)* | | |

### Acquired Companies
| Company | Year Acquired | Domain | Status |
|---------|--------------|--------|--------|
| *(research needed)* | | | |

### Infrastructure / IP Ranges
| ASN | IP Range | Description |
|-----|----------|-------------|
| *(research needed)* | | |

---

## Notes for Red Team Scoping

- Each identified subsidiary/domain should be verified with the client before inclusion in scope
- Cloud-hosted properties (e.g., SaaS products) may have different infrastructure teams
- Recently acquired companies may still operate separate IT infrastructure
- Brand/trademark entities may not have direct internet-facing assets