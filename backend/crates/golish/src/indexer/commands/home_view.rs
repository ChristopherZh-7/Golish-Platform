//! Project & worktree summaries that back the Home view.
//!
//! Walks every configured project (`crate::projects::list_projects`),
//! enumerates its git worktrees, and decorates each with a recent diff
//! stat + last-commit timestamp.  The result drives the Home dashboard.
//!
//! Several helpers here are exposed `pub(super)` because the
//! [`super::hidden_dirs::list_recent_directories`] command needs them too:
//! - [`format_relative_time`] for the "2h ago" rendering.
//! - [`get_git_stats`] for the per-directory diff summary.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

use super::codebases::get_codebase_file_count;

/// Git branch information for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Branch name (e.g., "main", "feature/new-components")
    pub name: String,
    /// Full path to the worktree/checkout
    pub path: String,
    /// Number of files with changes
    pub file_count: u32,
    /// Lines added
    pub insertions: i32,
    /// Lines deleted
    pub deletions: i32,
    /// Last activity time (ISO 8601 string)
    pub last_activity: String,
}

/// Project information for the home view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Path to the project root
    pub path: String,
    /// Project name (directory name)
    pub name: String,
    /// Git branches with their stats
    pub branches: Vec<BranchInfo>,
    /// Number of warnings/errors
    pub warnings: u32,
    /// Last activity time (relative, e.g., "2h ago")
    pub last_activity: String,
}

/// Helper to format duration as relative time.
pub(super) fn format_relative_time(datetime: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(datetime);

    if duration.num_days() > 0 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}

/// Get the last commit time for a git directory.
fn get_last_commit_time(path: &std::path::Path) -> Option<chrono::DateTime<chrono::Utc>> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .current_dir(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let date_str = String::from_utf8_lossy(&output.stdout);
    let date_str = date_str.trim();

    // Parse ISO 8601 date (e.g., "2025-01-28T15:30:00+00:00")
    chrono::DateTime::parse_from_rfc3339(date_str)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Represents a single git worktree.
#[derive(Debug, Clone)]
struct GitWorktree {
    /// Path to the worktree directory
    path: PathBuf,
    /// Branch name (or "detached" for detached HEAD, or commit hash)
    branch: String,
}

/// Get list of git worktrees for a repository.
fn get_git_worktrees(repo_path: &std::path::Path) -> Vec<GitWorktree> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut is_bare = false;

    for line in stdout.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            // Save previous worktree if complete
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                if !is_bare {
                    worktrees.push(GitWorktree { path, branch });
                }
            }
            current_path = Some(PathBuf::from(path_str));
            current_branch = None;
            is_bare = false;
        } else if line == "bare" {
            is_bare = true;
            // Bare worktrees don't have a branch, skip them
            current_path = None;
        } else if let Some(branch_ref) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch_ref.to_string());
        } else if line == "detached" {
            current_branch = Some("detached".to_string());
        } else if line.is_empty() {
            // End of worktree entry
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                if !is_bare {
                    worktrees.push(GitWorktree { path, branch });
                }
            }
            is_bare = false;
        }
    }

    // Handle last entry
    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        if !is_bare {
            worktrees.push(GitWorktree { path, branch });
        }
    }

    worktrees
}

/// Helper to get git status for a directory.
///
/// Returns `(branch, insertions, deletions, file_count)` parsed from
/// `git diff --stat HEAD`.
pub(super) fn get_git_stats(path: &std::path::Path) -> Option<(String, i32, i32, u32)> {
    use std::process::Command;

    // Get current branch
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    if !branch_output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    // Get diff stats
    let diff_output = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;

    let mut insertions = 0i32;
    let mut deletions = 0i32;
    let mut file_count = 0u32;

    if diff_output.status.success() {
        let diff_str = String::from_utf8_lossy(&diff_output.stdout);
        // Parse the summary line: "X files changed, Y insertions(+), Z deletions(-)"
        for line in diff_str.lines() {
            if line.contains("changed") {
                // Count files from individual file lines
                file_count = diff_str.lines().filter(|l| l.contains("|")).count() as u32;

                // Parse insertions
                if let Some(ins_match) = line.find("insertion") {
                    let before_ins = &line[..ins_match];
                    if let Some(num_str) = before_ins.split(',').next_back() {
                        insertions = num_str.trim().parse().unwrap_or(0);
                    }
                }

                // Parse deletions
                if let Some(del_match) = line.find("deletion") {
                    let before_del = &line[..del_match];
                    if let Some(num_str) = before_del.split(',').next_back() {
                        deletions = num_str.trim().parse().unwrap_or(0);
                    }
                }
            }
        }
    }

    Some((branch, insertions, deletions, file_count))
}

