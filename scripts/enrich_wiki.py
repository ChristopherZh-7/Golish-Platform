#!/usr/bin/env python3
"""Enrich partial wiki pages using nuclei template content from DB."""

import json
import os
import re
import sys
import yaml
from pathlib import Path

import psycopg2
import psycopg2.extras

DB_DSN = "postgres://golish:golish_local@localhost:15432/golish"
WIKI_ROOT = Path.home() / "Library" / "Application Support" / "golish-platform" / "wiki"


def parse_nuclei_yaml(content: str) -> dict:
    """Parse nuclei template YAML, handling raw HTTP blocks gracefully."""
    try:
        data = yaml.safe_load(content)
        if isinstance(data, dict):
            return data
    except yaml.YAMLError:
        pass

    info_match = re.search(r"^info:\s*\n((?:  .+\n|\n)+)", content, re.MULTILINE)
    result = {"info": {}}
    if info_match:
        try:
            info_block = "info:\n" + info_match.group(1)
            parsed = yaml.safe_load(info_block)
            if isinstance(parsed, dict):
                result["info"] = parsed.get("info", {})
        except yaml.YAMLError:
            pass

    http_match = re.search(r"^http:\s*\n([\s\S]+?)(?=^[a-z]|\Z)", content, re.MULTILINE)
    if http_match:
        result["_raw_http"] = http_match.group(1)

    network_match = re.search(r"^(?:tcp|network):\s*\n([\s\S]+?)(?=^[a-z]|\Z)", content, re.MULTILINE)
    if network_match:
        result["_raw_network"] = network_match.group(1)

    return result


def extract_http_payloads(raw: str) -> list[str]:
    """Extract raw HTTP request payloads from nuclei template."""
    payloads = []
    blocks = re.findall(r'\|\s*\n((?:        .+\n)+)', raw)
    for block in blocks:
        cleaned = "\n".join(line[8:] if line.startswith("        ") else line
                           for line in block.rstrip().split("\n"))
        if cleaned.strip():
            payloads.append(cleaned.strip())
    return payloads


def extract_matchers(raw: str) -> list[str]:
    """Extract matcher DSL conditions."""
    matchers = []
    dsl_blocks = re.findall(r"dsl:\s*\n((?:\s+- .+\n)+)", raw)
    for block in dsl_blocks:
        for line in block.strip().split("\n"):
            m = re.match(r"\s+- ['\"](.+)['\"]", line.strip())
            if m:
                matchers.append(m.group(1))
    return matchers


TECHNIQUE_MAP = {
    "sqli": ("sql-injection", "SQL Injection"),
    "sql-injection": ("sql-injection", "SQL Injection"),
    "rce": ("remote-code-execution", "Remote Code Execution"),
    "xxe": ("xxe", "XML External Entity Injection"),
    "lfi": ("path-traversal-lfi", "Path Traversal / LFI"),
    "file-upload": ("arbitrary-file-upload", "Arbitrary File Upload"),
    "fileupload": ("arbitrary-file-upload", "Arbitrary File Upload"),
    "ssrf": ("ssrf", "Server-Side Request Forgery"),
    "ssti": ("ssti", "Server-Side Template Injection"),
    "xss": ("xss", "Cross-Site Scripting"),
    "ognl": ("ognl-injection", "OGNL Injection"),
    "deserialization": ("deserialization", "Deserialization"),
    "command-injection": ("command-injection", "Command Injection"),
    "injection": ("command-injection", "Command Injection"),
    "default-login": ("default-credentials", "Default Credentials"),
    "backdoor": ("backdoor-command-execution", "Backdoor / Command Execution"),
    "auth-bypass": ("authentication-bypass", "Authentication Bypass"),
    "log4j": ("jndi-injection", "JNDI Injection (Log4j)"),
    "jndi": ("jndi-injection", "JNDI Injection"),
    "shellshock": ("command-injection", "Command Injection (Shellshock)"),
    "path-traversal": ("path-traversal", "Path Traversal"),
    "information-disclosure": ("information-disclosure", "Information Disclosure"),
}


def detect_techniques(tags: list[str], desc: str) -> list[tuple[str, str]]:
    found = {}
    for tag in tags:
        t = tag.lower()
        if t in TECHNIQUE_MAP:
            slug, name = TECHNIQUE_MAP[t]
            found[slug] = name
    desc_lower = desc.lower()
    for kw, (slug, name) in TECHNIQUE_MAP.items():
        if kw in desc_lower:
            found[slug] = name
    if not found:
        found["remote-code-execution"] = "Remote Code Execution"
    return list(found.items())


