//! Git worktree CRUD commands.
//!
//! Two endpoints:
//! - [`list_git_branches`]   — `git branch -a` for a repo, deduplicated and
//!   stripped of `origin/HEAD`.
//! - [`create_git_worktree`] — `git worktree add -b NAME PATH BASE` plus a
//!   best-effort `git push -u origin NAME` to set up tracking.  Defaults
//!   the worktree path to a sibling directory `<repo>-<branch>` when the
//!   caller doesn't pass one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Result of creating a new worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreated {
    /// Path to the new worktree
    pub path: String,
    /// Branch name
    pub branch: String,
    /// Whether the init script was run
    pub init_script_run: bool,
    /// Output from init script (if run)
    pub init_script_output: Option<String>,
}

/// List all branches in a git repository.
#[tauri::command]
pub async fn list_git_branches(repo_path: String) -> Result<Vec<String>, String> {
    use std::process::Command;

    let path = PathBuf::from(&repo_path);
    if !path.exists() {
        return Err(format!("Repository path does not exist: {}", repo_path));
    }

    let output = Command::new("git")
        .args(["branch", "-a", "--format=%(refname:short)"])
        .current_dir(&path)
        .output()
        .map_err(|e| format!("Failed to run git branch: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git branch failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let branches: Vec<String> = stdout
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        // Filter out remote tracking branches that duplicate local ones
        .filter(|s| !s.starts_with("origin/HEAD"))
        .map(|s| {
            // Convert origin/branch to just branch for display, but keep local branches as-is
            if let Some(branch) = s.strip_prefix("origin/") {
                branch.to_string()
            } else {
                s
            }
        })
        .collect::<std::collections::HashSet<_>>() // Deduplicate
        .into_iter()
        .collect();

    Ok(branches)
}

/// Create a new git worktree.
#[tauri::command]
pub async fn create_git_worktree(
    repo_path: String,
    branch_name: String,
    base_branch: String,
    worktree_path: Option<String>,
) -> Result<WorktreeCreated, String> {
    use std::process::Command;

    let repo = PathBuf::from(&repo_path);
    if !repo.exists() {
        return Err(format!("Repository path does not exist: {}", repo_path));
    }

    // Load project config to check project associations
    let project_configs = crate::projects::list_projects()
        .await
        .map_err(|e| format!("Failed to load projects: {}", e))?;

    let project_config = project_configs.iter().find(|p| {
        p.root_path
            .canonicalize()
            .ok()
            .map(|cp| repo.canonicalize().ok().map(|cr| cp == cr).unwrap_or(false))
            .unwrap_or(false)
    });

    let _ = project_config; // project_config is only used for lookup, not for worktree settings

    // Determine worktree path
    let wt_path = if let Some(custom_path) = worktree_path {
        PathBuf::from(custom_path)
    } else {
        // Default: sibling directory named project-branch
        let project_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());
        let parent = repo.parent().unwrap_or(&repo);
        parent.join(format!(
            "{}-{}",
            project_name,
            branch_name.replace('/', "-")
        ))
    };

    // Check if path already exists
    if wt_path.exists() {
        return Err(format!(
            "Worktree path already exists: {}",
            wt_path.display()
        ));
    }

    // Create parent directory if needed
    if let Some(parent) = wt_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory: {}", e))?;
        }
    }

    // Run git worktree add
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            &branch_name,
            &wt_path.to_string_lossy(),
            &base_branch,
        ])
        .current_dir(&repo)
        .output()
        .map_err(|e| format!("Failed to run git worktree add: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {}", stderr));
    }

    // Push the new branch to remote and set up tracking
    // Run from the new worktree directory
    tracing::info!(
        "Pushing new branch '{}' to origin and setting up tracking",
        branch_name
    );
    let push_output = Command::new("git")
        .args(["push", "-u", "origin", &branch_name])
        .current_dir(&wt_path)
        .output();

    match push_output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!(
                    "Failed to push branch to remote (continuing anyway): {}",
                    stderr
                );
            } else {
                tracing::info!("Successfully pushed branch '{}' to origin", branch_name);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to run git push (continuing anyway): {}", e);
        }
    }

    Ok(WorktreeCreated {
        path: wt_path.to_string_lossy().to_string(),
        branch: branch_name,
        init_script_run: false,
        init_script_output: None,
    })
}