/// Helper to get git stats for a specific worktree directory.
///
/// Like [`get_git_stats`] but doesn't bother resolving the branch (the
/// caller already knows it from `git worktree list`).
fn get_worktree_stats(worktree_path: &std::path::Path) -> (i32, i32, u32) {
    use std::process::Command;

    // Get diff stats for this worktree
    let diff_output = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(worktree_path)
        .output();

    let mut insertions = 0i32;
    let mut deletions = 0i32;
    let mut file_count = 0u32;

    if let Ok(output) = diff_output {
        if output.status.success() {
            let diff_str = String::from_utf8_lossy(&output.stdout);
            // Parse the summary line: "X files changed, Y insertions(+), Z deletions(-)"
            for line in diff_str.lines() {
                if line.contains("changed") {
                    // Count files from individual file lines
                    file_count = diff_str.lines().filter(|l| l.contains("|")).count() as u32;

                    // Parse insertions
                    if let Some(ins_match) = line.find("insertion") {
                        let before_ins = &line[..ins_match];
                        if let Some(num_str) = before_ins.split(',').next_back() {
                            insertions = num_str.trim().parse().unwrap_or(0);
                        }
                    }

                    // Parse deletions
                    if let Some(del_match) = line.find("deletion") {
                        let before_del = &line[..del_match];
                        if let Some(num_str) = before_del.split(',').next_back() {
                            deletions = num_str.trim().parse().unwrap_or(0);
                        }
                    }
                }
            }
        }
    }

    (insertions, deletions, file_count)
}

/// List projects for the home view.
///
/// Returns configured projects with git worktree information.
#[tauri::command]
pub async fn list_projects_for_home(
    _state: State<'_, AppState>,
) -> Result<Vec<ProjectInfo>, String> {
    // Load projects from storage (~/.golish/projects/)
    let project_configs = crate::projects::list_projects()
        .await
        .map_err(|e| format!("Failed to load projects: {}", e))?;

    let projects: Vec<ProjectInfo> = project_configs
        .iter()
        .filter_map(|config| {
            let path = &config.root_path;
            if !path.exists() {
                return None;
            }

            // Get all git worktrees for this project
            let worktrees = get_git_worktrees(path);

            // Convert worktrees to (BranchInfo, Option<DateTime>) for sorting
            let mut branches_with_time: Vec<(BranchInfo, Option<chrono::DateTime<chrono::Utc>>)> =
                worktrees
                    .iter()
                    .map(|wt| {
                        let (insertions, deletions, file_count) = get_worktree_stats(&wt.path);
                        let last_commit_time = get_last_commit_time(&wt.path);
                        let branch_info = BranchInfo {
                            name: wt.branch.clone(),
                            path: wt.path.to_string_lossy().to_string(),
                            file_count,
                            insertions,
                            deletions,
                            last_activity: last_commit_time
                                .map(format_relative_time)
                                .unwrap_or_else(|| "unknown".to_string()),
                        };
                        (branch_info, last_commit_time)
                    })
                    .collect();

            // Sort: main/master first, then by most recent commit time
            branches_with_time.sort_by(|(a, time_a), (b, time_b)| {
                let a_is_main = a.name == "main" || a.name == "master";
                let b_is_main = b.name == "main" || b.name == "master";

                match (a_is_main, b_is_main) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => {
                        // Both are main/master or neither - sort by time (most recent first)
                        match (time_a, time_b) {
                            (Some(ta), Some(tb)) => tb.cmp(ta), // Reverse order for most recent first
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    }
                }
            });

            // Extract just the BranchInfo
            let branches: Vec<BranchInfo> =
                branches_with_time.into_iter().map(|(b, _)| b).collect();

            // Get the most recent activity (from the first non-main branch or main itself)
            let most_recent_activity = branches
                .iter()
                .filter_map(|b| {
                    if b.last_activity == "unknown" {
                        None
                    } else {
                        Some(b.last_activity.clone())
                    }
                })
                .next()
                .unwrap_or_else(|| {
                    get_last_commit_time(path)
                        .map(format_relative_time)
                        .unwrap_or_else(|| "unknown".to_string())
                });

            // Count errors/warnings
            let file_count = get_codebase_file_count(path);
            let warnings = if file_count == 0 { 1 } else { 0 }; // Warn if not indexed

            Some(ProjectInfo {
                path: path.to_string_lossy().to_string(),
                name: config.name.clone(),
                branches,
                warnings,
                last_activity: most_recent_activity,
            })
        })
        .collect();

    Ok(projects)
}
