use super::FunctionDeclaration;
use serde_json::{json, Value};

fn terminal_exceptions_schema() -> Value {
    json!({
        "type": "array",
        "description": "Preview-only terminal coverage for Target Intel / External Attack Surface cells that DB truth cannot derive. Use checked_empty only with exact-technique evidence; blocked/not_applicable require a concrete note. The preview never persists or authorizes assets, and the same returned coverage_to_submit must be passed to submit_stage_deliverable. Enumeration remains DB/evidence authoritative and rejects any non-empty array.",
        "items": {
            "type": "object",
            "properties": {
                "asset": {"type": "string", "description": "Exact asset value from the current authoritative worklist."},
                "technique": {"type": "string", "description": "Exact technique id from the pending worklist cell."},
                "status": {"type": "string", "enum": ["checked_empty", "blocked", "not_applicable"]},
                "evidence_refs": {"type": "array", "items": {"type": "integer"}},
                "note": {"type": "string"},
                "reason_kind": {"type": "string", "enum": ["provider_missing", "credential_missing", "rate_limited", "tool_missing", "out_of_scope", "not_applicable"]},
                "tested_units": {"type": "integer"},
                "total_units": {"type": "integer"},
                "sampling_rationale": {"type": "string"}
            },
            "required": ["asset", "technique", "status"],
            "additionalProperties": false
        }
    })
}

pub fn security_analysis_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
            description: "Save JavaScript file analysis results for a target. Records discovered frameworks, libraries, API endpoints found in JS, potential secrets/tokens, and source map availability. Call after JS security analysis completes.".to_string(),
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
        FunctionDeclaration {
            name: "list_in_scope_targets".to_string(),
            description: "List the in-scope targets/assets collected by reconnaissance (organization recon, manual target-add). Returns each target's target_id (UUID), value (domain/IP/URL/CIDR), type, plus intel context (source, status, real_ip, ports, http_status, cdn_waf, organization_id). Call this FIRST to discover which targets exist, then use query_target_data(target_id) to drill into any one. For a ranked attack-surface worklist, prefer list_attack_surface_seeds. Takes no arguments.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "list_attack_surface_seeds".to_string(),
            description: "List in-scope assets as a RANKED attack-surface worklist for active mapping (external_attack_surface). Each seed carries target_id, value, type, source, status, real_ip, ports, http_status, cdn_waf, organization_id and a computed `priority` (resolved/alive web hosts first, whole CIDR netblocks last). Use this to prioritise probing instead of flat-scanning a large set; optional `limit` caps the returned set to the top-N by priority.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Optional cap: return only the top-N seeds by priority. Omit for all."
                    }
                },
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "list_enumeration_web_roots".to_string(),
            description: "List the enumeration worklist: only EAS-confirmed live web roots for the active organization, with current JSAPI/DIR/PARAM coverage state and suggested next tools. Call this FIRST in enumeration instead of list_in_scope_targets; it is read-only and does not mutate target status.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "organization_id": {
                        "type": "string",
                        "description": "Organization UUID to inspect. Omit to use the active per-org/root organization."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Optional cap: return only the first N live web roots. Default 25, max 50."
                    },
                    "include_coverage": {
                        "type": "boolean",
                        "description": "Include the full coverage cells for each web root. Default true."
                    }
                },
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "enum_preflight_web_origins".to_string(),
            description: "Run the trusted, bounded, read-only transport preflight for current Enumeration roots. It first atomically refreshes all four cells to non-terminal partial markers so stale blocked state cannot survive recovery. Any HTTP response means reachable and writes no terminal coverage. Only all-strategy transport/TLS failure produces target-bound evidence and atomically closes JS/DIR/PARAM/JSAPI as blocked.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "origins": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "target_id": {"type": "string"},
                                "target_url": {"type": "string"}
                            },
                            "required": ["target_id", "target_url"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["origins"],
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "check_stage_asset_coverage".to_string(),
            description: "Read the current stage asset-coverage matrix from database truth before submitting. It returns ready_to_submit=false when any asset×technique cell is still pending/error/partial after valid preview-only terminal exceptions are applied. Target Intel / EAS may preview exact checked_empty/blocked/not_applicable cells; Enumeration terminal truth remains backend-owned. Defaults to the active harness organization, current session, and current stage when available.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "stage": {
                        "type": "string",
                        "enum": ["target_intel", "external_attack_surface", "enumeration"],
                        "description": "Stage to inspect. Omit to use the active harness stage."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": "Organization UUID to inspect. Omit to use the active per-org/root organization."
                    },
                    "max_gaps": {
                        "type": "integer",
                        "description": "Maximum pending/error cells to return as examples. Defaults to 25."
                    },
                    "include_assets": {
                        "type": "boolean",
                        "description": "Set true only when you need the full asset matrix. Default false returns a compact preflight summary."
                    },
                    "terminal_exceptions": terminal_exceptions_schema()
                },
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "stage_worklist_status".to_string(),
            description: "Read a compact DB-truth status view for the current stage-local worklist, optionally previewing exact Target Intel / EAS terminal exceptions. Enumeration terminal state comes only from current-run producer or trusted preflight evidence.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "stage": {
                        "type": "string",
                        "enum": ["scoping", "target_intel", "external_attack_surface", "enumeration", "vuln_triage", "verification", "access_validation", "internal_discovery", "objective_pathing", "objective_simulation", "reporting", "cleanup"],
                        "description": "Stage to inspect. Omit to use the active harness stage."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": "Organization UUID to inspect. Omit to use the active per-org/root organization."
                    },
                    "terminal_exceptions": terminal_exceptions_schema()
                },
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "stage_worklist_next".to_string(),
            description: "Return the next batch of unfinished DB-truth work items for the active stage after applying preview-only Target Intel / EAS terminal exceptions. Each item is one asset×technique cell with state, suggested tools, evidence refs, and stage-specific focus. Enumeration returns at most 200 cells across at most 50 distinct exact-origin roots and rejects non-empty terminal exceptions.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "stage": {
                        "type": "string",
                        "enum": ["scoping", "target_intel", "external_attack_surface", "enumeration", "vuln_triage", "verification", "access_validation", "internal_discovery", "objective_pathing", "objective_simulation", "reporting", "cleanup"],
                        "description": "Stage to inspect. Omit to use the active harness stage."
                    },
                    "organization_id": {
                        "type": "string",
                        "description": "Organization UUID to inspect. Omit to use the active per-org/root organization."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum work-item cells to return. Defaults to 25, max 200. Enumeration additionally caps one response at 50 distinct exact-origin roots."
                    },
                    "prefer": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["pending", "error", "partial", "blocked", "next_wave_pending"]
                        },
                        "description": "Cell states to include. Defaults to pending+error+partial."
                    },
                    "terminal_exceptions": terminal_exceptions_schema()
                },
                "additionalProperties": false
            }),
        },
        FunctionDeclaration {
            name: "list_recent_evidence".to_string(),
            description: "List this run's recent REAL evidence-ledger ids with the context needed to cite them — each row has evidence_id plus (when known) tool, subject, technique, asset, outcome, kind, age_seconds. Call this BEFORE submit_stage_deliverable to discover the real ids your tool runs produced, then put the ids whose output backs each claim into that claim's evidence_ids and the top-level evidence_refs. This is the reliable id source: do NOT invent ids, copy placeholders (1,2,3), or use submit_stage_deliverable as a way to discover missing ids. Read-only; scoped to the current chat session.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum evidence rows to return, newest first. Defaults to 25, max 200."
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}
