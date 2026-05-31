//! Static markdown templates and the per-language PoC scaffolding.
//!
//! Centralised here so future schema/PoC tweaks don't require editing the
//! command modules. The PoC builder is a pure function — easy to unit test
//! and reuse from new commands later.

/// Top-level wiki categories — directory names under `<wiki>/`.
///
/// `infer_category_from_path` snaps any unrecognised path back to
/// `"uncategorized"` so the dashboard always has a valid bucket.
pub(in crate::wiki) const WIKI_CATEGORIES: &[&str] =
    &["products", "techniques", "pocs", "experience", "analysis"];

pub(super) const SCHEMA_MD: &str = r#"# Vulnerability Knowledge Base Schema

This wiki is structured to help the AI agent find exploit methods, PoCs, and research findings during penetration testing.

## Directory Structure

- **products/** — Per-product/component knowledge (e.g., `products/apache-log4j/`)
- **techniques/** — Attack techniques and methodology (e.g., `techniques/ssrf/`)
- **pocs/** — Proof-of-concept code and exploit scripts
- **experience/** — Past engagement notes, lessons learned
- **analysis/** — Deep-dive vulnerability analysis, root cause write-ups

## Page Conventions

Each page should include YAML-style frontmatter:

```
---
title: <descriptive title>
category: products|techniques|pocs|experience|analysis
tags: [tag1, tag2, ...]
cves: [CVE-XXXX-XXXXX, ...]
status: draft|partial|complete|needs-poc|verified
---
```

### Status Values

| Status | Meaning |
|--------|---------|
| `draft` | Just created, basic skeleton with CVE data only |
| `partial` | Some research done, missing exploit details or PoC |
| `complete` | Comprehensive: exploitation method + PoC + detection |
| `needs-poc` | Analysis complete but no working PoC available |
| `verified` | Tested and confirmed in actual engagement |

Followed by markdown content with actionable knowledge the agent can use.
"#;

pub(super) const INDEX_MD_HEADER: &str = r#"# Vulnerability Knowledge Base

> Auto-generated dashboard. Edited by the system — do not modify manually.

"#;

pub(super) const LOG_MD_HEADER: &str = "# Knowledge Base Change Log\n\n";

/// Render the README + PoC scaffolding for a freshly-created CVE folder.
///
/// Returns `(readme_markdown, poc_filename, poc_source)`. `poc_lang` falls
/// back to `"py"` when not provided. Unknown languages get a generic comment
/// stub so callers still end up with a writable file.
pub(super) fn cve_scaffold(
    cve_id: &str,
    title: &str,
    poc_lang: Option<&str>,
) -> (String, String, String) {
    let readme = format!(
        "# {cve_id}: {title}\n\n\
         ## 概述\n\n\
         <!-- 漏洞描述 -->\n\n\
         ## 影响范围\n\n\
         - 产品/版本:\n\
         - CVSS:\n\
         - 类型:\n\n\
         ## 复现步骤\n\n\
         1. \n\n\
         ## POC\n\n\
         参见 `poc` 文件。\n\n\
         ## 修复建议\n\n\
         <!-- 修复方案 -->\n\n\
         ## 参考\n\n\
         - https://nvd.nist.gov/vuln/detail/{cve_id}\n"
    );

    let lang = poc_lang.unwrap_or("py");
    let poc_name = format!("poc.{lang}");
    let poc_content = match lang {
        "py" => format!(
            "#!/usr/bin/env python3\n\
             \"\"\"POC for {cve_id}: {title}\"\"\"\n\n\
             import requests\nimport sys\n\n\
             def exploit(target: str):\n\
             \x20   # TODO: implement\n\
             \x20   pass\n\n\
             if __name__ == \"__main__\":\n\
             \x20   if len(sys.argv) < 2:\n\
             \x20       print(f\"Usage: {{sys.argv[0]}} <target>\")\n\
             \x20       sys.exit(1)\n\
             \x20   exploit(sys.argv[1])\n"
        ),
        "go" => format!(
            "package main\n\n\
             // POC for {cve_id}: {title}\n\n\
             import (\n\t\"fmt\"\n\t\"net/http\"\n\t\"os\"\n)\n\n\
             func exploit(target string) error {{\n\
             \t// TODO: implement\n\
             \treturn nil\n\
             }}\n\n\
             func main() {{\n\
             \tif len(os.Args) < 2 {{\n\
             \t\tfmt.Fprintf(os.Stderr, \"Usage: %s <target>\\n\", os.Args[0])\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             \tif err := exploit(os.Args[1]); err != nil {{\n\
             \t\tfmt.Fprintln(os.Stderr, err)\n\
             \t\tos.Exit(1)\n\
             \t}}\n\
             }}\n"
        ),
        "sh" | "bash" => format!(
            "#!/usr/bin/env bash\n\
             # POC for {cve_id}: {title}\n\n\
             set -euo pipefail\n\n\
             TARGET=\"${{1:?Usage: $0 <target>}}\"\n\n\
             # TODO: implement\n\
             echo \"[*] Target: $TARGET\"\n"
        ),
        _ => format!("// POC for {cve_id}: {title}\n// TODO: implement\n"),
    };

    (readme, poc_name, poc_content)
}
