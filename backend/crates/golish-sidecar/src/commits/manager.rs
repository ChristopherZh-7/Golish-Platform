//! Patch storage manager: create, list, apply, discard staged patches.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;

use super::diff::{generate_diff_for_single_file, generate_diff_from_strings};
use super::format::{extract_diff_from_patch, extract_message_from_patch, format_patch_content};
use super::types::{BoundaryReason, PatchMeta, StagedPatch};

/// Manages patches for a session
pub struct PatchManager {
    /// Session directory
    session_dir: PathBuf,
}

impl PatchManager {
    const PATCHES_DIR: &'static str = "patches";
    const STAGED_DIR: &'static str = "staged";
    const APPLIED_DIR: &'static str = "applied";
    const BASELINES_DIR: &'static str = "baselines";

    /// Create a new patch manager for a session
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    fn staged_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::PATCHES_DIR)
            .join(Self::STAGED_DIR)
    }

    fn applied_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::PATCHES_DIR)
            .join(Self::APPLIED_DIR)
    }

    fn baselines_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::PATCHES_DIR)
            .join(Self::BASELINES_DIR)
    }

    /// Ensure patch directories exist
    pub async fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.staged_dir())
            .await
            .context("Failed to create staged patches directory")?;
        fs::create_dir_all(self.applied_dir())
            .await
            .context("Failed to create applied patches directory")?;
        fs::create_dir_all(self.baselines_dir())
            .await
            .context("Failed to create baselines directory")?;
        Ok(())
    }

    fn baseline_path(&self, file_path: &Path) -> PathBuf {
        let hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            file_path.to_string_lossy().hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        };
        self.baselines_dir().join(hash)
    }

    async fn load_baseline(&self, file_path: &Path) -> Option<String> {
        let baseline_path = self.baseline_path(file_path);
        fs::read_to_string(&baseline_path).await.ok()
    }

    async fn save_baseline(&self, file_path: &Path, content: &str) -> Result<()> {
        let baseline_path = self.baseline_path(file_path);
        fs::write(&baseline_path, content)
            .await
            .context("Failed to save baseline")?;
        Ok(())
    }

    /// Save current file content as baseline for incremental diffs
    pub async fn save_file_baselines(&self, git_root: &Path, files: &[PathBuf]) -> Result<()> {
        self.ensure_dirs().await?;
        for file in files {
            let full_path = git_root.join(file);
            if let Ok(content) = fs::read_to_string(&full_path).await {
                self.save_baseline(file, &content).await?;
            }
        }
        Ok(())
    }

    /// Get the next patch ID
    pub async fn next_id(&self) -> Result<u32> {
        let staged = self.list_staged().await.unwrap_or_default();
        let applied = self.list_applied().await.unwrap_or_default();

        let max_staged = staged.iter().map(|p| p.meta.id).max().unwrap_or(0);
        let max_applied = applied.iter().map(|p| p.meta.id).max().unwrap_or(0);

        Ok(max_staged.max(max_applied) + 1)
    }

    /// Create a patch from file changes (without git staging)
    ///
    /// Uses incremental diffs: if a previous patch exists for the same files,
    /// the new patch will only contain changes since the previous patch.
    /// This allows patches to be applied sequentially without conflicts.
    pub async fn create_patch_from_changes(
        &self,
        git_root: &Path,
        files: &[PathBuf],
        message: &str,
        boundary_reason: BoundaryReason,
    ) -> Result<StagedPatch> {
        self.ensure_dirs().await?;

        let id = self.next_id().await?;

        let diff_content = self.generate_incremental_diff(git_root, files).await?;

        let patch_content = format_patch_content(message, &diff_content);

        let subject = message.lines().next().unwrap_or("changes").to_string();
        let file_strings: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();

        let meta = PatchMeta {
            id,
            created_at: Utc::now(),
            boundary_reason,
            applied_sha: None,
        };

        let patch = StagedPatch {
            meta: meta.clone(),
            subject,
            message: message.to_string(),
            files: file_strings,
        };

        let patch_path = self.staged_dir().join(patch.filename());
        fs::write(&patch_path, &patch_content)
            .await
            .context("Failed to write patch file")?;

        let meta_path = self.staged_dir().join(patch.meta_filename());
        let meta_content = toml::to_string_pretty(&meta)?;
        fs::write(&meta_path, &meta_content)
            .await
            .context("Failed to write patch metadata")?;

        self.save_file_baselines(git_root, files).await?;

        tracing::info!("Created staged patch: {}", patch.filename());
        Ok(patch)
    }

    /// Generate incremental diff for files using baselines
    ///
    /// For each file:
    /// - If a baseline exists, generate diff from baseline to current
    /// - If no baseline exists, generate diff from HEAD (or /dev/null for new files)
    async fn generate_incremental_diff(
        &self,
        git_root: &Path,
        files: &[PathBuf],
    ) -> Result<String> {
        let mut all_diffs = String::new();

        for file in files {
            let full_path = git_root.join(file);
            let file_str = file.to_string_lossy();

            let current_content = match fs::read_to_string(&full_path).await {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(baseline_content) = self.load_baseline(file).await {
                if baseline_content != current_content {
                    let diff =
                        generate_diff_from_strings(&file_str, &baseline_content, &current_content);
                    if !diff.is_empty() {
                        all_diffs.push_str(&diff);
                        if !all_diffs.ends_with('\n') {
                            all_diffs.push('\n');
                        }
                    }
                }
            } else {
                let diff = generate_diff_for_single_file(git_root, file).await?;
                if !diff.is_empty() {
                    all_diffs.push_str(&diff);
                    if !all_diffs.ends_with('\n') {
                        all_diffs.push('\n');
                    }
                }
            }
        }

        Ok(all_diffs)
    }

    /// List all staged patches
    pub async fn list_staged(&self) -> Result<Vec<StagedPatch>> {
        self.list_patches_in_dir(&self.staged_dir()).await
    }

    /// List all applied patches
    pub async fn list_applied(&self) -> Result<Vec<StagedPatch>> {
        self.list_patches_in_dir(&self.applied_dir()).await
    }

    async fn list_patches_in_dir(&self, dir: &Path) -> Result<Vec<StagedPatch>> {
        let mut patches = Vec::new();

        if !dir.exists() {
            return Ok(patches);
        }

        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "patch") {
                match self.load_patch(&path).await {
                    Ok(patch) => patches.push(patch),
                    Err(e) => {
                        tracing::warn!("Failed to load patch {:?}: {}", path, e);
                    }
                }
            }
        }

        patches.sort_by_key(|p| p.meta.id);
        Ok(patches)
    }

    async fn load_patch(&self, patch_path: &Path) -> Result<StagedPatch> {
        let patch_content = fs::read_to_string(patch_path)
            .await
            .context("Failed to read patch file")?;

        let meta_path = patch_path.with_extension("meta.toml");
        let meta_path = if meta_path.exists() {
            meta_path
        } else {
            let stem = patch_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let id_part = stem.split('-').next().unwrap_or("0000");
            patch_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("{}.meta.toml", id_part))
        };

        let meta: PatchMeta = if meta_path.exists() {
            let meta_content = fs::read_to_string(&meta_path).await?;
            toml::from_str(&meta_content)?
        } else {
            let id = patch_path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            PatchMeta {
                id,
                created_at: Utc::now(),
                boundary_reason: BoundaryReason::UserRequest,
                applied_sha: None,
            }
        };

        let subject =
            StagedPatch::parse_subject(&patch_content).unwrap_or_else(|| "unknown".to_string());
        let files = StagedPatch::parse_files(&patch_content);

        let message = extract_message_from_patch(&patch_content);

        Ok(StagedPatch {
            meta,
            subject,
            message,
            files,
        })
    }

    /// Get a specific staged patch by ID
    pub async fn get_staged(&self, id: u32) -> Result<Option<StagedPatch>> {
        let patches = self.list_staged().await?;
        Ok(patches.into_iter().find(|p| p.meta.id == id))
    }

    /// Discard a staged patch
    pub async fn discard_patch(&self, id: u32) -> Result<bool> {
        let patches = self.list_staged().await?;
        if let Some(patch) = patches.into_iter().find(|p| p.meta.id == id) {
            let patch_path = self.staged_dir().join(patch.filename());
            let meta_path = self.staged_dir().join(patch.meta_filename());

            fs::remove_file(&patch_path).await.ok();
            fs::remove_file(&meta_path).await.ok();

            tracing::info!("Discarded patch: {}", patch.filename());
            return Ok(true);
        }
        Ok(false)
    }

    /// Update the commit message for a staged patch
    ///
    /// This rewrites the patch file with the new message while preserving the diff.
    pub async fn update_patch_message(&self, id: u32, new_message: &str) -> Result<StagedPatch> {
        let patch = self
            .get_staged(id)
            .await?
            .context(format!("Patch {} not found in staged", id))?;

        let old_patch_path = self.staged_dir().join(patch.filename());

        let old_content = fs::read_to_string(&old_patch_path)
            .await
            .context("Failed to read patch file")?;

        let diff = extract_diff_from_patch(&old_content);

        let new_patch_content = format_patch_content(new_message, &diff);

        let new_subject = new_message.lines().next().unwrap_or("changes").to_string();
        let updated_patch = StagedPatch {
            meta: patch.meta.clone(),
            subject: new_subject.clone(),
            message: new_message.to_string(),
            files: patch.files.clone(),
        };

        let new_patch_path = self.staged_dir().join(updated_patch.filename());

        fs::write(&new_patch_path, &new_patch_content)
            .await
            .context("Failed to write updated patch file")?;

        if old_patch_path != new_patch_path && old_patch_path.exists() {
            fs::remove_file(&old_patch_path).await.ok();
        }

        tracing::info!("Updated patch {} message: {}", id, new_subject);
        Ok(updated_patch)
    }

    /// Get the raw diff content from a staged patch
    pub async fn get_patch_diff(&self, id: u32) -> Result<String> {
        let patch = self
            .get_staged(id)
            .await?
            .context(format!("Patch {} not found in staged", id))?;

        let patch_path = self.staged_dir().join(patch.filename());
        let content = fs::read_to_string(&patch_path)
            .await
            .context("Failed to read patch file")?;

        Ok(extract_diff_from_patch(&content))
    }

    /// Apply a staged patch using git am
    pub async fn apply_patch(&self, id: u32, git_root: &Path) -> Result<String> {
        let patch = self
            .get_staged(id)
            .await?
            .context(format!("Patch {} not found in staged", id))?;

        let patch_path = self.staged_dir().join(patch.filename());

        let output = Command::new("git")
            .args(["am", "--3way"])
            .arg(&patch_path)
            .current_dir(git_root)
            .output()
            .await
            .context("Failed to run git am")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = Command::new("git")
                .args(["am", "--abort"])
                .current_dir(git_root)
                .output()
                .await;
            bail!("git am failed: {}", stderr);
        }

        let sha_output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(git_root)
            .output()
            .await
            .context("Failed to get commit SHA")?;

        let sha = String::from_utf8_lossy(&sha_output.stdout)
            .trim()
            .to_string();

        self.mark_applied(id, &sha).await?;

        tracing::info!("Applied patch {} with SHA {}", id, sha);
        Ok(sha)
    }

    async fn mark_applied(&self, id: u32, sha: &str) -> Result<()> {
        let patches = self.list_staged().await?;
        if let Some(mut patch) = patches.into_iter().find(|p| p.meta.id == id) {
            patch.meta.applied_sha = Some(sha.to_string());

            let staged_patch = self.staged_dir().join(patch.filename());
            let applied_patch = self.applied_dir().join(patch.filename());
            fs::rename(&staged_patch, &applied_patch).await?;

            let staged_meta = self.staged_dir().join(patch.meta_filename());
            let applied_meta = self.applied_dir().join(patch.meta_filename());
            let meta_content = toml::to_string_pretty(&patch.meta)?;
            fs::write(&applied_meta, &meta_content).await?;
            fs::remove_file(&staged_meta).await.ok();
        }
        Ok(())
    }

    /// Apply all staged patches in order
    pub async fn apply_all_patches(&self, git_root: &Path) -> Result<Vec<(u32, String)>> {
        let staged = self.list_staged().await?;
        let mut results = Vec::new();

        for patch in staged {
            match self.apply_patch(patch.meta.id, git_root).await {
                Ok(sha) => {
                    results.push((patch.meta.id, sha));
                }
                Err(e) => {
                    bail!(
                        "Failed to apply patch {}: {}. Applied {} patches before failure.",
                        patch.meta.id,
                        e,
                        results.len()
                    );
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_patch_manager_lifecycle() {
        let temp = TempDir::new().unwrap();
        let manager = PatchManager::new(temp.path().to_path_buf());

        manager.ensure_dirs().await.unwrap();
        assert!(temp.path().join("patches/staged").exists());
        assert!(temp.path().join("patches/applied").exists());

        let id = manager.next_id().await.unwrap();
        assert_eq!(id, 1);
    }
}
