use serde_json::json;
use super::FunctionDeclaration;

pub fn plan_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}

pub fn shell_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}

pub fn ast_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}
