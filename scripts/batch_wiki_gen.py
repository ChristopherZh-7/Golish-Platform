#!/usr/bin/env python3
"""Batch-generate wiki pages for CVEs that have PoC but no wiki."""

import json
import os
import re
import sys
import textwrap
from datetime import datetime
from pathlib import Path

import psycopg2
import psycopg2.extras

DB_DSN = "postgres://golish:golish_local@localhost:15432/golish"
WIKI_ROOT = Path.home() / "Library" / "Application Support" / "golish-platform" / "wiki"
BATCH_SIZE = 200

TECHNIQUE_MAP = {
    "sqli": ("sql-injection", "SQL Injection"),
    "sql-injection": ("sql-injection", "SQL Injection"),
    "rce": ("remote-code-execution", "Remote Code Execution"),
    "remote-code-execution": ("remote-code-execution", "Remote Code Execution"),
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
    "default-credentials": ("default-credentials", "Default Credentials"),
    "backdoor": ("backdoor-command-execution", "Backdoor / Command Execution"),
    "auth-bypass": ("authentication-bypass", "Authentication Bypass"),
    "authentication-bypass": ("authentication-bypass", "Authentication Bypass"),
    "information-disclosure": ("information-disclosure", "Information Disclosure"),
    "path-traversal": ("path-traversal", "Path Traversal"),
    "log4j": ("jndi-injection", "JNDI Injection (Log4j)"),
    "jndi": ("jndi-injection", "JNDI Injection"),
    "shellshock": ("command-injection", "Command Injection (Shellshock)"),
    "buffer-overflow": ("memory-corruption-buffer-overflow", "Memory Corruption / Buffer Overflow"),
}


def slug_from_poc(poc: dict) -> str:
    """Derive a product slug from PoC metadata."""
    desc = poc.get("poc_description", "") or ""
    tags = poc.get("tags", []) or []
    cve_id = poc["cve_id"]

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

    for t in tags:
        if t.startswith("cve") or t in ("vuln", "intrusive", "vkev", "kev",
                                         "oast", "oob", "network", "tcp",
                                         "http", "unauth", "authenticated",
                                         "php", "wp", "wp-plugin", "wordpress"):
            continue
        slug_candidate = re.sub(r"[^a-z0-9]+", "-", t.lower()).strip("-")
        if len(slug_candidate) > 2:
            return slug_candidate

    return "unknown"


def detect_techniques(poc: dict) -> list[tuple[str, str]]:
    """Return list of (slug, display_name) technique matches."""
    tags = [t.lower() for t in (poc.get("tags") or [])]
    desc = (poc.get("poc_description") or "").lower()
    found = {}

    for tag in tags:
        if tag in TECHNIQUE_MAP:
            slug, name = TECHNIQUE_MAP[tag]
            found[slug] = name

    for kw, (slug, name) in TECHNIQUE_MAP.items():
        if kw in desc:
            found[slug] = name

    if not found:
        if any(t in tags for t in ("rce", "intrusive")):
            found["remote-code-execution"] = "Remote Code Execution"
        else:
            found["command-injection"] = "Command Injection"

    return list(found.items())


