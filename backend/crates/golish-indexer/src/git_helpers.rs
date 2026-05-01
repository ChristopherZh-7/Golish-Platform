//! Git utility functions: stats, worktree listing, relative time formatting.
//!
//! Used by the home view and recent directories commands.

use std::path::Path;

use crate::types::BranchInfo;

pub fn format_relative_time(datetime: chrono::DateTime<chrono::Utc>) -> String {
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

pub fn get_last_commit_time(path: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
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
    chrono::DateTime::parse_from_rfc3339(date_str.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Returns `(branch, insertions, deletions, file_count)`.
pub fn get_git_stats(path: &Path) -> Option<(String, i32, i32, u32)> {
    use std::process::Command;

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

    let (insertions, deletions, file_count) = parse_diff_stats(path);

    Some((branch, insertions, deletions, file_count))
}

/// Returns `(insertions, deletions, file_count)` from `git diff --stat HEAD`.
pub fn get_worktree_stats(worktree_path: &Path) -> (i32, i32, u32) {
    parse_diff_stats(worktree_path)
}

fn parse_diff_stats(path: &Path) -> (i32, i32, u32) {
    use std::process::Command;

    let diff_output = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(path)
        .output();

    let mut insertions = 0i32;
    let mut deletions = 0i32;
    let mut file_count = 0u32;

    if let Ok(output) = diff_output {
        if output.status.success() {
            let diff_str = String::from_utf8_lossy(&output.stdout);
            for line in diff_str.lines() {
                if line.contains("changed") {
                    file_count = diff_str.lines().filter(|l| l.contains("|")).count() as u32;

                    if let Some(ins_match) = line.find("insertion") {
                        let before_ins = &line[..ins_match];
                        if let Some(num_str) = before_ins.split(',').next_back() {
                            insertions = num_str.trim().parse().unwrap_or(0);
                        }
                    }

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

#[derive(Debug, Clone)]
pub struct GitWorktree {
    pub path: std::path::PathBuf,
    pub branch: String,
}

pub fn get_git_worktrees(repo_path: &Path) -> Vec<GitWorktree> {
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
    let mut current_path: Option<std::path::PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut is_bare = false;

    for line in stdout.lines() {
        if let Some(path_str) = line.strip_prefix("worktree ") {
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                if !is_bare {
                    worktrees.push(GitWorktree { path, branch });
                }
            }
            current_path = Some(std::path::PathBuf::from(path_str));
            current_branch = None;
            is_bare = false;
        } else if line == "bare" {
            is_bare = true;
            current_path = None;
        } else if let Some(branch_ref) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch_ref.to_string());
        } else if line == "detached" {
            current_branch = Some("detached".to_string());
        } else if line.is_empty() {
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                if !is_bare {
                    worktrees.push(GitWorktree { path, branch });
                }
            }
            is_bare = false;
        }
    }

    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        if !is_bare {
            worktrees.push(GitWorktree { path, branch });
        }
    }

    worktrees
}

/// Build `BranchInfo` entries for all worktrees in a repository.
pub fn build_branch_infos(repo_path: &Path) -> Vec<(BranchInfo, Option<chrono::DateTime<chrono::Utc>>)> {
    get_git_worktrees(repo_path)
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
        .collect()
}
