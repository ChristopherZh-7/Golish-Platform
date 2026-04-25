use serde_json::json;
use super::FunctionDeclaration;

pub fn file_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}

pub fn directory_declarations() -> Vec<FunctionDeclaration> {
    vec![
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
    ]
}
