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

mod realworld_fuzzy;
