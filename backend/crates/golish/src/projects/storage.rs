//! Project storage operations - load, save, delete, list.
//!
//! Projects are stored as directories: `~/.golish/projects/<slug>/config.toml`
//! Workspace state is stored alongside: `~/.golish/projects/<slug>/workspace.json`

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::schema::ProjectConfig;

/// Get the directory where project configs are stored.
pub fn projects_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("projects")
}

/// Convert a project name to a valid filename slug.
pub fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    let mut result = String::new();
    let mut last_was_hyphen = true;
    for c in slug.chars() {
        if c == '-' {
            if !last_was_hyphen {
                result.push(c);
                last_was_hyphen = true;
            }
        } else {
            result.push(c);
            last_was_hyphen = false;
        }
    }

    if result.ends_with('-') {
        result.pop();
    }

    if result.is_empty() {
        result = "project".to_string();
    }

    result
}

/// Get the directory for a project.
fn project_dir(name: &str) -> PathBuf {
    projects_dir().join(slugify(name))
}

/// Get the path to a project's config file.
fn config_path(name: &str) -> PathBuf {
    project_dir(name).join("config.toml")
}

/// Get the path to a project's workspace state file.
pub fn workspace_path(name: &str) -> PathBuf {
    project_dir(name).join("workspace.json")
}

/// Load all projects from the projects directory.
pub async fn list_projects() -> Result<Vec<ProjectConfig>> {
    let dir = projects_dir();

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut projects = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .context("Failed to read projects directory")?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        // Each project is a directory containing config.toml
        if path.is_dir() {
            let config_file = path.join("config.toml");
            if config_file.exists() {
                match load_project_from_path(&config_file).await {
                    Ok(project) => projects.push(project),
                    Err(e) => {
                        tracing::warn!("Failed to load project from {:?}: {}", config_file, e);
                    }
                }
            }
        }
        // Backwards compat: also check for legacy single-file .toml projects
        if path.extension().is_some_and(|ext| ext == "toml") && path.is_file() {
            match load_project_from_path(&path).await {
                Ok(project) => projects.push(project),
                Err(e) => {
                    tracing::warn!("Failed to load legacy project from {:?}: {}", path, e);
                }
            }
        }
    }

    projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(projects)
}

/// Load a single project by name.
pub async fn load_project(name: &str) -> Result<Option<ProjectConfig>> {
    let path = config_path(name);

    if !path.exists() {
        // Try legacy single-file format
        let legacy_path = projects_dir().join(format!("{}.toml", slugify(name)));
        if legacy_path.exists() {
            let project = load_project_from_path(&legacy_path).await?;
            return Ok(Some(project));
        }
        return Ok(None);
    }

    let project = load_project_from_path(&path).await?;
    Ok(Some(project))
}

/// Load a project from a specific file path.
async fn load_project_from_path(path: &PathBuf) -> Result<ProjectConfig> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .context("Failed to read project file")?;

    let project: ProjectConfig =
        toml::from_str(&contents).context("Failed to parse project config")?;

    Ok(project)
}

/// Save a project configuration to disk (directory-based).
pub async fn save_project(project: &ProjectConfig) -> Result<()> {
    let dir = project_dir(&project.name);

    tokio::fs::create_dir_all(&dir)
        .await
        .context("Failed to create project directory")?;

    let path = dir.join("config.toml");
    let contents = toml::to_string_pretty(project).context("Failed to serialize project config")?;

    // Atomic write
    let temp_path = path.with_extension("toml.tmp");
    tokio::fs::write(&temp_path, &contents)
        .await
        .context("Failed to write temp project file")?;

    tokio::fs::rename(&temp_path, &path)
        .await
        .context("Failed to rename temp project file")?;

    tracing::info!("Saved project '{}' to {:?}", project.name, dir);
    Ok(())
}

/// Delete a project configuration (removes the entire project directory).
pub async fn delete_project(name: &str) -> Result<bool> {
    let dir = project_dir(name);

    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .context("Failed to delete project directory")?;

        tracing::info!("Deleted project '{}' from {:?}", name, dir);
        return Ok(true);
    }

    // Try legacy single-file format
    let legacy_path = projects_dir().join(format!("{}.toml", slugify(name)));
    if legacy_path.exists() {
        tokio::fs::remove_file(&legacy_path)
            .await
            .context("Failed to delete legacy project file")?;

        tracing::info!("Deleted legacy project '{}' from {:?}", name, legacy_path);
        return Ok(true);
    }

    Ok(false)
}

/// Save workspace state JSON for a project.
pub async fn save_workspace(name: &str, json: &str) -> Result<()> {
    let dir = project_dir(name);
    tokio::fs::create_dir_all(&dir)
        .await
        .context("Failed to create project directory")?;

    let path = workspace_path(name);
    let temp_path = path.with_extension("json.tmp");

    tokio::fs::write(&temp_path, json)
        .await
        .context("Failed to write workspace state")?;

    tokio::fs::rename(&temp_path, &path)
        .await
        .context("Failed to rename workspace state file")?;

    Ok(())
}

/// Load workspace state JSON for a project.
pub async fn load_workspace(name: &str) -> Result<Option<String>> {
    let path = workspace_path(name);

    if !path.exists() {
        return Ok(None);
    }

    let contents = tokio::fs::read_to_string(&path)
        .await
        .context("Failed to read workspace state")?;

    Ok(Some(contents))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("my-project"), "my-project");
        assert_eq!(slugify("My Project"), "my-project");
        assert_eq!(slugify("my_project"), "my-project");
        assert_eq!(slugify("My  Project!"), "my-project");
        assert_eq!(slugify("  leading spaces  "), "leading-spaces");
        assert_eq!(slugify("UPPERCASE"), "uppercase");
        assert_eq!(slugify("with--multiple---dashes"), "with-multiple-dashes");
        assert_eq!(slugify(""), "project");
        assert_eq!(slugify("---"), "project");
    }
}
