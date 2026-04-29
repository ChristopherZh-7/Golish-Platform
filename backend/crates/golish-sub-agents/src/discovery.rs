//! Agent discovery: merge built-in (system) agents with file-based agents.
//!
//! Priority: file-based agents override built-in agents with the same ID.
//! System agents (worker, memorist, reflector) always exist but their prompts
//! can be overridden by file versions.

use std::collections::HashMap;
use std::path::Path;

use crate::defaults::create_default_sub_agents;
use crate::definition::SubAgentDefinition;
use crate::file_loader::{agent_dirs, load_agents_from_dir, serialize_agent_to_md};

/// IDs of system-level agents that cannot be deleted.
const SYSTEM_AGENT_IDS: &[&str] = &["worker", "memorist", "reflector", "orchestrator"];

/// Discover all available agents by merging built-in defaults with file-based definitions.
///
/// - Built-in agents serve as fallbacks (created as files on first run).
/// - File-based agents override built-in agents with the same ID.
/// - System agents (worker, memorist, reflector) are always present.
pub fn discover_agents(workspace: Option<&Path>) -> Vec<SubAgentDefinition> {
    let mut agent_map: HashMap<String, SubAgentDefinition> = HashMap::new();

    // 1. Load built-in defaults as the base
    for mut agent in create_default_sub_agents() {
        if SYSTEM_AGENT_IDS.contains(&agent.id.as_str()) {
            agent.is_system = true;
        }
        agent_map.insert(agent.id.clone(), agent);
    }

    // 2. Load file-based agents (global first, then local — local overrides global)
    for dir in agent_dirs(workspace) {
        for mut agent in load_agents_from_dir(&dir) {
            if SYSTEM_AGENT_IDS.contains(&agent.id.as_str()) {
                agent.is_system = true;
            }
            agent_map.insert(agent.id.clone(), agent);
        }
    }

    let mut agents: Vec<_> = agent_map.into_values().collect();
    agents.sort_by(|a, b| a.id.cmp(&b.id));
    agents
}

/// Ensure default agent files exist on disk. Called on first startup.
/// Creates `~/.golish/agents/*.md` for each built-in agent that doesn't have a file yet.
pub fn seed_default_agent_files() -> anyhow::Result<usize> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    let agents_dir = home.join(".golish").join("agents");
    std::fs::create_dir_all(&agents_dir)?;

    let defaults = create_default_sub_agents();
    let mut created = 0;

    for mut agent in defaults {
        let file_path = agents_dir.join(format!("{}.md", agent.id));
        if !file_path.exists() {
            if SYSTEM_AGENT_IDS.contains(&agent.id.as_str()) {
                agent.is_system = true;
            }
            let content = serialize_agent_to_md(&agent);
            std::fs::write(&file_path, content)?;
            tracing::info!(id = %agent.id, path = %file_path.display(), "Created default agent file");
            created += 1;
        }
    }

    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::AgentSource;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_discover_agents_defaults_only() {
        let agents = discover_agents(None);
        assert!(!agents.is_empty());

        let ids: Vec<_> = agents.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"worker"));
        assert!(ids.contains(&"pentester"));
        assert!(ids.contains(&"memorist"));
        assert!(ids.contains(&"orchestrator"));
    }

    #[test]
    fn test_discover_agents_file_overrides_builtin() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".golish").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        fs::write(
            agents_dir.join("pentester.md"),
            "---\nname: Custom Pentester\ndescription: My custom pentester\nallowed_tools:\n  - run_pty_cmd\n---\n\nCustom prompt here.\n",
        ).unwrap();

        let agents = discover_agents(Some(dir.path()));
        let pentester = agents.iter().find(|a| a.id == "pentester").unwrap();

        assert_eq!(pentester.name, "Custom Pentester");
        assert_eq!(pentester.system_prompt, "Custom prompt here.");
        assert!(matches!(pentester.source, AgentSource::File(_)));
    }

    #[test]
    fn test_discover_agents_new_agent_from_file() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".golish").join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        fs::write(
            agents_dir.join("my-custom-agent.md"),
            "---\nname: My Custom Agent\ndescription: Does custom things\n---\n\nCustom agent prompt.\n",
        ).unwrap();

        let agents = discover_agents(Some(dir.path()));
        let custom = agents.iter().find(|a| a.id == "my-custom-agent");
        assert!(custom.is_some());
        assert_eq!(custom.unwrap().name, "My Custom Agent");
    }

    #[test]
    fn test_system_agents_flagged() {
        let agents = discover_agents(None);
        let worker = agents.iter().find(|a| a.id == "worker").unwrap();
        let memorist = agents.iter().find(|a| a.id == "memorist").unwrap();
        let reflector = agents.iter().find(|a| a.id == "reflector").unwrap();
        let orchestrator = agents.iter().find(|a| a.id == "orchestrator").unwrap();
        let pentester = agents.iter().find(|a| a.id == "pentester").unwrap();

        assert!(worker.is_system);
        assert!(memorist.is_system);
        assert!(reflector.is_system);
        assert!(orchestrator.is_system);
        assert!(!pentester.is_system);
    }

    #[test]
    fn test_seed_default_agent_files() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".golish").join("agents");
        // seed_default_agent_files uses home_dir, so we test file_loader directly
        fs::create_dir_all(&agents_dir).unwrap();

        let defaults = create_default_sub_agents();
        for agent in &defaults {
            let path = agents_dir.join(format!("{}.md", agent.id));
            let content = serialize_agent_to_md(agent);
            fs::write(&path, content).unwrap();
        }

        let loaded = load_agents_from_dir(&agents_dir);
        assert_eq!(loaded.len(), defaults.len());
    }
}
