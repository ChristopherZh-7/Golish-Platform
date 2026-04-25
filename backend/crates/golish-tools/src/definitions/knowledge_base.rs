use serde_json::json;
use super::FunctionDeclaration;

pub fn knowledge_base_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "search_knowledge_base".to_string(),
            description: "Search the vulnerability knowledge base for exploit methods, PoCs, product analysis, attack techniques, and past engagement experience. Uses PostgreSQL full-text search. Supports filtering by category and tag. Use this before attempting any exploit to check if relevant knowledge already exists.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query — natural language or keywords (e.g. 'log4j RCE exploit', 'SSRF bypass techniques', 'Apache Tomcat CVE')"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["products", "techniques", "pocs", "experience", "analysis"],
                        "description": "Optional: filter by knowledge category"
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional: filter by tag (e.g. 'rce', 'ssrf', 'java')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (default: 10)"
                    }
                },
                "required": ["query"]
            }),
        },
        FunctionDeclaration {
            name: "write_knowledge".to_string(),
            description: "Write or update a page in the vulnerability knowledge base. The page is stored as a markdown file and indexed in PostgreSQL for full-text search. Use YAML frontmatter for metadata (title, category, tags, cves, status). Status values: draft (skeleton), partial (some research), complete (exploit+PoC+detection), needs-poc (analysis done, no PoC), verified (tested in engagement). IMPORTANT: Always update the status field to reflect completeness. Pass cve_id to auto-link the page to a CVE.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Wiki path (e.g. 'products/apache-log4j/CVE-2021-44228.md', 'techniques/ssrf/cloud-metadata.md')"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full markdown content including YAML frontmatter. Frontmatter should include: title, category (products|techniques|pocs|experience|analysis), tags (array), and optionally cves (array)."
                    },
                    "cve_id": {
                        "type": "string",
                        "description": "Optional: CVE identifier to auto-link this page to (e.g. 'CVE-2021-44228'). When provided, the page will appear in the Wiki tab for that CVE."
                    }
                },
                "required": ["path", "content"]
            }),
        },
        FunctionDeclaration {
            name: "read_knowledge".to_string(),
            description: "Read a specific page from the vulnerability knowledge base by its path. Returns the full markdown content including frontmatter.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Wiki path to read (e.g. 'products/apache-log4j/CVE-2021-44228.md')"
                    }
                },
                "required": ["path"]
            }),
        },
        FunctionDeclaration {
            name: "ingest_cve".to_string(),
            description: "Create a knowledge base entry for a CVE. Looks up the CVE in the vulnerability intelligence database, creates a structured wiki page under products/ with all known information, and links the CVE to the page. Use when encountering a new CVE that needs to be researched and documented.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cve_id": {
                        "type": "string",
                        "description": "CVE identifier (e.g. 'CVE-2021-44228')"
                    },
                    "product": {
                        "type": "string",
                        "description": "Product or component name (e.g. 'apache-log4j'). Used to organize the wiki path."
                    },
                    "additional_context": {
                        "type": "string",
                        "description": "Optional: extra analysis, notes, or exploit details to include in the page beyond what the CVE database provides."
                    }
                },
                "required": ["cve_id", "product"]
            }),
        },
        FunctionDeclaration {
            name: "save_poc".to_string(),
            description: "Save a PoC (Proof of Concept) to the knowledge base, linked to a specific CVE. Supports rich metadata: source (nuclei_template, github, exploitdb, manual), severity, description, and tags. Use this for all PoC types — Nuclei templates discovered during fingerprinting, GitHub exploit scripts, ExploitDB entries, or manually crafted scripts.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cve_id": {
                        "type": "string",
                        "description": "CVE identifier to link this PoC to (e.g. 'CVE-2021-44228')"
                    },
                    "name": {
                        "type": "string",
                        "description": "Descriptive name for the PoC (e.g. 'Log4Shell JNDI RCE Exploit', 'Nuclei Detection Template')"
                    },
                    "poc_type": {
                        "type": "string",
                        "enum": ["nuclei", "script", "manual"],
                        "description": "Type of PoC: 'nuclei' for Nuclei YAML templates, 'script' for executable scripts, 'manual' for manual testing procedures"
                    },
                    "language": {
                        "type": "string",
                        "description": "Programming language or format (e.g. 'yaml', 'python', 'bash', 'go', 'markdown')"
                    },
                    "content": {
                        "type": "string",
                        "description": "The full PoC content (code, template, or testing instructions)"
                    },
                    "source": {
                        "type": "string",
                        "enum": ["nuclei_template", "github", "exploitdb", "manual"],
                        "description": "Where this PoC was sourced from"
                    },
                    "source_url": {
                        "type": "string",
                        "description": "URL to the original PoC source (GitHub repo, ExploitDB page, Nuclei template path)"
                    },
                    "severity": {
                        "type": "string",
                        "enum": ["critical", "high", "medium", "low", "info", "unknown"],
                        "description": "Severity of the vulnerability this PoC targets"
                    },
                    "description": {
                        "type": "string",
                        "description": "Brief description of what this PoC does and its prerequisites"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tags for categorization (e.g. ['rce', 'authenticated', 'apache', 'java'])"
                    }
                },
                "required": ["cve_id", "name", "poc_type", "language", "content"]
            }),
        },
        FunctionDeclaration {
            name: "list_cves_with_pocs".to_string(),
            description: "List all CVE identifiers that have at least one PoC in the knowledge base, sorted by severity. Returns a summary per CVE: number of PoCs, max severity, verification status, and whether research/wiki pages exist. Use this to see what PoCs have been collected and which CVEs still need research.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        FunctionDeclaration {
            name: "list_unresearched_cves".to_string(),
            description: "List CVEs that have PoCs but have NOT been researched yet. This is the priority queue for the 'PoC first, research later' workflow — these are actionable CVEs with known exploit paths that need investigation. Results are sorted by severity (critical first). Use after collecting PoCs via Nuclei/fingerprinting to decide what to research next.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of CVEs to return (default 20)"
                    }
                },
                "required": []
            }),
        },
        FunctionDeclaration {
            name: "poc_stats".to_string(),
            description: "Get statistics about the PoC knowledge base: counts by source (nuclei_template, github, etc.), counts by severity, total unique CVEs covered, and number of verified PoCs. Use to gauge coverage and identify gaps.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    ]
}
