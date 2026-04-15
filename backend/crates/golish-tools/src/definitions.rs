//! Tool function declarations for LLM consumption.
//!
//! This module provides the `build_function_declarations()` function that returns
//! tool schemas in the format expected by LLM providers.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Function declaration format for LLM tool calling.
///
/// This struct matches the format expected by vtcode_core::tools::registry::FunctionDeclaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDeclaration {
    /// Tool name (must match what the tool registry expects)
    pub name: String,
    /// Human-readable description for the LLM
    pub description: String,
    /// JSON Schema for the tool's parameters
    pub parameters: Value,
}

/// Build all tool declarations for LLM consumption.
///
/// This is a drop-in replacement for vtcode_core::tools::registry::build_function_declarations().
///
/// Returns a vector of function declarations that describe all available tools
/// and their parameter schemas.
pub fn build_function_declarations() -> Vec<FunctionDeclaration> {
    vec![
        // ====================================================================
        // File Operations
        // ====================================================================
        FunctionDeclaration {
            name: "read_file".to_string(),
            description: "Read the contents of a file. Supports optional line range for reading specific sections.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to workspace)"
                    },
                    "line_start": {
                        "type": "integer",
                        "description": "Starting line number (1-indexed)"
                    },
                    "line_end": {
                        "type": "integer",
                        "description": "Ending line number (1-indexed, inclusive)"
                    }
                },
                "required": ["path"]
            }),
        },
        FunctionDeclaration {
            name: "write_file".to_string(),
            description: "Write content to a file, replacing existing content. Creates the file and parent directories if they don't exist.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to workspace)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        FunctionDeclaration {
            name: "create_file".to_string(),
            description: "Create a new file with the specified content. Fails if the file already exists.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path for the new file (relative to workspace)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Initial content for the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        FunctionDeclaration {
            name: "edit_file".to_string(),
            description: "Edit a file by replacing text. The old_text must match exactly once in the file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file (relative to workspace)"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Text to find and replace (must match exactly once)"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text"
                    },
                    "display_description": {
                        "type": "string",
                        "description": "Human-readable description of the edit"
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        },
        FunctionDeclaration {
            name: "delete_file".to_string(),
            description: "Delete a file from the filesystem.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to delete (relative to workspace)"
                    }
                },
                "required": ["path"]
            }),
        },
        // ====================================================================
        // Directory Operations
        // ====================================================================
        FunctionDeclaration {
            name: "list_files".to_string(),
            description: "List files matching a glob pattern. Respects .gitignore by default.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to search (relative to workspace, default: root)"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files (e.g., '*.rs', '**/*.ts')"
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Search recursively (default: true)"
                    }
                },
                "required": []
            }),
        },
        FunctionDeclaration {
            name: "list_directory".to_string(),
            description: "List the contents of a directory with file/directory type indicators.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (relative to workspace)"
                    }
                },
                "required": ["path"]
            }),
        },
        FunctionDeclaration {
            name: "grep_file".to_string(),
            description: "Search file contents using regex pattern. Returns matching lines with file paths and line numbers.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search (default: workspace root)"
                    },
                    "include": {
                        "type": "string",
                        "description": "Glob pattern to filter files (e.g., '*.rs')"
                    }
                },
                "required": ["pattern"]
            }),
        },
        // ====================================================================
        // Planning
        // ====================================================================
        FunctionDeclaration {
            name: "update_plan".to_string(),
            description: "Create or update the task plan. Use this to track progress on multi-step tasks. Each step should have a description and status (pending, in_progress, or completed). Only one step can be in_progress at a time.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "explanation": {
                        "type": "string",
                        "description": "Optional high-level explanation or summary of the plan"
                    },
                    "plan": {
                        "type": "array",
                        "description": "List of plan steps (1-12 steps)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": {
                                    "type": "string",
                                    "description": "Description of this step"
                                },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Current status of the step"
                                }
                            },
                            "required": ["step"]
                        }
                    }
                },
                "required": ["plan"]
            }),
        },
        // ====================================================================
        // Shell Execution
        // ====================================================================
        FunctionDeclaration {
            name: "run_pty_cmd".to_string(),
            description: "Execute a shell command and return the output. Commands run in a shell environment with access to common tools.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory (relative to workspace)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 120)"
                    }
                },
                "required": ["command"]
            }),
        },
        // ====================================================================
        // AST-based Code Search
        // ====================================================================
        FunctionDeclaration {
            name: "ast_grep".to_string(),
            description: "Search code using AST patterns. Unlike regex, this understands code structure. Use meta-variables like $VAR to match any expression. Examples: 'fn $NAME($$$ARGS) { $$$BODY }' matches Rust functions, 'console.log($MSG)' matches JS logging calls. Pattern must include complete syntactic structures.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "AST pattern to search for. Use $VAR for single nodes, $$$VAR for multiple nodes. Must be a complete syntactic structure."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search (relative to workspace). Defaults to current directory."
                    },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "typescript", "javascript", "python", "go", "java", "c", "cpp"],
                        "description": "Language for pattern parsing. Auto-detected from file extension if not specified."
                    }
                },
                "required": ["pattern"]
            }),
        },
        FunctionDeclaration {
            name: "ast_grep_replace".to_string(),
            description: "Replace code patterns using AST-aware rewriting. Captured meta-variables from the pattern can be used in the replacement. Example: pattern='console.log($MSG)' replacement='logger.info($MSG)' transforms logging calls.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "AST pattern to match. Use $VAR for captures."
                    },
                    "replacement": {
                        "type": "string",
                        "description": "Replacement template. Use captured $VAR names from pattern."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to modify (relative to workspace)."
                    },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "typescript", "javascript", "python", "go", "java", "c", "cpp"],
                        "description": "Language for pattern parsing. Auto-detected if not specified."
                    }
                },
                "required": ["pattern", "replacement", "path"]
            }),
        },
        // ====================================================================
        // Memory Operations
        // ====================================================================
        FunctionDeclaration {
            name: "search_memories".to_string(),
            description: "Search long-term memory for relevant past findings, knowledge, and context. Uses semantic similarity to find related memories across sessions. Returns the most relevant memories ranked by similarity.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language query describing what you're looking for (e.g. 'open ports on 10.0.0.1', 'SQL injection vulnerabilities')"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["recon", "vulnerability", "credential", "configuration", "technique", "topology", "failed_approach"],
                        "description": "Optional category filter to narrow results"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of memories to return (default: 10, max: 50)"
                    }
                },
                "required": ["query"]
            }),
        },
        FunctionDeclaration {
            name: "store_memory".to_string(),
            description: "Store important information in long-term memory for future retrieval. Use for significant findings, discoveries, and knowledge that should persist across sessions. Each memory should be atomic (one finding per entry).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The information to store. Should be a clear, self-contained description of the finding or knowledge."
                    },
                    "category": {
                        "type": "string",
                        "enum": ["recon", "vulnerability", "credential", "configuration", "technique", "topology", "failed_approach"],
                        "description": "Category for organizing and filtering memories"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags for search (e.g. 'nmap,port-scan,10.0.0.1')"
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["project", "global"],
                        "description": "Storage scope. 'project' (default): tied to current project. 'global': visible across all projects. Use 'global' for general techniques, tool patterns, and reusable knowledge."
                    }
                },
                "required": ["content", "category"]
            }),
        },
        FunctionDeclaration {
            name: "list_memories".to_string(),
            description: "List recent memories, optionally filtered by category. Shows the most recent entries first. Use to review what has been stored.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "enum": ["recon", "vulnerability", "credential", "configuration", "technique", "topology", "failed_approach"],
                        "description": "Optional category filter"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of memories to return (default: 20, max: 100)"
                    }
                }
            }),
        },
        // ====================================================================
        // Vulnerability Knowledge Base
        // ====================================================================
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
        // ====================================================================
        // Security Analysis Tools
        // ====================================================================
        FunctionDeclaration {
            name: "log_operation".to_string(),
            description: "Log a penetration testing operation. Every significant action (scan, manual test, exploit attempt, recon step) should be logged for audit and reporting. The detail field accepts arbitrary JSON for operation-specific data.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target this operation relates to (optional)"
                    },
                    "op_type": {
                        "type": "string",
                        "enum": ["scan", "analysis", "manual_test", "ai_action", "recon", "exploit", "report", "general"],
                        "description": "Category of the operation"
                    },
                    "tool_name": {
                        "type": "string",
                        "description": "Name of the tool or technique used (e.g. 'nmap', 'burpsuite', 'manual_xss')"
                    },
                    "summary": {
                        "type": "string",
                        "description": "One-line description of what was done and the outcome"
                    },
                    "detail": {
                        "type": "object",
                        "description": "Arbitrary JSON with operation-specific data (command, payload, response, findings)"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["completed", "failed", "in_progress", "cancelled"],
                        "description": "Status of the operation"
                    }
                },
                "required": ["op_type", "summary"]
            }),
        },
        FunctionDeclaration {
            name: "discover_apis".to_string(),
            description: "Record discovered API endpoints for a target. Call this after crawling, proxy capture, JS analysis, or manual discovery to persist endpoint data. Endpoints are stored per-target and include method, path, parameters, and risk level.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target these endpoints belong to"
                    },
                    "endpoints": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "url": {"type": "string", "description": "Full URL of the endpoint"},
                                "method": {"type": "string", "description": "HTTP method (GET, POST, PUT, DELETE, etc.)"},
                                "path": {"type": "string", "description": "URL path component"},
                                "params": {"type": "array", "description": "Parameter names/types discovered"},
                                "auth_type": {"type": "string", "description": "Authentication type if known (bearer, basic, cookie, none)"},
                                "risk_level": {"type": "string", "enum": ["unknown", "low", "medium", "high", "critical"]}
                            },
                            "required": ["url", "method", "path"]
                        },
                        "description": "Array of discovered API endpoints"
                    },
                    "source": {
                        "type": "string",
                        "description": "How these endpoints were discovered (js_analysis, proxy, crawler, manual, ai)"
                    }
                },
                "required": ["target_id", "endpoints", "source"]
            }),
        },
        FunctionDeclaration {
            name: "save_js_analysis".to_string(),
            description: "Save JavaScript file analysis results for a target. Records discovered frameworks, libraries, API endpoints found in JS, potential secrets/tokens, and source map availability. Call after the js_analyzer sub-agent or manual JS analysis completes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target this JS file belongs to"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL where the JS file was found"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Filename of the JS file"
                    },
                    "frameworks": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "Detected frameworks: [{name, version, confidence}]"
                    },
                    "libraries": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "Detected libraries: [{name, version}]"
                    },
                    "endpoints_found": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "API endpoints found in JS: [{url, method, context}]"
                    },
                    "secrets_found": {
                        "type": "array",
                        "items": {"type": "object"},
                        "description": "Potential secrets: [{type, value_preview, line, context}]"
                    },
                    "source_maps": {
                        "type": "boolean",
                        "description": "Whether source maps are available"
                    },
                    "risk_summary": {
                        "type": "string",
                        "description": "Brief risk assessment of findings in this JS file"
                    }
                },
                "required": ["target_id", "url", "filename"]
            }),
        },
        FunctionDeclaration {
            name: "fingerprint_target".to_string(),
            description: "Record a technology fingerprint for a target. Stores detected technologies with confidence scores. Duplicates are merged (higher confidence wins). Use for web server, CMS, WAF, framework, language, and OS detection.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target"
                    },
                    "fingerprints": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "category": {"type": "string", "enum": ["technology", "framework", "cms", "waf", "cdn", "os", "server", "language"]},
                                "name": {"type": "string", "description": "Technology name (e.g. 'Apache', 'WordPress', 'React')"},
                                "version": {"type": "string", "description": "Version if detected"},
                                "confidence": {"type": "number", "description": "Detection confidence 0.0-1.0"},
                                "evidence": {"type": "array", "description": "Evidence strings supporting detection"},
                                "cpe": {"type": "string", "description": "CPE string if known"}
                            },
                            "required": ["category", "name", "confidence"]
                        },
                        "description": "Array of detected technology fingerprints"
                    },
                    "source": {
                        "type": "string",
                        "description": "Detection method (wappalyzer, header_analysis, manual, nmap, ai)"
                    }
                },
                "required": ["target_id", "fingerprints", "source"]
            }),
        },
        FunctionDeclaration {
            name: "log_scan_result".to_string(),
            description: "Log a passive scan or manual security test result against a target. Records test type (XSS, SQLi, etc.), payload, result, and evidence. Used for tracking what has been tested and documenting findings during penetration testing.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target"
                    },
                    "test_type": {
                        "type": "string",
                        "description": "Type of test: xss, sqli, cmd_injection, ssrf, idor, auth_bypass, lfi, rfi, xxe, open_redirect, cors, csrf, info_leak, etc."
                    },
                    "payload": {
                        "type": "string",
                        "description": "The payload or input used for testing"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL that was tested"
                    },
                    "parameter": {
                        "type": "string",
                        "description": "Parameter name that was tested"
                    },
                    "result": {
                        "type": "string",
                        "enum": ["vulnerable", "not_vulnerable", "potential", "error", "pending"],
                        "description": "Test result"
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Evidence supporting the result (response snippet, error message, etc.)"
                    },
                    "severity": {
                        "type": "string",
                        "enum": ["critical", "high", "medium", "low", "info"],
                        "description": "Severity if vulnerability was found"
                    },
                    "tool_used": {
                        "type": "string",
                        "description": "Tool used for testing (burp, sqlmap, manual, custom script name)"
                    },
                    "tester": {
                        "type": "string",
                        "description": "Who performed the test: manual, ai, or scanner name"
                    }
                },
                "required": ["target_id", "test_type", "result"]
            }),
        },
        FunctionDeclaration {
            name: "query_target_data".to_string(),
            description: "Query aggregated security data for a target. Returns assets, API endpoints, fingerprints, JS analysis results, and scan logs. Use this to get a comprehensive overview of what is known about a target before planning next steps.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "string",
                        "description": "UUID of the target to query"
                    },
                    "sections": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["assets", "endpoints", "fingerprints", "js_analysis", "scan_logs", "all"]
                        },
                        "description": "Which data sections to include (default: all)"
                    }
                },
                "required": ["target_id"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_function_declarations_returns_all_tools() {
        let declarations = build_function_declarations();

        // 15 original + 5 knowledge base + 6 security analysis tools
        assert_eq!(declarations.len(), 26);

        // Verify all expected tools are present
        let names: Vec<&str> = declarations.iter().map(|d| d.name.as_str()).collect();

        // File operations
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"create_file"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"delete_file"));

        // Directory operations
        assert!(names.contains(&"list_files"));
        assert!(names.contains(&"list_directory"));
        assert!(names.contains(&"grep_file"));

        // AST-based code search
        assert!(names.contains(&"ast_grep"));
        assert!(names.contains(&"ast_grep_replace"));

        // Shell
        assert!(names.contains(&"run_pty_cmd"));

        // Planning
        assert!(names.contains(&"update_plan"));

        // Memory
        assert!(names.contains(&"search_memories"));
        assert!(names.contains(&"store_memory"));
        assert!(names.contains(&"list_memories"));

        // Knowledge Base
        assert!(names.contains(&"search_knowledge_base"));
        assert!(names.contains(&"write_knowledge"));
        assert!(names.contains(&"read_knowledge"));
        assert!(names.contains(&"ingest_cve"));
        assert!(names.contains(&"save_poc"));
        assert!(names.contains(&"list_cves_with_pocs"));
        assert!(names.contains(&"list_unresearched_cves"));
        assert!(names.contains(&"poc_stats"));

        // Security Analysis
        assert!(names.contains(&"log_operation"));
        assert!(names.contains(&"discover_apis"));
        assert!(names.contains(&"save_js_analysis"));
        assert!(names.contains(&"fingerprint_target"));
        assert!(names.contains(&"log_scan_result"));
        assert!(names.contains(&"query_target_data"));
    }

    #[test]
    fn test_declarations_have_valid_schemas() {
        let declarations = build_function_declarations();

        for decl in declarations {
            // Each declaration should have a non-empty name
            assert!(!decl.name.is_empty(), "Declaration should have a name");

            // Each declaration should have a non-empty description
            assert!(
                !decl.description.is_empty(),
                "Declaration should have a description"
            );

            // Parameters should be an object type
            assert_eq!(
                decl.parameters.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "Parameters should be an object type for {}",
                decl.name
            );

            // Parameters should have a properties field
            assert!(
                decl.parameters.get("properties").is_some(),
                "Parameters should have properties for {}",
                decl.name
            );
        }
    }

    #[test]
    fn test_read_file_declaration() {
        let declarations = build_function_declarations();
        let read_file = declarations
            .iter()
            .find(|d| d.name == "read_file")
            .expect("read_file should exist");

        // Should have path as required
        let required = read_file.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));

        // Should have line_start and line_end as optional
        let props = read_file.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("line_start"));
        assert!(props.contains_key("line_end"));
    }

    #[test]
    fn test_edit_file_declaration() {
        let declarations = build_function_declarations();
        let edit_file = declarations
            .iter()
            .find(|d| d.name == "edit_file")
            .expect("edit_file should exist");

        // Should have path, old_text, new_text as required
        let required = edit_file.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("old_text")));
        assert!(required.contains(&json!("new_text")));
    }

    #[test]
    fn test_run_pty_cmd_declaration() {
        let declarations = build_function_declarations();
        let run_pty_cmd = declarations
            .iter()
            .find(|d| d.name == "run_pty_cmd")
            .expect("run_pty_cmd should exist");

        // Should have command as required
        let required = run_pty_cmd.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("command")));

        // Should have cwd and timeout as optional
        let props = run_pty_cmd.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("command"));
        assert!(props.contains_key("cwd"));
        assert!(props.contains_key("timeout"));
    }

    #[test]
    fn test_function_declaration_serialization() {
        let decl = FunctionDeclaration {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "arg": {"type": "string"}
                },
                "required": ["arg"]
            }),
        };

        // Should serialize to JSON
        let json_str = serde_json::to_string(&decl).unwrap();
        assert!(json_str.contains("test_tool"));

        // Should deserialize back
        let parsed: FunctionDeclaration = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.name, "test_tool");
    }
}
