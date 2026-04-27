use serde_json::json;
use super::FunctionDeclaration;

pub fn graph_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "graph_add_entity".to_string(),
            description: "Add or update an entity in the knowledge graph. Entities are automatically deduplicated by type+name. Use for hosts, services, vulnerabilities, credentials found during testing.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entity_type": {
                        "type": "string",
                        "enum": ["host", "service", "vulnerability", "credential", "technique", "endpoint"],
                        "description": "Kind of entity to insert. Hosts are reachable network nodes; services are exposed listeners (e.g. http/443); vulnerabilities reference CVE or finding identifiers; credentials hold captured secrets; techniques map to MITRE ATT&CK or playbook steps; endpoints are application URLs/routes."
                    },
                    "name": {
                        "type": "string",
                        "description": "Stable, human-readable identifier (e.g. '10.0.0.5', 'http/8080', 'CVE-2024-1234'). Used together with entity_type for deduplication."
                    },
                    "properties": {
                        "type": "object",
                        "description": "Optional free-form JSON metadata (banner, CVSS score, evidence, …). Merged into existing properties on conflict."
                    }
                },
                "required": ["entity_type", "name"]
            }),
        },
        FunctionDeclaration {
            name: "graph_add_relation".to_string(),
            description: "Add a relationship between two entities in the knowledge graph. Automatically resolves entity names to IDs.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from_entity": {
                        "type": "string",
                        "description": "Source entity. Either an existing entity name (resolved by best-match) or a UUID returned from a previous graph_add_entity call."
                    },
                    "to_entity": {
                        "type": "string",
                        "description": "Destination entity. Same resolution rules as from_entity."
                    },
                    "relation_type": {
                        "type": "string",
                        "enum": ["runs_service", "has_vulnerability", "exploited_by", "lateral_move", "authenticates_to", "exposes_endpoint"],
                        "description": "Directed edge kind. runs_service: host→service. has_vulnerability: target→vuln. exploited_by: vuln→technique/credential. lateral_move: host→host. authenticates_to: credential→target. exposes_endpoint: service→endpoint."
                    },
                    "properties": {
                        "type": "object",
                        "description": "Optional edge metadata (port, exploit reference, evidence)."
                    }
                },
                "required": ["from_entity", "to_entity", "relation_type"]
            }),
        },
        FunctionDeclaration {
            name: "graph_search".to_string(),
            description: "Search for entities in the knowledge graph by name. Returns matching entities with their properties.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Substring to match against entity names (case-insensitive)."
                    },
                    "entity_type": {
                        "type": "string",
                        "description": "Optional filter restricting results to a single entity_type (host/service/vulnerability/credential/technique/endpoint)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entities to return (default: 10)."
                    }
                },
                "required": ["query"]
            }),
        },
        FunctionDeclaration {
            name: "graph_neighbors".to_string(),
            description: "Get all entities directly connected to a given entity. Optionally filter by relation type.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entity": {
                        "type": "string",
                        "description": "Entity name or UUID whose outgoing neighbors should be returned."
                    },
                    "relation_type": {
                        "type": "string",
                        "description": "Optional filter restricting results to a single relation kind (runs_service, has_vulnerability, exploited_by, lateral_move, authenticates_to, exposes_endpoint)."
                    }
                },
                "required": ["entity"]
            }),
        },
        FunctionDeclaration {
            name: "graph_attack_paths".to_string(),
            description: "Find all attack paths starting from a given entity. Uses graph traversal to discover exploitation chains (host->service->vulnerability->credential->lateral_move).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from_entity": {
                        "type": "string",
                        "description": "Entity name or UUID at which to start the traversal (typically the initial foothold)."
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum walk depth in graph edges (default: 5). Cycles are pruned automatically."
                    }
                },
                "required": ["from_entity"]
            }),
        },
    ]
}
