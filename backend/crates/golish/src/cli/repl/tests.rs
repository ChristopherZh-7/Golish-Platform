//! Tests for the REPL.
//!
//! `parse_tests` cover the user-input → `ReplCommand` parser; `skill_body_tests`
//! cover the SKILL.md frontmatter stripper exposed by the [`super::discovery`]
//! module.

use super::discovery::parse_skill_body;
use super::*;

// ────────────────────────────────────────────────────────────────────────────
// ReplCommand::parse
// ────────────────────────────────────────────────────────────────────────────

mod parse_tests {
    use super::*;

    #[test]
    fn parses_quit_command() {
        assert_eq!(ReplCommand::parse("/quit"), ReplCommand::Quit);
    }

    #[test]
    fn parses_exit_command() {
        assert_eq!(ReplCommand::parse("/exit"), ReplCommand::Quit);
    }

    #[test]
    fn parses_q_command() {
        assert_eq!(ReplCommand::parse("/q"), ReplCommand::Quit);
    }

    #[test]
    fn parses_quit_case_insensitive() {
        assert_eq!(ReplCommand::parse("/QUIT"), ReplCommand::Quit);
        assert_eq!(ReplCommand::parse("/Quit"), ReplCommand::Quit);
        assert_eq!(ReplCommand::parse("/EXIT"), ReplCommand::Quit);
        assert_eq!(ReplCommand::parse("/Q"), ReplCommand::Quit);
    }

    #[test]
    fn parses_slash_command_without_args() {
        assert_eq!(
            ReplCommand::parse("/my-prompt"),
            ReplCommand::SlashCommand {
                name: "my-prompt".to_string(),
                args: None
            }
        );
    }

    #[test]
    fn parses_slash_command_with_args() {
        assert_eq!(
            ReplCommand::parse("/my-prompt some arguments here"),
            ReplCommand::SlashCommand {
                name: "my-prompt".to_string(),
                args: Some("some arguments here".to_string())
            }
        );
    }

    #[test]
    fn parses_slash_command_with_multiword_args() {
        assert_eq!(
            ReplCommand::parse("/test-skill fix the bug in auth.rs"),
            ReplCommand::SlashCommand {
                name: "test-skill".to_string(),
                args: Some("fix the bug in auth.rs".to_string())
            }
        );
    }

    #[test]
    fn parses_slash_command_trims_args() {
        assert_eq!(
            ReplCommand::parse("/my-prompt   spaced args  "),
            ReplCommand::SlashCommand {
                name: "my-prompt".to_string(),
                args: Some("spaced args".to_string())
            }
        );
    }

    #[test]
    fn parses_slash_command_empty_args_becomes_none() {
        assert_eq!(
            ReplCommand::parse("/my-prompt   "),
            ReplCommand::SlashCommand {
                name: "my-prompt".to_string(),
                args: None
            }
        );
    }

    #[test]
    fn parses_unknown_for_bare_slash() {
        assert_eq!(
            ReplCommand::parse("/"),
            ReplCommand::Unknown("/".to_string())
        );
    }

    #[test]
    fn parses_regular_prompt() {
        assert_eq!(
            ReplCommand::parse("Hello world"),
            ReplCommand::Prompt("Hello world".to_string())
        );
    }

    #[test]
    fn parses_prompt_with_slash_in_middle() {
        // Slash in middle should not be treated as command.
        assert_eq!(
            ReplCommand::parse("Read /tmp/file.txt"),
            ReplCommand::Prompt("Read /tmp/file.txt".to_string())
        );
    }

    #[test]
    fn parses_empty_input() {
        assert_eq!(ReplCommand::parse(""), ReplCommand::Empty);
        assert_eq!(ReplCommand::parse("   "), ReplCommand::Empty);
        assert_eq!(ReplCommand::parse("\t\n"), ReplCommand::Empty);
    }

    #[test]
    fn trims_whitespace_from_prompt() {
        assert_eq!(
            ReplCommand::parse("  hello  "),
            ReplCommand::Prompt("hello".to_string())
        );
    }

    #[test]
    fn trims_whitespace_from_command() {
        assert_eq!(ReplCommand::parse("  /quit  "), ReplCommand::Quit);
    }

    #[test]
    fn handles_newline_in_input() {
        // Simulates input from stdin with trailing newline.
        assert_eq!(
            ReplCommand::parse("hello\n"),
            ReplCommand::Prompt("hello".to_string())
        );
        assert_eq!(ReplCommand::parse("/quit\n"), ReplCommand::Quit);
    }
}

mod skill_body_tests {
    use super::*;

    #[test]
    fn parses_skill_with_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
---

You are a testing assistant."#;
        let body = parse_skill_body(content);
        assert_eq!(body.trim(), "You are a testing assistant.");
    }

    #[test]
    fn returns_content_without_frontmatter() {
        let content = "Just plain markdown content";
        let body = parse_skill_body(content);
        assert_eq!(body, "Just plain markdown content");
    }

    #[test]
    fn handles_empty_body() {
        let content = r#"---
name: empty-skill
description: Empty body
---
"#;
        let body = parse_skill_body(content);
        assert!(body.is_empty() || body.chars().all(|c| c.is_whitespace()));
    }
}
