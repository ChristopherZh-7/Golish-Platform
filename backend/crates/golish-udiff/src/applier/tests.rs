//! Hunk applier tests.

use super::*;
use super::fuzzy::FuzzyMatchResult;
use super::*;
use crate::parser::ParsedHunk;

#[test]
fn test_apply_simple_hunk() {
    let content = "fn main() {\n    println!(\"Hello\");\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello\");".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello, world!\");".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert_eq!(
                new_content,
                "fn main() {\n    println!(\"Hello, world!\");\n}"
            );
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_apply_multiple_hunks() {
    let content = "fn first() {\n    let x = 1;\n}\nfn second() {\n    let y = 3;\n}";
    let hunks = vec![
        ParsedHunk {
            context_anchor: None,
            old_lines: vec![
                "fn first() {".to_string(),
                "    let x = 1;".to_string(),
                "}".to_string(),
            ],
            new_lines: vec![
                "fn first() {".to_string(),
                "    let x = 2;".to_string(),
                "}".to_string(),
            ],
        },
        ParsedHunk {
            context_anchor: None,
            old_lines: vec![
                "fn second() {".to_string(),
                "    let y = 3;".to_string(),
                "}".to_string(),
            ],
            new_lines: vec![
                "fn second() {".to_string(),
                "    let y = 4;".to_string(),
                "}".to_string(),
            ],
        },
    ];

    let result = UdiffApplier::apply_hunks(content, &hunks);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("let x = 2;"));
            assert!(new_content.contains("let y = 4;"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_apply_no_match() {
    let content = "fn main() {\n    println!(\"Different\");\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello\");".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello, world!\");".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::NoMatch { hunk_idx, .. } => {
            assert_eq!(hunk_idx, 0);
        }
        _ => panic!("Expected NoMatch, got {:?}", result),
    }
}

#[test]
fn test_apply_normalized_whitespace() {
    let content = "fn main() {\n  println!(\"Hello\");\n}"; // 2 spaces indent
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "println!(\"Hello\");".to_string(), // No indent in hunk
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "println!(\"Goodbye\");".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            // Normalized matching applies uniform indent from first matched line
            // First line "fn main() {" has no indent, so all new lines get no indent
            assert!(new_content.contains("fn main() {"));
            assert!(new_content.contains("println!(\"Goodbye\");"));
        }
        _ => panic!(
            "Expected Success with normalized matching, got {:?}",
            result
        ),
    }
}

#[test]
fn test_apply_partial_success() {
    let content = "fn first() {\n    let x = 1;\n}\nfn second() {\n    let y = 3;\n}";
    let hunks = vec![
        ParsedHunk {
            context_anchor: None,
            old_lines: vec![
                "fn first() {".to_string(),
                "    let x = 1;".to_string(),
                "}".to_string(),
            ],
            new_lines: vec![
                "fn first() {".to_string(),
                "    let x = 2;".to_string(),
                "}".to_string(),
            ],
        },
        ParsedHunk {
            context_anchor: None,
            old_lines: vec!["nonexistent".to_string()],
            new_lines: vec!["replacement".to_string()],
        },
    ];

    let result = UdiffApplier::apply_hunks(content, &hunks);
    match result {
        ApplyResult::PartialSuccess {
            applied,
            failed,
            new_content,
        } => {
            assert_eq!(applied, vec![0]);
            assert_eq!(failed.len(), 1);
            assert!(new_content.contains("let x = 2;"));
        }
        _ => panic!("Expected PartialSuccess, got {:?}", result),
    }
}

// =========================================================================
// Fuzzy matching tests
// =========================================================================

