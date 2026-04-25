use serde_json::json;
use super::FunctionDeclaration;

pub fn memory_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}

pub fn code_store_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "search_code".to_string(),
            description: "Search the code sample store for previously saved code snippets, exploits, and scripts. Filters by programming language when specified. Use this before writing code to check if similar code already exists.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query describing the code you're looking for (e.g. 'python reverse shell', 'nmap XML parser')"
                    },
                    "language": {
                        "type": "string",
                        "description": "Optional: filter by programming language (e.g. 'python', 'bash', 'rust', 'javascript')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 5)"
                    }
                },
                "required": ["query"]
            }),
        },
        FunctionDeclaration {
            name: "save_code".to_string(),
            description: "Save a useful code snippet, exploit, or script to the code store for future reuse. Include the language and a brief description. Only save code that worked and would be useful again.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The code content to save"
                    },
                    "language": {
                        "type": "string",
                        "description": "Programming language (e.g. 'python', 'bash', 'rust')"
                    },
                    "description": {
                        "type": "string",
                        "description": "Brief description of what the code does"
                    }
                },
                "required": ["content", "language"]
            }),
        },
    ]
}

pub fn guide_store_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "search_guide".to_string(),
            description: "Search the guide store for previously saved procedures, how-tos, and operational playbooks. Use this before starting a new procedure to check if a guide already exists.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query describing the guide you're looking for (e.g. 'how to exploit SQL injection', 'nmap service detection guide')"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["procedure", "playbook", "checklist", "reference"],
                        "description": "Optional: filter by guide type"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum results to return (default: 5)"
                    }
                },
                "required": ["query"]
            }),
        },
        FunctionDeclaration {
            name: "save_guide".to_string(),
            description: "Save a useful procedure, how-to guide, or operational playbook for future reference. Only save guides that contain actionable steps that worked.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The guide content to save"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["procedure", "playbook", "checklist", "reference"],
                        "description": "Type of guide (default: procedure)"
                    }
                },
                "required": ["content"]
            }),
        },
    ]
}
