//! Lightweight REPL (Read-Eval-Print-Loop) for golish-cli.
//!
//! Provides an interactive mode when no prompt is provided via `-e` or `-f`.
//!
//! Supports commands:
//! - `/quit`, `/exit`, `/q` — exit the REPL
//! - `/<prompt-name>` or `/<skill-name>` `[args]` — execute a prompt or skill with optional args
//! - any other input — sent as a prompt to the agent
//!
//! Sub-modules:
//!
//! - [`discovery`] — workspace/global lookup of prompts and skills

use std::fs;
use std::io::{self, BufRead, Write};

use anyhow::Result;

use super::bootstrap::CliContext;
use super::runner::execute_once;

mod discovery;

#[cfg(test)]
mod tests;

use discovery::{find_prompt, find_skill, list_available_commands, parse_skill_body};

/// REPL command variants.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplCommand {
    /// Exit the REPL.
    Quit,
    /// Unknown command (will show help).
    Unknown(String),
    /// Regular prompt to send to the agent.
    Prompt(String),
    /// Slash command (prompt or skill) with optional arguments.
    SlashCommand { name: String, args: Option<String> },
    /// Empty input (skip).
    Empty,
}

impl ReplCommand {
    /// Parse user input into a REPL command.
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return ReplCommand::Empty;
        }

        if let Some(after_slash) = trimmed.strip_prefix('/') {
            // Built-in commands first (case-insensitive).
            let lower = after_slash.to_lowercase();
            if lower == "quit" || lower == "exit" || lower == "q" {
                return ReplCommand::Quit;
            }

            // Parse as slash command: `/name [args]`.
            let parts: Vec<&str> = after_slash.splitn(2, ' ').collect();
            let name = parts[0].to_string();
            let args = parts
                .get(1)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            if name.is_empty() {
                return ReplCommand::Unknown(trimmed.to_string());
            }

            ReplCommand::SlashCommand { name, args }
        } else {
            ReplCommand::Prompt(trimmed.to_string())
        }
    }
}

/// Run an interactive REPL session.
///
/// Returns when the user exits or on EOF (Ctrl+D).
pub async fn run_repl(ctx: &mut CliContext) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!("golish-cli interactive mode");
    eprintln!("Type /quit to exit\n");

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        if stdin.lock().read_line(&mut input)? == 0 {
            // EOF (Ctrl+D)
            eprintln!("\nGoodbye!");
            break;
        }

        match ReplCommand::parse(&input) {
            ReplCommand::Empty => continue,
            ReplCommand::Quit => {
                eprintln!("Goodbye!");
                break;
            }
            ReplCommand::Unknown(cmd) => {
                eprintln!("Unknown command: {}", cmd);
                eprintln!("Available: /quit, /exit, /q");
                continue;
            }
            ReplCommand::SlashCommand { name, args } => {
                handle_slash_command(ctx, &name, args.as_deref()).await;
                println!();
            }
            ReplCommand::Prompt(prompt) => {
                if let Err(e) = execute_once(ctx, &prompt).await {
                    eprintln!("Error: {}", e);
                }
                println!();
            }
        }
    }

    Ok(())
}

/// Resolve and execute a `/name [args]` invocation.
///
/// Prompts take precedence over skills with the same name. Falls back to
/// listing available commands when nothing matches.
async fn handle_slash_command(ctx: &mut CliContext, name: &str, args: Option<&str>) {
    if let Some(prompt_path) = find_prompt(&ctx.workspace, name) {
        match fs::read_to_string(&prompt_path) {
            Ok(content) => {
                let full_content = match args {
                    Some(args_str) => format!("{}\n\n{}", content, args_str),
                    None => content,
                };
                if let Err(e) = execute_once(ctx, &full_content).await {
                    eprintln!("Error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to read prompt '{}': {}", name, e);
            }
        }
    } else if let Some(skill_path) = find_skill(&ctx.workspace, name) {
        let skill_md_path = skill_path.join("SKILL.md");
        match fs::read_to_string(&skill_md_path) {
            Ok(content) => {
                let body = parse_skill_body(&content);
                let full_content = match args {
                    Some(args_str) => format!("{}\n\n{}", body, args_str),
                    None => body,
                };
                if let Err(e) = execute_once(ctx, &full_content).await {
                    eprintln!("Error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to read skill '{}': {}", name, e);
            }
        }
    } else {
        eprintln!("Unknown command: /{}", name);
        let (prompts, skills) = list_available_commands(&ctx.workspace);
        eprintln!("Available: /quit, /exit, /q");
        if !prompts.is_empty() {
            eprintln!(
                "Prompts: {}",
                prompts
                    .iter()
                    .map(|p| format!("/{}", p))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !skills.is_empty() {
            eprintln!(
                "Skills: {}",
                skills
                    .iter()
                    .map(|s| format!("/{}", s))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}