def cwe_description(cwe_id: str) -> str:
    """Return human-readable CWE description for common CWEs."""
    cwe_map = {
        "CWE-78": "OS Command Injection",
        "CWE-79": "Cross-site Scripting (XSS)",
        "CWE-89": "SQL Injection",
        "CWE-94": "Code Injection",
        "CWE-98": "PHP Remote File Inclusion",
        "CWE-119": "Buffer Overflow",
        "CWE-200": "Information Exposure",
        "CWE-264": "Permissions, Privileges, and Access Controls",
        "CWE-287": "Improper Authentication",
        "CWE-306": "Missing Authentication for Critical Function",
        "CWE-352": "Cross-Site Request Forgery (CSRF)",
        "CWE-434": "Unrestricted Upload of File with Dangerous Type",
        "CWE-502": "Deserialization of Untrusted Data",
        "CWE-611": "XML External Entity (XXE) Processing",
        "CWE-787": "Out-of-bounds Write",
        "CWE-798": "Use of Hard-coded Credentials",
        "CWE-917": "Expression Language Injection",
        "CWE-918": "Server-Side Request Forgery (SSRF)",
    }
    return cwe_map.get(cwe_id, cwe_id)


def build_enriched_page(cve_id: str, poc_row: dict, vuln_entry: dict | None) -> str:
    """Generate a comprehensive wiki page from nuclei template + vuln entry."""
    content_raw = poc_row.get("content", "")
    template = parse_nuclei_yaml(content_raw)
    info = template.get("info", {})

    name = info.get("name", poc_row.get("poc_description", cve_id))
    description = info.get("description", "").strip()
    impact = info.get("impact", "").strip()
    remediation_text = info.get("remediation", "").strip()
    references = info.get("reference", []) or []
    if isinstance(references, str):
        references = [references]

    classification = info.get("classification", {}) or {}
    cvss_metrics = classification.get("cvss-metrics", "")
    cvss_score = classification.get("cvss-score", "")
    cwe_id = classification.get("cwe-id", "")
    epss_score = classification.get("epss-score", "")
    epss_pct = classification.get("epss-percentile", "")
    cpe = classification.get("cpe", "")

    metadata = info.get("metadata", {}) or {}
    verified = metadata.get("verified", False)
    product = metadata.get("product", "")
    vendor = metadata.get("vendor", "")
    fofa_query = metadata.get("fofa-query", "")
    shodan_query = metadata.get("shodan-query", "")
    zoomeye_query = metadata.get("zoomeye-query", "")

    tags = info.get("tags", "")
    if isinstance(tags, str):
        tags = [t.strip() for t in tags.split(",") if t.strip()]

    severity = info.get("severity", poc_row.get("severity", "critical"))

    # Extract attack payloads
    raw_http = template.get("_raw_http", "")
    raw_network = template.get("_raw_network", "")
    payloads = extract_http_payloads(raw_http) if raw_http else []
    matchers = extract_matchers(raw_http or raw_network) if (raw_http or raw_network) else []

    # Enrich from vuln_entry if available
    vuln_desc = ""
    vuln_refs = []
    affected_products = ""
    if vuln_entry:
        vuln_desc = vuln_entry.get("description", "")
        refs_raw = vuln_entry.get("refs", [])
        if isinstance(refs_raw, str):
            try:
                refs_raw = json.loads(refs_raw)
            except json.JSONDecodeError:
                refs_raw = []
        vuln_refs = refs_raw if isinstance(refs_raw, list) else []
        prods = vuln_entry.get("affected_products", [])
        if isinstance(prods, str):
            try:
                prods = json.loads(prods)
            except json.JSONDecodeError:
                prods = []
        if isinstance(prods, list):
            affected_products = ", ".join(sorted(set(str(p) for p in prods[:8])))

    full_description = description or vuln_desc or f"{name} vulnerability"
    if impact and impact not in full_description:
        full_description += f"\n\n**Impact:** {impact}"

    prod_display = product.replace("-", " ").title() if product else name.split(" - ")[0].strip()
    techniques = detect_techniques(tags, full_description)
    primary_tech = techniques[0] if techniques else None

    tech_refs = "\n".join(f"- [{n}](../../techniques/{s}.md)" for s, n in techniques)
    primary_ref = f"\nPrimary technique reference: [{primary_tech[1]}](../../techniques/{primary_tech[0]}.md).\n" if primary_tech else ""

    # Build references
    all_refs = []
    if cve_id.startswith("CVE-"):
        all_refs.append(f"https://nvd.nist.gov/vuln/detail/{cve_id}")
    all_refs.extend(references)
    all_refs.extend(vuln_refs)
    poc_url = poc_row.get("poc_source_url", "")
    if poc_url and poc_url not in all_refs:
        all_refs.append(poc_url)
    seen = set()
    unique_refs = []
    for r in all_refs:
        if isinstance(r, str) and r.strip() and r not in seen:
            seen.add(r)
            unique_refs.append(r)
    refs_md = "\n".join(f"- [{r}]({r})" for r in unique_refs[:10])

    # Classification section
    classification_section = ""
    if cvss_score or cwe_id or epss_score:
        lines = []
        if cvss_score:
            lines.append(f"- **CVSS Score**: {cvss_score} ({severity})")
        if cvss_metrics:
            lines.append(f"- **CVSS Vector**: `{cvss_metrics}`")
        if cwe_id:
            lines.append(f"- **CWE**: {cwe_id} — {cwe_description(cwe_id)}")
        if epss_score:
            lines.append(f"- **EPSS Score**: {epss_score} (percentile: {epss_pct})")
        if cpe:
            lines.append(f"- **CPE**: `{cpe}`")
        classification_section = "\n## Classification\n\n" + "\n".join(lines)

    # Attack chain / payload section
    payload_section = ""
    if payloads:
        payload_section = "\n## Attack Chain / Payload\n\n"
        payload_section += "The following HTTP request(s) demonstrate the exploitation:\n\n"
        for i, p in enumerate(payloads[:3]):
            payload_section += f"```http\n{p}\n```\n\n"
        if matchers:
            payload_section += "**Detection matchers**:\n"
            for m in matchers[:5]:
                payload_section += f"- `{m}`\n"

    # Detection fingerprint section
    detection_section = "## Detection\n\n"
    if fofa_query or shodan_query or zoomeye_query:
        detection_section += "### Asset Discovery\n\n"
        if fofa_query:
            detection_section += f"- **FOFA**: `{fofa_query}`\n"
        if shodan_query:
            detection_section += f"- **Shodan**: `{shodan_query}`\n"
        if zoomeye_query:
            detection_section += f"- **ZoomEye**: `{zoomeye_query}`\n"
        detection_section += "\n"

    detection_section += """### Validation Guidelines

- Confirm the product and affected version before running active checks.
- Prefer non-destructive validation using version evidence, benign response markers, or controlled out-of-band callbacks.
- Correlate scanner output with vendor advisories and local exposure; one negative test does not prove the product is patched.
- Avoid collecting secrets, executing arbitrary commands, or leaving uploaded artifacts during validation."""

    if matchers and not payloads:
        detection_section += "\n\n### Matcher Conditions\n\n"
        for m in matchers[:5]:
            detection_section += f"- `{m}`\n"

    # Remediation section
    remediation_section = "## Remediation\n\n"
    if remediation_text:
        remediation_section += remediation_text + "\n"
    else:
        remediation_section += f"- Update to the latest patched version of {prod_display}.\n"
    remediation_section += f"- Restrict the vulnerable endpoint to trusted networks until the fix is applied.\n"
    remediation_section += f"- Review logs and filesystem/process indicators for prior exploitation before treating remediation as complete."

    # Affected products
    affected_section = ""
    if cpe:
        affected_section = f"The local PoC metadata maps this issue to {prod_display} with CPE `{cpe}`. "
    if affected_products:
        affected_section += f"Known affected: {affected_products}. "
    affected_section += "Verify exact affected versions against NVD and vendor advisories before production remediation."

    # PoC Notes
    poc_type = poc_row.get("poc_type", "nuclei")
    poc_lang = poc_row.get("poc_language", "yaml")
    poc_source = poc_row.get("poc_source", "nuclei_template")
    poc_name = poc_row.get("poc_name", cve_id)

    poc_notes = f"""## Existing PoC Notes

Existing PoC in the local knowledge base:

- **Name**: `{poc_name}`
- **Type**: `{poc_type}`
- **Language**: `{poc_lang}`
- **Source**: `{poc_source}`
- **Source URL**: `{poc_url}`
- **Severity**: `{severity}`
- **Verified locally**: `{str(verified).lower()}`"""

    if poc_type == "nuclei":
        poc_notes += f"""

The stored nuclei template uses {"HTTP request flow" if raw_http else "network protocol"}, {"intrusive" if "intrusive" in tags else "non-intrusive"} validation to check for vulnerable behavior and response markers. {"It should be treated as intrusive and run only in explicitly authorized scope." if "intrusive" in tags else "Use it as detection guidance."}"""

    # Determine status
    status = "complete" if (verified or len(unique_refs) >= 3) else "needs-poc" if not payloads else "partial"

    tag_list = sorted(set(tags + [poc_row.get("severity", "critical")]))
    tag_str = ", ".join(tag_list)

    title = f"{cve_id} - {name}" if cve_id not in name else name

    page = f"""---
title: "{title}"
category: products
tags: [{tag_str}]
cves: [{cve_id}]
status: {status}
---

# {title}

## Overview

{full_description}
{primary_ref}
## Affected Products/Versions

{affected_section}

## Exploitation Conditions

- The target runs an affected {prod_display} deployment.
- The vulnerable endpoint or protocol surface is reachable from the tester or attacker network.
{f"- Attacker-controlled input reaches an expression, template, eval-like, or code execution path." if any(t in tags for t in ("rce", "ssti", "ognl", "injection")) else "- The attacker can reach the vulnerable component without additional authentication." if any(t in tags for t in ("unauth",)) else "- Exploitation may require valid credentials or prior access."}
{classification_section}
{payload_section}
{poc_notes}

{detection_section}

{remediation_section}

## Related Techniques

{tech_refs}

## References

{refs_md}
"""
    return page.strip() + "\n"


