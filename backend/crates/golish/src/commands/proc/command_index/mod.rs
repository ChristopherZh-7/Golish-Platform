//! Command index used by "auto" input mode to classify input as a terminal
//! command vs a natural-language agent prompt.
//!
//! `mod.rs` owns the [`CommandIndex`] struct, its build/classify logic, and
//! the Tauri entry point. Sibling modules contribute pure helpers:
//!
//! - [`path_resolution`] — login-shell PATH discovery, executable-bit checks,
//!   and shell-operator detection inside quoted strings.
//! - [`shell_builtins`] — `$SHELL` detection plus the per-shell builtin
//!   tables (zsh/bash/fish/PowerShell/cmd/POSIX-fallback).

use std::collections::HashSet;
use std::sync::RwLock;

use serde::Serialize;

mod path_resolution;
mod shell_builtins;

use path_resolution::{contains_shell_operator, first_token, is_executable, resolve_shell_path};
use shell_builtins::{detect_shell_type, shell_builtins};

/// Index of executable commands available in the user's PATH plus shell builtins.
/// Used by "auto" input mode to classify whether user input is a command or natural language.
pub struct CommandIndex {
    commands: RwLock<HashSet<String>>,
    initialized: RwLock<bool>,
}

/// Result of classifying user input as terminal command vs agent prompt.
#[derive(Debug, Clone, Serialize)]
pub struct ClassifyResult {
    pub route: String,
    pub detected_command: Option<String>,
}

