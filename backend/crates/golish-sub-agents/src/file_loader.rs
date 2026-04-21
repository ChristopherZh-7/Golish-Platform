//! Load sub-agent definitions from `.golish/agents/*.md` files.
//!
//! File format follows Cursor's subagent convention:
//! ```markdown
//! ---
//! name: pentester
//! description: Penetration testing specialist...
//! model: inherit
//! allowed_tools:
//!   - run_pty_cmd
//!   - read_file
//! max_iterations: 50
//! timeout_secs: 900
//! idle_timeout_secs: 300
//! readonly: false
//! is_background: false
//! ---
//!
//! <identity>
//! You are a penetration testing specialist...
//! </identity>
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::definition::{AgentSource, SubAgentDefinition};

/// YAML frontmatter parsed from an agent `.md` file.
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    max_iterations: Option<usize>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    idle_timeout_secs: Option<u64>,
    #[serde(default)]
    readonly: Option<bool>,
    #[serde(default)]
    is_background: Option<bool>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    prompt_template: Option<String>,
}

/// Info about an agent file, for listing in the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFileInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    /// "built-in", "file"
    pub source: String,
    /// "global", "project", or "built-in"
    pub scope: String,
    pub is_system: bool,
    pub model: Option<String>,
    pub allowed_tools: Vec<String>,
    pub max_iterations: usize,
    pub timeout_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
    pub readonly: bool,
    pub is_background: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
}

impl From<&SubAgentDefinition> for AgentFileInfo {
    fn from(def: &SubAgentDefinition) -> Self {
        let scope = match &def.source {
            AgentSource::BuiltIn => "built-in",
            AgentSource::File(p) => {
                let is_global = dirs::home_dir()
                    .map(|h| p.starts_with(h.join(".golish")))
                    .unwrap_or(false);
                if is_global { "global" } else { "project" }
            }
        };
        Self::build(def, scope)
    }
}

impl AgentFileInfo {
    fn build(def: &SubAgentDefinition, scope: &str) -> Self {
        Self {
            id: def.id.clone(),
            name: def.name.clone(),
            description: def.description.clone(),
            path: match &def.source {
                AgentSource::File(p) => p.to_string_lossy().to_string(),
                AgentSource::BuiltIn => String::new(),
            },
            source: match &def.source {
                AgentSource::BuiltIn => "built-in".to_string(),
                AgentSource::File(_) => "file".to_string(),
            },
            scope: scope.to_string(),
            is_system: def.is_system,
            model: def.model_override.as_ref().map(|(_, m)| m.clone()),
            allowed_tools: def.allowed_tools.clone(),
            max_iterations: def.max_iterations,
            timeout_secs: def.timeout_secs,
            idle_timeout_secs: def.idle_timeout_secs,
            readonly: def.readonly,
            is_background: def.is_background,
            temperature: def.temperature,
            max_tokens: def.max_tokens,
            top_p: def.top_p,
        }
    }
}

/// Parse a single `.md` agent file into a `SubAgentDefinition`.
pub fn parse_agent_file(path: &Path) -> anyhow::Result<SubAgentDefinition> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&content)?;
    let fm: AgentFrontmatter = serde_yaml::from_str(&frontmatter)?;

    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let id = file_stem.clone();
    let name = fm.name.unwrap_or_else(|| title_case(&file_stem));
    let description = fm.description.unwrap_or_default();

    let mut def = SubAgentDefinition::new(&id, name, description, body.trim());
    def.source = AgentSource::File(path.to_path_buf());

    if let Some(tools) = fm.allowed_tools {
        def.allowed_tools = tools;
    }
    if let Some(max) = fm.max_iterations {
        def.max_iterations = max;
    }
    if let Some(t) = fm.timeout_secs {
        def.timeout_secs = Some(t);
    }
    if let Some(t) = fm.idle_timeout_secs {
        def.idle_timeout_secs = Some(t);
    }
    if let Some(r) = fm.readonly {
        def.readonly = r;
    }
    if let Some(b) = fm.is_background {
        def.is_background = b;
    }
    if let Some(t) = fm.temperature {
        def.temperature = Some(t);
    }
    if let Some(m) = fm.max_tokens {
        def.max_tokens = Some(m);
    }
    if let Some(t) = fm.top_p {
        def.top_p = Some(t);
    }
    if let Some(pt) = fm.prompt_template {
        def.prompt_template = Some(pt);
    }

    // "inherit" or empty means no override
    if let Some(model) = fm.model {
        if model != "inherit" && !model.is_empty() {
            if model == "fast" {
                def.model_override = Some(("auto".to_string(), "fast".to_string()));
            } else {
                def.model_override = Some(("auto".to_string(), model));
            }
        }
    }

    Ok(def)
}

/// Scan a directory for agent `.md` files and parse them.
pub fn load_agents_from_dir(dir: &Path) -> Vec<SubAgentDefinition> {
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut agents = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "Failed to read agents directory");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            match parse_agent_file(&path) {
                Ok(agent) => {
                    tracing::debug!(id = %agent.id, path = %path.display(), "Loaded agent from file");
                    agents.push(agent);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Failed to parse agent file");
                }
            }
        }
    }

    agents
}