def slug_from_poc(poc: dict, template_info: dict) -> str:
    """Derive product slug from nuclei metadata or PoC data."""
    metadata = template_info.get("info", {}).get("metadata", {}) or {}
    product = metadata.get("product", "")
    vendor = metadata.get("vendor", "")

    if vendor and product:
        slug = f"{vendor}-{product}".lower()
        slug = re.sub(r"[^a-z0-9]+", "-", slug).strip("-")
        if len(slug) > 3:
            return slug
    if product:
        slug = re.sub(r"[^a-z0-9]+", "-", product.lower()).strip("-")
        if len(slug) > 3:
            return slug

    cve_id = poc["cve_id"]
    desc = poc.get("poc_description", "") or ""

    if cve_id.startswith("NUCLEI-"):
        parts = cve_id.replace("NUCLEI-", "").lower().split("-")
        first_known = []
        for p in parts:
            if p in ("sqli", "rce", "lfi", "ssrf", "xss", "ssti", "log4j",
                      "backdoor", "default", "login", "unauth", "fileupload",
                      "file", "upload", "auth", "bypass", "install",
                      "detection", "config", "exposure"):
                break
            first_known.append(p)
        if first_known:
            return "-".join(first_known)

    prod_parts = desc.split(" - ")[0].strip() if " - " in desc else ""
    if prod_parts:
        slug = re.sub(r"[^a-z0-9]+", "-", prod_parts.lower()).strip("-")
        if len(slug) > 3:
            return slug

    return "unknown"