impl Default for CommandIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandIndex {
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashSet::new()),
            initialized: RwLock::new(false),
        }
    }

    /// Build the command index by scanning PATH directories for executables
    /// and adding shell builtins (detected from `$SHELL` env var).
    pub fn build(&self) {
        let shell_type = detect_shell_type();
        let mut commands = HashSet::new();

        // Resolve the user's full shell PATH. On macOS, GUI apps launched from
        // the dock/Finder don't inherit the user's shell PATH, so directories
        // like ~/.local/bin won't be included in std::env::var("PATH").
        let path_var = resolve_shell_path().or_else(|| std::env::var("PATH").ok());

        if let Some(ref path_var) = path_var {
            for dir in path_var.split(':') {
                let dir_path = std::path::Path::new(dir);
                if let Ok(entries) = std::fs::read_dir(dir_path) {
                    for entry in entries.flatten() {
                        // Use std::fs::metadata (not entry.metadata()) to follow
                        // symlinks. On Unix, entry.metadata() is equivalent to
                        // lstat and reports symlinks as non-files.
                        if let Ok(metadata) = std::fs::metadata(entry.path()) {
                            if metadata.is_file() && is_executable(&metadata) {
                                if let Some(name) = entry.file_name().to_str() {
                                    commands.insert(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        for builtin in shell_builtins(shell_type) {
            commands.insert(builtin.to_string());
        }

        let count = commands.len();
        *self.commands.write().unwrap() = commands;
        *self.initialized.write().unwrap() = true;
        tracing::info!("[command-index] Built index with {} commands", count);
    }

    /// Classify user input as either a terminal command or an agent prompt.
    pub fn classify(&self, input: &str) -> ClassifyResult {
        let input = input.trim();
        if input.is_empty() {
            return ClassifyResult {
                route: "agent".to_string(),
                detected_command: None,
            };
        }

        // 1. Path prefix → Terminal.
        if input.starts_with("./") || input.starts_with('/') || input.starts_with("~/") {
            return ClassifyResult {
                route: "terminal".to_string(),
                detected_command: None,
            };
        }

        // 2. Shell operators → Terminal.
        if contains_shell_operator(input) {
            let first_token = first_token(input);
            let commands = self.commands.read().unwrap();
            let detected = if commands.contains(first_token) {
                Some(first_token.to_string())
            } else {
                None
            };
            return ClassifyResult {
                route: "terminal".to_string(),
                detected_command: detected,
            };
        }

        // 3. First token vs known commands.
        let first = first_token(input);
        let commands = self.commands.read().unwrap();

        if commands.contains(first) {
            let tokens: Vec<&str> = input.split_whitespace().collect();

            // Has flags (e.g. -x, --foo) → definitely a command.
            if tokens.iter().any(|t| t.starts_with('-')) {
                return ClassifyResult {
                    route: "terminal".to_string(),
                    detected_command: Some(first.to_string()),
                };
            }

            // Only 1-2 tokens → likely a command (e.g. "ls", "git status").
            if tokens.len() <= 2 {
                return ClassifyResult {
                    route: "terminal".to_string(),
                    detected_command: Some(first.to_string()),
                };
            }

            // 3+ plain English tokens → likely natural language that happens
            // to start with a command name (e.g. "make sure the tests pass").
            let rest_tokens = &tokens[1..];
            let all_plain_words = rest_tokens.iter().all(|t| {
                t.chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '\'' || c == ',')
            });

            if all_plain_words && rest_tokens.len() >= 2 {
                return ClassifyResult {
                    route: "agent".to_string(),
                    detected_command: Some(first.to_string()),
                };
            }

            // Has paths, special chars, etc. → command.
            return ClassifyResult {
                route: "terminal".to_string(),
                detected_command: Some(first.to_string()),
            };
        }

        // 4. First token not recognized → treat as natural language.
        ClassifyResult {
            route: "agent".to_string(),
            detected_command: None,
        }
    }
}

// -- Tauri command --

#[tauri::command]
pub async fn classify_input(
    state: tauri::State<'_, crate::state::AppState>,
    input: String,
) -> Result<ClassifyResult, String> {
    Ok(state.command_index.classify(&input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_index(extra_commands: &[&str]) -> CommandIndex {
        let index = CommandIndex::new();
        {
            let mut cmds = index.commands.write().unwrap();
            for cmd in [
                "ls", "cd", "git", "cat", "grep", "echo", "python", "node", "cargo", "make",
                "docker", "ssh", "curl", "find", "rm", "mkdir", "cp", "mv",
            ] {
                cmds.insert(cmd.to_string());
            }
            for cmd in extra_commands {
                cmds.insert(cmd.to_string());
            }
            *index.initialized.write().unwrap() = true;
        }
        index
    }

    #[test]
    fn test_path_prefix_routes_to_terminal() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("./script.sh").route, "terminal");
        assert_eq!(index.classify("/usr/bin/python3").route, "terminal");
        assert_eq!(index.classify("~/bin/run.sh").route, "terminal");
    }

    #[test]
    fn test_shell_operators_route_to_terminal() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("cat foo | grep bar").route, "terminal");
        assert_eq!(index.classify("echo hello > file.txt").route, "terminal");
        assert_eq!(index.classify("ls && pwd").route, "terminal");
        assert_eq!(index.classify("cmd1 ; cmd2").route, "terminal");
    }

    #[test]
    fn test_known_command_with_flags() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("ls -la").route, "terminal");
        assert_eq!(index.classify("git --version").route, "terminal");
        assert_eq!(index.classify("docker run --rm").route, "terminal");
    }

    #[test]
    fn test_single_known_command() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("ls").route, "terminal");
        assert_eq!(index.classify("git status").route, "terminal");
    }

    #[test]
    fn test_natural_language_starting_with_command() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("make sure the tests pass").route, "agent");
        assert_eq!(index.classify("find all the bugs").route, "agent");
    }

    #[test]
    fn test_unknown_first_token() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("what files are here").route, "agent");
        assert_eq!(index.classify("explain this code").route, "agent");
        assert_eq!(index.classify("python is great").route, "agent");
    }

    #[test]
    fn test_command_with_path_args() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("cat src/main.rs").route, "terminal");
    }

    #[test]
    fn test_empty_input() {
        let index = build_test_index(&[]);
        assert_eq!(index.classify("").route, "agent");
        assert_eq!(index.classify("  ").route, "agent");
    }

    #[test]
    fn test_shell_operators_in_quotes_ignored() {
        let index = build_test_index(&[]);
        // Pipe inside quotes should not be treated as operator,
        // and "echo \"hello | world\"" has 2 tokens after quoting → terminal.
        assert_eq!(index.classify("echo \"hello | world\"").route, "terminal");
    }

    #[test]
    fn test_classify_result_detected_command() {
        let index = build_test_index(&[]);
        let result = index.classify("git status");
        assert_eq!(result.route, "terminal");
        assert_eq!(result.detected_command, Some("git".to_string()));

        let result = index.classify("what is this");
        assert_eq!(result.route, "agent");
        assert_eq!(result.detected_command, None);
    }
}