def build_wiki_content(cve_id: str, poc: dict, vuln: dict | None) -> str:
    """Generate markdown wiki page content."""
    desc_text = poc.get("poc_description", "") or cve_id
    tags = poc.get("tags", []) or []
    severity = poc.get("severity", "unknown")
    product_slug = slug_from_poc(poc)
    techniques = detect_techniques(poc)

    prod_display = desc_text.split(" - ")[0].strip() if " - " in desc_text else product_slug.replace("-", " ").title()
    vuln_title = desc_text if " - " in desc_text else f"{prod_display} - {severity.title()} Vulnerability"

    title = f"{cve_id} - {vuln_title}" if cve_id not in vuln_title else vuln_title
    if len(title) > 120:
        title = title[:117] + "..."

    tag_list = sorted(set([product_slug] + [severity] + [t for t in tags if t not in ("cve",)]))
    tag_str = ", ".join(tag_list)

    tech_refs = "\n".join(
        f"- [{name}](../../techniques/{slug}.md)" for slug, name in techniques
    )
    primary_tech = techniques[0] if techniques else None
    primary_ref = f"\nPrimary technique reference: [{primary_tech[1]}](../../techniques/{primary_tech[0]}.md).\n" if primary_tech else ""

    vuln_desc = ""
    affected = ""
    refs_section = ""
    if vuln:
        if vuln.get("description"):
            vuln_desc = vuln["description"][:1000]
        if vuln.get("affected_products"):
            prods = vuln["affected_products"]
            if isinstance(prods, str):
                prods = json.loads(prods) if prods.startswith("[") else [prods]
            affected = ", ".join(prods[:5]) if prods else ""
        if vuln.get("refs"):
            refs = vuln["refs"]
            if isinstance(refs, str):
                refs = json.loads(refs) if refs.startswith("[") else [refs]
            refs_section = "\n".join(f"- [{r}]({r})" for r in refs[:6])

    if not vuln_desc:
        vuln_desc = desc_text

    if not refs_section:
        ref_urls = []
        if cve_id.startswith("CVE-"):
            ref_urls.append(f"https://nvd.nist.gov/vuln/detail/{cve_id}")
        poc_url = poc.get("poc_source_url", "")
        if poc_url:
            ref_urls.append(poc_url)
        refs_section = "\n".join(f"- [{u}]({u})" for u in ref_urls)

    poc_source_url = poc.get("poc_source_url", "")
    poc_source = poc.get("poc_source", "manual")
    poc_name = poc.get("poc_name", cve_id)
    poc_type = poc.get("poc_type", "nuclei")
    poc_lang = poc.get("poc_language", "yaml")

    content = f"""---
title: "{title}"
category: products
tags: [{tag_str}]
cves: [{cve_id}]
status: partial
---

# {title}

## Overview

{vuln_desc}
{primary_ref}
## Affected Products/Versions

{f"Known affected: {affected}" if affected else f"Affected product: {prod_display}. Verify exact affected versions against vendor advisories before production remediation."}

## Exploitation Conditions

- The target runs an affected {prod_display} deployment.
- The vulnerable endpoint or protocol surface is reachable from the tester or attacker network.
- Severity: **{severity}**

## Existing PoC Notes

Existing PoC in the local knowledge base:

- **Name**: `{poc_name}`
- **Type**: `{poc_type}`
- **Language**: `{poc_lang}`
- **Source**: `{poc_source}`
- **Source URL**: `{poc_source_url}`
- **Severity**: `{severity}`

## Detection

- Confirm the product and affected version before running active checks.
- Prefer non-destructive validation using version evidence, benign response markers, or controlled out-of-band callbacks.
- Correlate scanner output with vendor advisories and local exposure.

## Remediation

- Update to the latest patched version of {prod_display}.
- Restrict the vulnerable endpoint to trusted networks until the fix is applied.
- Review logs for indicators of prior exploitation.

## Related Techniques

{tech_refs}

## References

{refs_section}
"""
    return content.strip() + "\n"


def ensure_technique_page(slug: str, name: str, cve_id: str, conn):
    """Create or update technique page if needed."""
    tech_path = WIKI_ROOT / "techniques" / f"{slug}.md"
    wiki_path = f"techniques/{slug}.md"

    if tech_path.exists():
        existing = tech_path.read_text()
        if cve_id not in existing:
            fm_match = re.search(r"^cves:\s*\[([^\]]*)\]", existing, re.MULTILINE)
            if fm_match:
                old_cves = fm_match.group(1)
                new_cves = f"{old_cves}, {cve_id}" if old_cves.strip() else cve_id
                updated = existing.replace(f"cves: [{old_cves}]", f"cves: [{new_cves}]")
                tech_path.write_text(updated)
                upsert_wiki_page(conn, wiki_path, name, "techniques",
                                 [slug], "partial", updated)
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

{name} is an attack technique used in vulnerability exploitation.

## Methodology

(To be enriched by AI research)

## Known CVEs

- {cve_id}

## Detection & Prevention