def upsert_wiki_page(conn, path, title, category, tags, status, content):
    word_count = len(content.split())
    with conn.cursor() as cur:
        cur.execute("""
            INSERT INTO wiki_pages (path, title, category, tags, status, content, word_count)
            VALUES (%s, %s, %s, %s, %s, %s, %s)
            ON CONFLICT (path) DO UPDATE SET
                title = EXCLUDED.title, category = EXCLUDED.category,
                tags = EXCLUDED.tags, status = EXCLUDED.status,
                content = EXCLUDED.content, word_count = EXCLUDED.word_count,
                updated_at = NOW()
        """, (path, title, category, tags, status, content, word_count))


def ensure_technique_page(slug, name, cve_id, conn):
    tech_path = WIKI_ROOT / "techniques" / f"{slug}.md"
    wiki_path = f"techniques/{slug}.md"
    if tech_path.exists():
        existing = tech_path.read_text()
        if cve_id not in existing:
            fm = re.search(r"^cves:\s*\[([^\]]*)\]", existing, re.MULTILINE)
            if fm:
                old = fm.group(1)
                new = f"{old}, {cve_id}" if old.strip() else cve_id
                updated = existing.replace(f"cves: [{old}]", f"cves: [{new}]")
                tech_path.write_text(updated)
                upsert_wiki_page(conn, wiki_path, name, "techniques", [slug], "partial", updated)
        return

    content = f"""---
title: "{name}"
category: techniques
tags: [{slug}]
cves: [{cve_id}]
status: draft
---

# {name}

## Overview

{name} is an attack technique exploited across multiple vulnerabilities.

## Methodology

(To be enriched by detailed research)

## Known CVEs

- {cve_id}

## Detection & Prevention

(To be enriched by detailed research)
"""
    tech_path.parent.mkdir(parents=True, exist_ok=True)
    tech_path.write_text(content)
    upsert_wiki_page(conn, wiki_path, name, "techniques", [slug], "draft", content)


