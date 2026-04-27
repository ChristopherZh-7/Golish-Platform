use serde_json::json;
use super::FunctionDeclaration;

pub fn sploitus_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "search_exploits".to_string(),
            description: "Search the Sploitus vulnerability database for exploits, tools, and CVEs. Returns structured results with source URLs, CVE references, and descriptions.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query for exploits or CVEs, e.g. 'apache 2.4.49' or 'CVE-2021-41773'"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["exploits", "tools", "cve"],
                        "description": "Sploitus index to query. 'exploits' (default) covers Metasploit / ExploitDB / PacketStorm entries; 'tools' covers offensive tooling write-ups; 'cve' returns CVE-tagged entries."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum normalized entries to return (default: 10, capped at 100)."
                    }
                },
                "required": ["query"]
            }),
        },
    ]
}