/// Serialize a `SubAgentDefinition` back to `.md` format for saving.
pub fn serialize_agent_to_md(def: &SubAgentDefinition) -> String {
    let mut fm = String::from("---\n");
    fm.push_str(&format!("name: {}\n", def.name));
    fm.push_str(&format!(
        "description: \"{}\"\n",
        def.description.replace('"', "\\\"")
    ));

    if let Some((_, model)) = &def.model_override {
        fm.push_str(&format!("model: {model}\n"));
    } else {
        fm.push_str("model: inherit\n");
    }

    if !def.allowed_tools.is_empty() {
        fm.push_str("allowed_tools:\n");
        for tool in &def.allowed_tools {
            fm.push_str(&format!("  - {tool}\n"));
        }
    }

    fm.push_str(&format!("max_iterations: {}\n", def.max_iterations));

    if let Some(t) = def.timeout_secs {
        fm.push_str(&format!("timeout_secs: {t}\n"));
    }
    if let Some(t) = def.idle_timeout_secs {
        fm.push_str(&format!("idle_timeout_secs: {t}\n"));
    }
    if def.readonly {
        fm.push_str("readonly: true\n");
    }
    if def.is_background {
        fm.push_str("is_background: true\n");
    }
    if let Some(t) = def.temperature {
        fm.push_str(&format!("temperature: {t}\n"));
    }
    if let Some(m) = def.max_tokens {
        fm.push_str(&format!("max_tokens: {m}\n"));
    }
    if let Some(t) = def.top_p {
        fm.push_str(&format!("top_p: {t}\n"));
    }

    fm.push_str("---\n\n");
    fm.push_str(&def.system_prompt);
    if !def.system_prompt.ends_with('\n') {
        fm.push('\n');
    }
    fm
}

/// Get the agent directories to scan (global + local).
pub fn agent_dirs(workspace: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Global: ~/.golish/agents/
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".golish").join("agents"));
    }

    // Local (project): <workspace>/.golish/agents/
    if let Some(ws) = workspace {
        dirs.push(ws.join(".golish").join("agents"));
    }

    dirs
}

// ── Helpers ──────────────────────────────────────────

fn split_frontmatter(content: &str) -> anyhow::Result<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        anyhow::bail!("No YAML frontmatter found (file must start with ---)");
    }

    let after_first = &trimmed[3..];
    let end = after_first
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("No closing --- for frontmatter"))?;

    let frontmatter = after_first[..end].trim().to_string();
    let body = after_first[end + 4..].to_string();

    Ok((frontmatter, body))
}

fn title_case(s: &str) -> String {
    s.split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_split_frontmatter() {
        let content = "---\nname: test\ndescription: hello\n---\n\nBody content here.";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert_eq!(fm, "name: test\ndescription: hello");
        assert!(body.contains("Body content here."));
    }

    #[test]
    fn test_split_frontmatter_no_frontmatter() {
        let content = "Just some text without frontmatter";
        assert!(split_frontmatter(content).is_err());
    }

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("pentester"), "Pentester");
        assert_eq!(title_case("js-analyzer"), "Js Analyzer");
        assert_eq!(title_case("code_reviewer"), "Code Reviewer");
    }

    #[test]
    fn test_parse_agent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pentester.md");
        fs::write(
            &path,
            r#"---
name: Pentester
description: "Security testing specialist"
model: inherit
allowed_tools:
  - run_pty_cmd
  - read_file
max_iterations: 50
timeout_secs: 900
readonly: false
---

You are a penetration testing specialist.
"#,
        )
        .unwrap();

        let agent = parse_agent_file(&path).unwrap();
        assert_eq!(agent.id, "pentester");
        assert_eq!(agent.name, "Pentester");
        assert_eq!(agent.description, "Security testing specialist");
        assert_eq!(agent.allowed_tools, vec!["run_pty_cmd", "read_file"]);
        assert_eq!(agent.max_iterations, 50);
        assert_eq!(agent.timeout_secs, Some(900));
        assert!(!agent.readonly);
        assert!(agent.model_override.is_none());
        assert!(matches!(agent.source, AgentSource::File(_)));
    }

    #[test]
    fn test_parse_agent_file_with_model() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fast-search.md");
        fs::write(
            &path,
            "---\nname: Fast Search\ndescription: Quick search\nmodel: fast\n---\n\nSearch fast.\n",
        )
        .unwrap();

        let agent = parse_agent_file(&path).unwrap();
        assert_eq!(
            agent.model_override,
            Some(("auto".to_string(), "fast".to_string()))
        );
    }

    #[test]
    fn test_load_agents_from_dir() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("agent1.md"),
            "---\nname: Agent1\ndescription: First\n---\n\nPrompt 1\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("agent2.md"),
            "---\nname: Agent2\ndescription: Second\n---\n\nPrompt 2\n",
        )
        .unwrap();
        fs::write(dir.path().join("not-an-agent.txt"), "ignored").unwrap();

        let agents = load_agents_from_dir(dir.path());
        assert_eq!(agents.len(), 2);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let agent = SubAgentDefinition::new(
            "test",
            "Test Agent",
            "A test agent",
            "You are a test agent.",
        )
        .with_tools(vec!["read_file".to_string(), "write_file".to_string()])
        .with_max_iterations(30)
        .with_timeout(600);

        let md = serialize_agent_to_md(&agent);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: Test Agent"));
        assert!(md.contains("description: \"A test agent\""));
        assert!(md.contains("- read_file"));
        assert!(md.contains("- write_file"));
        assert!(md.contains("max_iterations: 30"));
        assert!(md.contains("You are a test agent."));
    }
}