def main():
    conn = psycopg2.connect(DB_DSN)
    conn.autocommit = False

    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute("""
                SELECT l.cve_id, l.wiki_path
                FROM vuln_kb_links l
                JOIN wiki_pages wp ON wp.path = l.wiki_path
                WHERE wp.status = 'partial'
                  AND wp.category = 'products'
                ORDER BY wp.created_at DESC
                LIMIT 200
            """)
            partial_pages = cur.fetchall()

        print(f"Found {len(partial_pages)} partial pages to enrich")

        cve_ids = list(set(p["cve_id"] for p in partial_pages))

        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute("""
                SELECT cve_id, name as poc_name, poc_type, language as poc_language,
                       source as poc_source, source_url as poc_source_url,
                       severity, description as poc_description, tags, content
                FROM vuln_kb_pocs
                WHERE cve_id = ANY(%s)
            """, (cve_ids,))
            poc_map = {}
            for row in cur.fetchall():
                poc_map[row["cve_id"]] = row

        vuln_map = {}
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute("""
                SELECT cve_id, title, description, sev, cvss_score,
                       published, source, refs, affected_products
                FROM vuln_entries WHERE cve_id = ANY(%s)
            """, (cve_ids,))
            for row in cur.fetchall():
                vuln_map[row["cve_id"]] = row

        enriched = 0
        errors = 0
        for page in partial_pages:
            cve_id = page["cve_id"]
            wiki_path = page["wiki_path"]
            try:
                poc = poc_map.get(cve_id)
                if not poc:
                    continue

                template = parse_nuclei_yaml(poc.get("content", ""))
                product_slug = slug_from_poc(dict(poc), template)
                correct_path = f"products/{product_slug}/{cve_id}.md"

                vuln = vuln_map.get(cve_id)
                content = build_enriched_page(cve_id, dict(poc), vuln)

                file_path = WIKI_ROOT / correct_path
                file_path.parent.mkdir(parents=True, exist_ok=True)
                file_path.write_text(content)

                info = template.get("info", {})
                title_text = f"{cve_id} - {info.get('name', poc.get('poc_description', cve_id))}"
                all_tags = info.get("tags", "")
                if isinstance(all_tags, str):
                    all_tags = [t.strip() for t in all_tags.split(",") if t.strip()]
                all_tags = sorted(set(all_tags + [poc.get("severity", "critical")]))

                verified = (info.get("metadata", {}) or {}).get("verified", False)
                refs = info.get("reference", []) or []
                status = "complete" if (verified or len(refs) >= 3) else "needs-poc"

                upsert_wiki_page(conn, correct_path, title_text[:200], "products",
                                 all_tags, status, content)

                if correct_path != wiki_path:
                    old_file = WIKI_ROOT / wiki_path
                    if old_file.exists():
                        old_file.unlink()
                    with conn.cursor() as cur2:
                        cur2.execute("DELETE FROM wiki_pages WHERE path = %s", (wiki_path,))
                        cur2.execute("""
                            UPDATE vuln_kb_links SET wiki_path = %s
                            WHERE cve_id = %s AND wiki_path = %s
                        """, (correct_path, cve_id, wiki_path))

                desc = poc.get("poc_description", "") or ""
                techniques = detect_techniques(all_tags, desc)
                for tech_slug, tech_name in techniques:
                    ensure_technique_page(tech_slug, tech_name, cve_id, conn)

                conn.commit()
                enriched += 1
                if enriched % 20 == 0:
                    print(f"  ... enriched {enriched}/{len(partial_pages)}")

            except Exception as e:
                conn.rollback()
                errors += 1
                print(f"  ERROR [{cve_id}]: {e}", file=sys.stderr)

        print(f"\nDone: {enriched} pages enriched, {errors} errors")

    finally:
        conn.close()


if __name__ == "__main__":
    main()
