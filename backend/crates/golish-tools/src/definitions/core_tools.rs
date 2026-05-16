use super::FunctionDeclaration;
use serde_json::json;

pub fn plan_declarations() -> Vec<FunctionDeclaration> {
    vec![
        FunctionDeclaration {
            name: "update_plan".to_string(),
            description: "Create or update the task plan. Use this to track progress on multi-step tasks. Each step should have a description and status (pending, in_progress, or completed). Only one step can be in_progress at a time. Prefer `update_plan_patch` for small refinements once a plan already exists.".to_string(),
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
        FunctionDeclaration {
            name: "update_plan_patch".to_string(),
            description: "Incrementally refine an existing task plan via a sequence of patch operations. Prefer this over `update_plan` once a plan exists, since it avoids rewriting completed steps and keeps stable step ids. Supports four op types: `add` (insert a new step after a given id), `remove` (delete a step by id), `modify` (change title / status / failure_kind of a step by id), and `reorder` (move a step after another id). At most one step may be in_progress at a time and the total step count may not exceed 12.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "explanation": {
                        "type": "string",
                        "description": "Optional one-sentence rationale for this refinement (purely informational; does not overwrite the plan-level explanation)."
                    },
                    "ops": {
                        "type": "array",
                        "description": "Ordered list of patch operations to apply on top of the current plan (1-12 ops).",
                        "items": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "op": { "type": "string", "enum": ["add"] },
                                        "after_id": {
                                            "type": ["string", "null"],
                                            "description": "Insert after this step id. Null means insert at the head; unknown id appends to the end."
                                        },
                                        "title": { "type": "string", "description": "Description of the new step." },
                                        "status": {
                                            "type": "string",
                                            "enum": ["pending", "in_progress", "completed"],
                                            "description": "Initial status (defaults to pending)."
                                        }
                                    },
                                    "required": ["op", "title"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "op": { "type": "string", "enum": ["remove"] },
                                        "id": { "type": "string", "description": "Step id to remove (no-op when missing)." }
                                    },
                                    "required": ["op", "id"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "op": { "type": "string", "enum": ["modify"] },
                                        "id": { "type": "string", "description": "Step id to modify." },
                                        "title": { "type": "string", "description": "New title (omit to keep current)." },
                                        "status": {
                                            "type": "string",
                                            "enum": ["pending", "in_progress", "completed", "cancelled", "failed"],
                                            "description": "New status (omit to keep current)."
                                        },
                                        "failure_kind": {
                                            "type": "string",
                                            "enum": ["technical", "environmental", "conceptual", "external"],
                                            "description": "When status is `failed`, categorise why so the refiner can pivot strategy on repeated similar failures."
                                        }
                                    },
                                    "required": ["op", "id"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "op": { "type": "string", "enum": ["reorder"] },
                                        "id": { "type": "string", "description": "Step id to move." },
                                        "after_id": {
                                            "type": ["string", "null"],
                                            "description": "Move after this step id. Null means move to the head."
                                        }
                                    },
                                    "required": ["op", "id"]
                                }
                            ]
                        }
                    }
                },
                "required": ["ops"]
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