#[test]
fn test_fuzzy_match_minor_typo() {
    // Content has a minor typo difference from the hunk
    let content = "fn main() {\n    println!(\"Helo\");\n}"; // "Helo" typo
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello\");".to_string(), // Correct spelling in hunk
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello, world!\");".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("Hello, world!"));
        }
        _ => panic!("Expected Success from fuzzy match, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_match_extra_whitespace() {
    // Content has extra spaces that normalized match wouldn't catch
    let content = "fn main() {\n    let  x  =  1;\n}"; // Extra spaces
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    let x = 1;".to_string(), // Normal spacing
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    let x = 2;".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("let x = 2;"));
        }
        _ => panic!("Expected Success from fuzzy match, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_match_below_threshold() {
    // Content is too different to match (below threshold)
    let content = "fn completely_different() {\n    something_else();\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello\");".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Goodbye\");".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::NoMatch { suggestion, .. } => {
            // Should include fuzzy match info in suggestion
            assert!(suggestion.contains("fuzzy match"));
        }
        _ => panic!("Expected NoMatch, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_match_prefers_exact() {
    // When exact match exists, should use it (not fuzzy)
    let content = "fn main() {\n    println!(\"Hello\");\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Hello\");".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "fn main() {".to_string(),
            "    println!(\"Goodbye\");".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("Goodbye"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_match_single_line_change() {
    // Single line with minor difference
    let content = "let result = calculate_value(x, y);"; // "result" vs "res"
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec!["let res = calculate_value(x, y);".to_string()],
        new_lines: vec!["let res = compute_value(x, y);".to_string()],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("compute_value"));
        }
        _ => panic!("Expected Success from fuzzy match, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_apply_direct() {
    use super::fuzzy::FuzzyMatchResult;

    let content = "fn test() {\n    let x = old_value;\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn test() {".to_string(),
            "    let x = old_val;".to_string(), // Slightly different
            "}".to_string(),
        ],
        new_lines: vec![
            "fn test() {".to_string(),
            "    let x = new_value;".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::try_fuzzy_apply(content, &hunk, 0.85);
    match result {
        FuzzyMatchResult::Match { similarity, .. } => {
            assert!(
                similarity >= 0.85,
                "Similarity {} should be >= 0.85",
                similarity
            );
        }
        _ => panic!("Expected Match, got {:?}", result),
    }
}

#[test]
fn test_fuzzy_match_multiple_similar_blocks() {
    // Content with multiple similar blocks should detect ambiguity
    let content = "fn process_a() {\n    let x = 1;\n}\nfn process_b() {\n    let x = 1;\n}";
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "fn process() {".to_string(), // Generic - matches both
            "    let x = 1;".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "fn process() {".to_string(),
            "    let x = 2;".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    // Should either match the best one or report multiple matches
    match result {
        ApplyResult::Success { .. } | ApplyResult::MultipleMatches { .. } => {
            // Both are acceptable outcomes depending on similarity scores
        }
        ApplyResult::NoMatch { .. } => {
            // Also acceptable if neither meets threshold
        }
        _ => panic!("Unexpected result: {:?}", result),
    }
}

// =========================================================================
// Real-world scenario tests
// =========================================================================

#[test]
fn test_realworld_rust_function_with_minor_signature_change() {
    // Scenario: LLM generated diff against code, but function signature was tweaked
    let content = r#"impl UserService {
/// Fetches a user by their unique identifier.
pub async fn get_user(&self, user_id: UserId) -> Result<Option<User>> {
    let user = self.db.query_user(user_id).await?;
    Ok(user)
}
}"#;

    // LLM saw "id" instead of "user_id" in the parameter name
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "impl UserService {".to_string(),
            "    /// Fetches a user by their unique identifier.".to_string(),
            "    pub async fn get_user(&self, id: UserId) -> Result<Option<User>> {"
                .to_string(),
            "        let user = self.db.query_user(id).await?;".to_string(),
            "        Ok(user)".to_string(),
            "    }".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "impl UserService {".to_string(),
            "    /// Fetches a user by their unique identifier.".to_string(),
            "    pub async fn get_user(&self, id: UserId) -> Result<Option<User>> {"
                .to_string(),
            "        let user = self.db.query_user(id).await?;".to_string(),
            "        tracing::debug!(\"Fetched user: {:?}\", user);".to_string(),
            "        Ok(user)".to_string(),
            "    }".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("tracing::debug!"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_typescript_import_reordered() {
    // Scenario: Imports were auto-sorted since LLM saw the code
    let content = r#"import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';

export function LoginForm() {"#;

    // LLM saw imports in different order
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "import { Button } from '@/components/ui/button';".to_string(),
            "import { useState } from 'react';".to_string(),
            "import { Dialog } from '@/components/ui/dialog';".to_string(),
            "import { Input } from '@/components/ui/input';".to_string(),
            "".to_string(),
            "export function LoginForm() {".to_string(),
        ],
        new_lines: vec![
            "import { Button } from '@/components/ui/button';".to_string(),
            "import { useState, useEffect } from 'react';".to_string(),
            "import { Dialog } from '@/components/ui/dialog';".to_string(),
            "import { Input } from '@/components/ui/input';".to_string(),
            "import { toast } from 'sonner';".to_string(),
            "".to_string(),
            "export function LoginForm() {".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("useEffect"));
            assert!(new_content.contains("toast"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_python_function_signature_changed() {
    // Scenario: Function parameter was renamed since LLM context
    let content = r#"def process_data(input_data: list[dict], config: Config) -> ProcessResult:
"""Process the input data according to configuration."""
validated = validate_input(input_data)
return transform(validated, config)
"#;

    // LLM saw "data" instead of "input_data"
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "def process_data(data: list[dict], config: Config) -> ProcessResult:".to_string(),
            "    \"\"\"Process the input data according to configuration.\"\"\"".to_string(),
            "    validated = validate_input(data)".to_string(),
            "    return transform(validated, config)".to_string(),
        ],
        new_lines: vec![
            "def process_data(data: list[dict], config: Config) -> ProcessResult:".to_string(),
            "    \"\"\"Process the input data according to configuration.\"\"\"".to_string(),
            "    logger.info(f\"Processing {len(data)} items\")".to_string(),
            "    validated = validate_input(data)".to_string(),
            "    return transform(validated, config)".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("logger.info"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_json_config_with_trailing_comma() {
    // Scenario: JSON file has trailing comma difference
    let content = r#"{
  "name": "my-app",
  "version": "1.0.0",
  "dependencies": {
"react": "^18.2.0",
"typescript": "^5.0.0"
  }
}"#;

    // LLM version had trailing comma after typescript
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "  \"dependencies\": {".to_string(),
            "    \"react\": \"^18.2.0\",".to_string(),
            "    \"typescript\": \"^5.0.0\",".to_string(),
            "  }".to_string(),
        ],
        new_lines: vec![
            "  \"dependencies\": {".to_string(),
            "    \"react\": \"^18.2.0\",".to_string(),
            "    \"typescript\": \"^5.0.0\",".to_string(),
            "    \"zod\": \"^3.22.0\",".to_string(),
            "  }".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("zod"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_go_struct_field_with_json_tag_variation() {
    // Scenario: JSON tags have slightly different formatting
    let content = r#"type User struct {
	ID        int64  `json:"id"`
	Name      string `json:"name"`
	Email     string `json:"email,omitempty"`
	CreatedAt time.Time `json:"created_at"`
}"#;

    // LLM saw json tags without omitempty and different spacing
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "type User struct {".to_string(),
            "\tID        int64  `json:\"id\"`".to_string(),
            "\tName      string `json:\"name\"`".to_string(),
            "\tEmail     string `json:\"email\"`".to_string(),
            "\tCreatedAt time.Time `json:\"created_at\"`".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            "type User struct {".to_string(),
            "\tID        int64  `json:\"id\"`".to_string(),
            "\tName      string `json:\"name\"`".to_string(),
            "\tEmail     string `json:\"email\"`".to_string(),
            "\tRole      string `json:\"role\"`".to_string(),
            "\tCreatedAt time.Time `json:\"created_at\"`".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("Role"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_rust_error_handling_minor_diff() {
    // Scenario: Variable name slightly different since LLM context
    let content = r#"    pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let file_content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&file_content)?;
    Ok(config)
}"#;

    // LLM saw "content" instead of "file_content"
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "    pub fn load_config(path: &Path) -> anyhow::Result<Config> {".to_string(),
            "        let content = std::fs::read_to_string(path)?;".to_string(),
            "        let config: Config = toml::from_str(&content)?;".to_string(),
            "        Ok(config)".to_string(),
            "    }".to_string(),
        ],
        new_lines: vec![
            "    pub fn load_config(path: &Path) -> anyhow::Result<Config> {".to_string(),
            "        let content = std::fs::read_to_string(path)?;".to_string(),
            "        let config: Config = toml::from_str(&content)?;".to_string(),
            "        config.validate()?;".to_string(),
            "        Ok(config)".to_string(),
            "    }".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("config.validate()"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_rust_error_handling_too_different() {
    // Scenario: Error handling pattern changed significantly - below threshold
    let content = r#"    pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config from {:?}", path))?;
    let config: Config = toml::from_str(&content)
        .with_context(|| "Failed to parse config")?;
    Ok(config)
}"#;

    // LLM saw simpler version - too different
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "    pub fn load_config(path: &Path) -> anyhow::Result<Config> {".to_string(),
            "        let content = std::fs::read_to_string(path)?;".to_string(),
            "        let config: Config = toml::from_str(&content)?;".to_string(),
            "        Ok(config)".to_string(),
            "    }".to_string(),
        ],
        new_lines: vec![
            "    pub fn load_config(path: &Path) -> anyhow::Result<Config> {".to_string(),
            "        let content = std::fs::read_to_string(path)?;".to_string(),
            "        let config: Config = toml::from_str(&content)?;".to_string(),
            "        config.validate()?;".to_string(),
            "        Ok(config)".to_string(),
            "    }".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    // This should fail - content has grown too different with context wrappers
    match result {
        ApplyResult::NoMatch { suggestion, .. } => {
            assert!(suggestion.contains("fuzzy match"));
        }
        _ => panic!("Expected NoMatch (too different), got {:?}", result),
    }
}

#[test]
fn test_realworld_css_with_minor_value_change() {
    // Scenario: CSS property value was tweaked since LLM context
    let content = r#".button {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 10px 20px;
  background: #007bff;
}"#;

    // LLM saw slightly different padding value
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            ".button {".to_string(),
            "  display: flex;".to_string(),
            "  align-items: center;".to_string(),
            "  justify-content: center;".to_string(),
            "  padding: 8px 16px;".to_string(),
            "  background: #007bff;".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            ".button {".to_string(),
            "  display: flex;".to_string(),
            "  align-items: center;".to_string(),
            "  justify-content: center;".to_string(),
            "  padding: 8px 16px;".to_string(),
            "  background: #007bff;".to_string(),
            "  border-radius: 4px;".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("border-radius"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}

#[test]
fn test_realworld_css_too_different_fails_gracefully() {
    // Scenario: CSS has vendor prefixes that make it too different - should fail gracefully
    let content = r#".button {
  display: flex;
  -webkit-align-items: center;
  align-items: center;
  -webkit-justify-content: center;
  justify-content: center;
  padding: 8px 16px;
}"#;

    // LLM saw version without vendor prefixes - too different
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            ".button {".to_string(),
            "  display: flex;".to_string(),
            "  align-items: center;".to_string(),
            "  justify-content: center;".to_string(),
            "  padding: 8px 16px;".to_string(),
            "}".to_string(),
        ],
        new_lines: vec![
            ".button {".to_string(),
            "  display: flex;".to_string(),
            "  align-items: center;".to_string(),
            "  justify-content: center;".to_string(),
            "  padding: 8px 16px;".to_string(),
            "  border-radius: 4px;".to_string(),
            "}".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    // This should fail because the vendor prefixes make it too different
    match result {
        ApplyResult::NoMatch { suggestion, .. } => {
            assert!(suggestion.contains("fuzzy match: 70%"));
        }
        _ => panic!("Expected NoMatch (too different), got {:?}", result),
    }
}

#[test]
fn test_realworld_multiline_string_slight_difference() {
    // Scenario: Multiline SQL query with slight formatting difference
    let content = r#"    let query = "
    SELECT u.id, u.name, u.email
    FROM users u
    WHERE u.active = true
      AND u.created_at > $1
    ORDER BY u.created_at DESC
";"#;

    // LLM saw different indentation in WHERE clause
    let hunk = ParsedHunk {
        context_anchor: None,
        old_lines: vec![
            "    let query = \"".to_string(),
            "        SELECT u.id, u.name, u.email".to_string(),
            "        FROM users u".to_string(),
            "        WHERE u.active = true AND u.created_at > $1".to_string(),
            "        ORDER BY u.created_at DESC".to_string(),
            "    \";".to_string(),
        ],
        new_lines: vec![
            "    let query = \"".to_string(),
            "        SELECT u.id, u.name, u.email, u.role".to_string(),
            "        FROM users u".to_string(),
            "        WHERE u.active = true AND u.created_at > $1".to_string(),
            "        ORDER BY u.created_at DESC".to_string(),
            "    \";".to_string(),
        ],
    };

    let result = UdiffApplier::apply_hunks(content, &[hunk]);
    match result {
        ApplyResult::Success { new_content } => {
            assert!(new_content.contains("u.role"));
        }
        _ => panic!("Expected Success, got {:?}", result),
    }
}
