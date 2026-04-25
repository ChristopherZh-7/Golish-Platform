use serde_json::json;
use super::FunctionDeclaration;

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
    ]
}
