//! Git helpers used by the processor (status detection, diffs, binary filtering).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use golish_core::utils::truncate_str;

/// Represents a git change with file path and optional diff
#[derive(Debug)]
pub(super) struct GitChange {
    pub(super) path: String,
    pub(super) diff: String,
}

/// Get modified files and their diffs using git
pub(super) async fn get_git_changes(cwd: &Path) -> Vec<GitChange> {
    let is_git = tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git {
        tracing::debug!("[sidecar] Not a git repository, skipping git diff");
        return vec![];
    }

    let output = match tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("[sidecar] Failed to run git status: {}", e);
            return vec![];
        }
    };

    if !output.status.success() {
        return vec![];
    }

    let status_output = String::from_utf8_lossy(&output.stdout);
    let mut changes = Vec::new();

    for line in status_output.lines() {
        if line.len() < 4 {
            continue;
        }

        let status = &line[0..2];
        let path = line[3..].trim().to_string();

        if is_binary_or_artifact(&path) {
            tracing::debug!("[sidecar] Skipping binary/artifact: {}", path);
            continue;
        }

        if status.contains('D') {
            changes.push(GitChange {
                path: path.clone(),
                diff: "(deleted)".to_string(),
            });
            continue;
        }

        if is_git_binary(cwd, &path).await {
            tracing::debug!("[sidecar] Skipping git-detected binary: {}", path);
            continue;
        }

        let diff = get_file_diff(cwd, &path).await;
        changes.push(GitChange { path, diff });
    }

    changes
}

/// Check if a file is likely a binary or build artifact based on path/extension
fn is_binary_or_artifact(path: &str) -> bool {
    let path_lower = path.to_lowercase();

    let binary_extensions = [
        ".exe", ".dll", ".so", ".dylib", ".a", ".o", ".obj", ".pyc", ".pyo", ".class", ".jar",
        ".war", ".zip", ".tar", ".gz", ".bz2", ".xz", ".7z", ".rar", ".png", ".jpg", ".jpeg",
        ".gif", ".bmp", ".ico", ".svg", ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".wasm", ".node",
    ];

    let artifact_dirs = [
        "node_modules/",
        "target/",
        "build/",
        "dist/",
        "out/",
        ".git/",
        "__pycache__/",
        ".pytest_cache/",
        ".mypy_cache/",
        "vendor/",
        "bin/",
        "obj/",
    ];

    for ext in &binary_extensions {
        if path_lower.ends_with(ext) {
            return true;
        }
    }

    for dir in &artifact_dirs {
        if path_lower.contains(dir) {
            return true;
        }
    }

    let filename = path.rsplit('/').next().unwrap_or(path);
    if !filename.contains('.') {
        let allowed_extensionless = [
            "Makefile",
            "Dockerfile",
            "Jenkinsfile",
            "Vagrantfile",
            "README",
            "LICENSE",
            "CHANGELOG",
            "AUTHORS",
            "CONTRIBUTORS",
            "Gemfile",
            "Rakefile",
            "Procfile",
            "Brewfile",
            ".gitignore",
            ".gitattributes",
            ".dockerignore",
            ".editorconfig",
        ];
        if !allowed_extensionless
            .iter()
            .any(|&f| filename == f || filename.starts_with('.'))
        {
            return true;
        }
    }

    false
}

/// Check if git considers a file binary using diff --numstat
async fn is_git_binary(cwd: &Path, file_path: &str) -> bool {
    let output = tokio::process::Command::new("git")
        .args(["diff", "--numstat", "HEAD", "--", file_path])
        .current_dir(cwd)
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.starts_with("-\t-\t")
        }
        _ => false,
    }
}

/// Get the diff for a specific file
async fn get_file_diff(cwd: &Path, file_path: &str) -> String {
    let output = tokio::process::Command::new("git")
        .args(["diff", "HEAD", "--", file_path])
        .current_dir(cwd)
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let diff = String::from_utf8_lossy(&o.stdout).to_string();
            if diff.is_empty() {
                "(new file)".to_string()
            } else if diff.chars().count() > 2000 {
                format!(
                    "{}...\n(truncated, {} more lines)",
                    truncate_str(&diff, 2000),
                    diff.lines().count().saturating_sub(50)
                )
            } else {
                diff
            }
        }
        _ => "(unable to get diff)".to_string(),
    }
}

/// Get git diff for a list of files (used for commit-message synthesis)
pub(super) async fn get_diff_for_files(git_root: &PathBuf, files: &[PathBuf]) -> Result<String> {
    use tokio::process::Command;

    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("HEAD").arg("--").current_dir(git_root);

    for file in files {
        cmd.arg(file);
    }

    let output = cmd.output().await.context("Failed to run git diff")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