(To be enriched by AI research)
"""
    tech_path.parent.mkdir(parents=True, exist_ok=True)
    tech_path.write_text(content)
    upsert_wiki_page(conn, wiki_path, name, "techniques",
                     [slug], "draft", content)


def upsert_wiki_page(conn, path: str, title: str, category: str,
                     tags: list[str], status: str, content: str):
    """Insert or update wiki_pages record."""
    word_count = len(content.split())
    with conn.cursor() as cur:
        cur.execute("""
            INSERT INTO wiki_pages (path, title, category, tags, status, content, word_count)
            VALUES (%s, %s, %s, %s, %s, %s, %s)
            ON CONFLICT (path) DO UPDATE SET
                title = EXCLUDED.title,
                category = EXCLUDED.category,
                tags = EXCLUDED.tags,
                status = EXCLUDED.status,
                content = EXCLUDED.content,
                word_count = EXCLUDED.word_count,
                updated_at = NOW()
        """, (path, title, category, tags, status, content, word_count))


def link_cve_wiki(conn, cve_id: str, wiki_path: str):
    """Insert vuln_kb_links record."""
    with conn.cursor() as cur:
        cur.execute("""
            INSERT INTO vuln_kb_links (cve_id, wiki_path)
            VALUES (%s, %s)
            ON CONFLICT (cve_id, wiki_path) DO NOTHING
        """, (cve_id, wiki_path))


def main():
    conn = psycopg2.connect(DB_DSN)
    conn.autocommit = False

    try:
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute("""
                SELECT
                    p.cve_id,
                    p.name as poc_name,
                    p.poc_type,
                    p.language as poc_language,
                    p.source as poc_source,
                    p.source_url as poc_source_url,
                    p.severity,
                    p.description as poc_description,
                    p.tags
                FROM vuln_kb_pocs p
                WHERE NOT EXISTS(SELECT 1 FROM vuln_kb_links l WHERE l.cve_id = p.cve_id)
                AND p.severity = 'critical'
                GROUP BY p.cve_id, p.name, p.poc_type, p.language, p.source,
                         p.source_url, p.severity, p.description, p.tags
                ORDER BY
                    CASE p.severity
                        WHEN 'critical' THEN 1
                        WHEN 'high' THEN 2
                        WHEN 'medium' THEN 3
                        ELSE 4
                    END
                LIMIT %s
            """, (BATCH_SIZE,))
            pocs = cur.fetchall()

        print(f"Found {len(pocs)} CVEs to process")

        cve_ids = [p["cve_id"] for p in pocs]
        vuln_map = {}
        with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
            cur.execute("""
                SELECT cve_id, title, description, sev, cvss_score,
                       published, source, refs, affected_products
                FROM vuln_entries
                WHERE cve_id = ANY(%s)
            """, (cve_ids,))
            for row in cur.fetchall():
                vuln_map[row["cve_id"]] = row

        created = 0
        errors = 0
        for poc in pocs:
            cve_id = poc["cve_id"]
            try:
                product_slug = slug_from_poc(poc)
                wiki_path = f"products/{product_slug}/{cve_id}.md"
                file_path = WIKI_ROOT / "products" / product_slug / f"{cve_id}.md"

                if file_path.exists():
                    link_cve_wiki(conn, cve_id, wiki_path)
                    conn.commit()
                    continue

                vuln = vuln_map.get(cve_id)
                content = build_wiki_content(cve_id, dict(poc), vuln)

                file_path.parent.mkdir(parents=True, exist_ok=True)
                file_path.write_text(content)

                desc_text = poc.get("poc_description", "") or cve_id
                vuln_title = desc_text if " - " in desc_text else f"{product_slug} - {cve_id}"
                page_title = f"{cve_id} - {vuln_title}" if cve_id not in vuln_title else vuln_title
                tags_list = sorted(set([product_slug, poc.get("severity", "critical")]
                                       + list(poc.get("tags") or [])))

                upsert_wiki_page(conn, wiki_path, page_title[:200], "products",
                                 tags_list, "partial", content)
                link_cve_wiki(conn, cve_id, wiki_path)

                techniques = detect_techniques(poc)
                for tech_slug, tech_name in techniques:
                    ensure_technique_page(tech_slug, tech_name, cve_id, conn)

                conn.commit()
                created += 1
                if created % 20 == 0:
                    print(f"  ... processed {created}/{len(pocs)}")

            except Exception as e:
                conn.rollback()
                errors += 1
                print(f"  ERROR [{cve_id}]: {e}", file=sys.stderr)

        print(f"\nDone: {created} wiki pages created, {errors} errors, {len(pocs) - created - errors} skipped")

    finally:
        conn.close()


if __name__ == "__main__":
    main()
