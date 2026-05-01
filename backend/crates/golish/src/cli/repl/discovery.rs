//! Filesystem discovery for prompts (`<workspace>/.golish/prompts/*.md`)
//! and skills (`<workspace>/.golish/skills/<name>/SKILL.md`).
//!
//! Lookups follow a "local-first, global-fallback" rule: workspace-local
//! files in `.golish/prompts` / `.golish/skills` win over the global ones in
//! `~/.golish/`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Find a prompt file by name, checking local then global directories.
pub(super) fn find_prompt(workspace: &Path, name: &str) -> Option<PathBuf> {
    let local_path = workspace
        .join(".golish")
        .join("prompts")
        .join(format!("{}.md", name));
    if local_path.exists() {
        return Some(local_path);
    }

    if let Some(home) = dirs::home_dir() {
        let global_path = home
            .join(".golish")
            .join("prompts")
            .join(format!("{}.md", name));
        if global_path.exists() {
            return Some(global_path);
        }
    }

    None
}

/// Find a skill directory by name, checking local then global directories.
pub(super) fn find_skill(workspace: &Path, name: &str) -> Option<PathBuf> {
    let local_path = workspace.join(".golish").join("skills").join(name);
    if local_path.join("SKILL.md").exists() {
        return Some(local_path);
    }

    if let Some(home) = dirs::home_dir() {
        let global_path = home.join(".golish").join("skills").join(name);
        if global_path.join("SKILL.md").exists() {
            return Some(global_path);
        }
    }

    None
}

/// Parse SKILL.md content and extract just the body (instructions).
///
/// Strips a leading YAML frontmatter block delimited by `---` … `---`. If no
/// frontmatter is present, returns the content unchanged.
pub(super) fn parse_skill_body(content: &str) -> String {
    if !content.starts_with("---") {
        return content.to_string();
    }

    let after_first = &content[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        // Skip past `---` + body + `\n---`.
        let body_start = 3 + end_pos + 4;
        if body_start < content.len() {
            return content[body_start..].trim_start_matches('\n').to_string();
        }
    }

    content.to_string()
}

/// List available prompts and skills for the help message.
///
/// Returns `(prompts, skills)`, each sorted alphabetically and de-duplicated
/// (workspace entries hide same-named global entries).
pub(super) fn list_available_commands(workspace: &Path) -> (Vec<String>, Vec<String>) {
    let mut prompts = Vec::new();
    let mut skills = Vec::new();
    let mut seen_prompts: HashMap<String, bool> = HashMap::new();
    let mut seen_skills: HashMap<String, bool> = HashMap::new();

    let local_prompts_dir = workspace.join(".golish").join("prompts");
    if local_prompts_dir.exists() {
        if let Ok(entries) = fs::read_dir(&local_prompts_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        seen_prompts.insert(stem.to_string(), true);
                        prompts.push(stem.to_string());
                    }
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_prompts_dir = home.join(".golish").join("prompts");
        if global_prompts_dir.exists() {
            if let Ok(entries) = fs::read_dir(&global_prompts_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "md") {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if !seen_prompts.contains_key(stem) {
                                prompts.push(stem.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let local_skills_dir = workspace.join(".golish").join("skills");
    if local_skills_dir.exists() {
        if let Ok(entries) = fs::read_dir(&local_skills_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() && path.join("SKILL.md").exists() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        seen_skills.insert(name.to_string(), true);
                        skills.push(name.to_string());
                    }
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_skills_dir = home.join(".golish").join("skills");
        if global_skills_dir.exists() {
            if let Ok(entries) = fs::read_dir(&global_skills_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() && path.join("SKILL.md").exists() {
                        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                            if !seen_skills.contains_key(name) {
                                skills.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    prompts.sort();
    skills.sort();
    (prompts, skills)
}
