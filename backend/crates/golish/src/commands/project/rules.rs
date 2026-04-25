//! Rules management commands.
//!
//! Rules are persistent instructions for the AI agent, stored as `.md` files
//! in `~/.golish/rules/` (global) and `<project>/.golish/rules/` (local).
//!
//! Each rule file has YAML frontmatter:
//! ```yaml
//! ---
//! description: Always use TypeScript strict mode
//! globs: "*.ts,*.tsx"
//! alwaysApply: false
//! ---
//!
//! When writing TypeScript, always enable strict mode...
//! ```

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleInfo {
    pub name: String,
    pub path: String,
    pub source: String,
    pub description: String,
    pub globs: Option<String>,
    pub always_apply: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleFrontmatter {
    description: Option<String>,
    globs: Option<String>,
    #[serde(default)]
    always_apply: bool,
}

fn parse_rule_file(path: &Path) -> Option<RuleInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let end = after_first.find("\n---")?;
    let yaml = after_first[..end].trim();
    let fm: RuleFrontmatter = serde_yaml::from_str(yaml).ok()?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Some(RuleInfo {
        name,
        path: path.to_string_lossy().to_string(),
        source: String::new(),
        description: fm.description.unwrap_or_default(),
        globs: fm.globs,
        always_apply: fm.always_apply,
    })
}

fn discover_rules(working_directory: Option<&str>) -> Vec<RuleInfo> {
    let mut rules = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let dir = home.join(".golish").join("rules");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(mut rule) = parse_rule_file(&path) {
                        rule.source = "global".to_string();
                        rules.push(rule);
                    }
                }
            }
        }
    }

    if let Some(wd) = working_directory {
        let dir = PathBuf::from(wd).join(".golish").join("rules");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(mut rule) = parse_rule_file(&path) {
                        rule.source = "project".to_string();
                        rules.push(rule);
                    }
                }
            }
        }
    }

    rules.sort_by(|a, b| a.name.cmp(&b.name));
    rules
}

#[tauri::command]
pub async fn list_rules(working_directory: Option<String>) -> Result<Vec<RuleInfo>> {
    Ok(discover_rules(working_directory.as_deref()))
}

#[tauri::command]
pub async fn read_rule_body(rule_path: String) -> Result<String> {
    let content = std::fs::read_to_string(&rule_path)?;
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(content);
    }
    let after_first = &trimmed[3..];
    if let Some(end) = after_first.find("\n---") {
        Ok(after_first[end + 4..].trim().to_string())
    } else {
        Ok(content)
    }
}

#[tauri::command]
pub async fn save_rule(
    name: String,
    description: String,
    body: String,
    globs: Option<String>,
    always_apply: bool,
    scope: Option<String>,
    working_directory: Option<String>,
) -> Result<String> {
    let base_dir = match scope.as_deref() {
        Some("project") | Some("local") => {
            let wd = working_directory.ok_or_else(|| {
                crate::error::GolishError::Internal(
                    "Working directory required for project rules".into(),
                )
            })?;
            PathBuf::from(wd).join(".golish").join("rules")
        }
        _ => {
            let home = dirs::home_dir().ok_or_else(|| {
                crate::error::GolishError::Internal("No home directory".into())
            })?;
            home.join(".golish").join("rules")
        }
    };

    std::fs::create_dir_all(&base_dir)?;

    let mut content = String::from("---\n");
    content.push_str(&format!(
        "description: \"{}\"\n",
        description.replace('"', "\\\"")
    ));
    if let Some(g) = &globs {
        if !g.is_empty() {
            content.push_str(&format!("globs: \"{g}\"\n"));
        }
    }
    content.push_str(&format!("alwaysApply: {always_apply}\n"));
    content.push_str("---\n\n");
    content.push_str(&body);
    if !body.ends_with('\n') {
        content.push('\n');
    }

    let file_path = base_dir.join(format!("{name}.md"));
    std::fs::write(&file_path, &content)?;

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn delete_rule(rule_path: String) -> Result<()> {
    let path = Path::new(&rule_path);
    if !path.exists() {
        return Err(crate::error::GolishError::Internal(format!(
            "Rule not found: {}",
            rule_path
        )));
    }
    std::fs::remove_file(path)?;
    Ok(())
}
