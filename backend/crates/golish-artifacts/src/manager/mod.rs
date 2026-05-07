pub(crate) mod diff;
mod types;

pub use types::{ArtifactFile, ArtifactMeta};

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::generators::{generate_claude_md_update, generate_readme_update};
use crate::synthesis::{
    synthesize_claude_md, synthesize_readme, ArtifactSynthesisConfig, ArtifactSynthesisInput,
};
use diff::{continue_or_error, generate_simple_diff};

/// Manages artifacts for a session
pub struct ArtifactManager {
    session_dir: PathBuf,
}

impl ArtifactManager {
    const ARTIFACTS_DIR: &'static str = "artifacts";
    const PENDING_DIR: &'static str = "pending";
    const APPLIED_DIR: &'static str = "applied";

    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    pub fn pending_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::ARTIFACTS_DIR)
            .join(Self::PENDING_DIR)
    }

    pub fn applied_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::ARTIFACTS_DIR)
            .join(Self::APPLIED_DIR)
    }

    pub async fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.pending_dir())
            .await
            .context("Failed to create pending artifacts directory")?;
        fs::create_dir_all(self.applied_dir())
            .await
            .context("Failed to create applied artifacts directory")?;
        Ok(())
    }

    pub async fn create_artifact(&self, artifact: &ArtifactFile) -> Result<PathBuf> {
        self.ensure_dirs().await?;

        let path = self.pending_dir().join(&artifact.filename);
        let content = artifact.to_file_content();

        fs::write(&path, &content)
            .await
            .context("Failed to write artifact file")?;

        tracing::info!("Created pending artifact: {}", artifact.filename);
        Ok(path)
    }

    pub async fn list_pending(&self) -> Result<Vec<ArtifactFile>> {
        self.list_artifacts_in_dir(&self.pending_dir()).await
    }

    pub async fn list_applied(&self) -> Result<Vec<ArtifactFile>> {
        self.list_artifacts_in_dir(&self.applied_dir()).await
    }

    async fn list_artifacts_in_dir(&self, dir: &Path) -> Result<Vec<ArtifactFile>> {
        let mut artifacts = Vec::new();

        if !dir.exists() {
            return Ok(artifacts);
        }

        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                match self.load_artifact(&path).await {
                    Ok(artifact) => artifacts.push(artifact),
                    Err(e) => {
                        tracing::warn!("Failed to load artifact {:?}: {}", path, e);
                    }
                }
            }
        }

        artifacts.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(artifacts)
    }

    async fn load_artifact(&self, path: &Path) -> Result<ArtifactFile> {
        let content = fs::read_to_string(path)
            .await
            .context("Failed to read artifact file")?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        ArtifactFile::from_file_content(&filename, &content)
    }

    pub async fn get_pending(&self, filename: &str) -> Result<Option<ArtifactFile>> {
        let path = self.pending_dir().join(filename);
        if !path.exists() {
            return Ok(None);
        }
        self.load_artifact(&path).await.map(Some)
    }

    pub async fn discard_artifact(&self, filename: &str) -> Result<bool> {
        let path = self.pending_dir().join(filename);
        if !path.exists() {
            return Ok(false);
        }

        fs::remove_file(&path)
            .await
            .context("Failed to remove artifact file")?;

        tracing::info!("Discarded artifact: {}", filename);
        Ok(true)
    }

    pub async fn apply_artifact(&self, filename: &str, git_root: &Path) -> Result<PathBuf> {
        let artifact = self
            .get_pending(filename)
            .await?
            .context(format!("Artifact {} not found in pending", filename))?;

        let target_path = &artifact.meta.target;

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .await
                .context("Failed to create target directory")?;
        }

        fs::write(target_path, &artifact.content)
            .await
            .context("Failed to write to target file")?;

        let relative_path = target_path
            .strip_prefix(git_root)
            .unwrap_or(target_path)
            .to_string_lossy()
            .to_string();

        tokio::process::Command::new("git")
            .args(["add", &relative_path])
            .current_dir(git_root)
            .output()
            .await
            .context("Failed to git add artifact")?;

        let pending_path = self.pending_dir().join(filename);
        let applied_path = self.applied_dir().join(filename);

        self.ensure_dirs().await?;
        fs::rename(&pending_path, &applied_path)
            .await
            .context("Failed to move artifact to applied")?;

        tracing::info!("Applied artifact {} to {}", filename, target_path.display());
        Ok(target_path.clone())
    }

    pub async fn apply_all_artifacts(&self, git_root: &Path) -> Result<Vec<(String, PathBuf)>> {
        let pending = self.list_pending().await?;
        let mut results = Vec::new();

        for artifact in pending {
            match self.apply_artifact(&artifact.filename, git_root).await {
                Ok(path) => {
                    results.push((artifact.filename.clone(), path));
                }
                Err(e) => {
                    bail!(
                        "Failed to apply artifact {}: {}. Applied {} artifacts before failure.",
                        artifact.filename,
                        e,
                        results.len()
                    );
                }
            }
        }

        Ok(results)
    }

    pub async fn preview_artifact(&self, filename: &str) -> Result<String> {
        let artifact = self
            .get_pending(filename)
            .await?
            .context(format!("Artifact {} not found in pending", filename))?;

        let target_path = &artifact.meta.target;

        let current_content = if target_path.exists() {
            fs::read_to_string(target_path).await.unwrap_or_default()
        } else {
            String::new()
        };

        Ok(generate_simple_diff(&current_content, &artifact.content))
    }

    /// Regenerate artifacts based on applied patches (L2 -> L3 cascade)
    pub async fn regenerate_from_patches(
        &self,
        git_root: &Path,
        patch_subjects: &[String],
        session_context: &str,
    ) -> Result<Vec<PathBuf>> {
        let config = ArtifactSynthesisConfig::default();
        self.regenerate_from_patches_with_config(git_root, patch_subjects, session_context, &config)
            .await
    }

    /// Regenerate artifacts based on applied patches with explicit config (L2 -> L3 cascade)
    pub async fn regenerate_from_patches_with_config(
        &self,
        git_root: &Path,
        patch_subjects: &[String],
        session_context: &str,
        config: &ArtifactSynthesisConfig,
    ) -> Result<Vec<PathBuf>> {
        self.ensure_dirs().await?;

        let mut created = Vec::new();

        let input = ArtifactSynthesisInput::new(
            String::new(),
            patch_subjects.to_vec(),
            session_context.to_string(),
        );

        let readme_path = git_root.join("README.md");
        if readme_path.exists() {
            let current_readme = fs::read_to_string(&readme_path).await.unwrap_or_default();

            let readme_input = ArtifactSynthesisInput::new(
                current_readme.clone(),
                input.patches_summary.clone(),
                input.session_context.clone(),
            );

            let updated_readme = match synthesize_readme(config, &readme_input).await {
                Ok(result) => {
                    tracing::debug!("README synthesis using {} backend", result.backend);
                    result.content
                }
                Err(e) if config.uses_llm() => {
                    tracing::warn!(
                        "LLM synthesis failed for README.md, falling back to template: {}",
                        e
                    );
                    generate_readme_update(&current_readme, session_context, patch_subjects)
                }
                Err(e) => {
                    tracing::warn!("Template synthesis failed for README.md: {}", e);
                    continue_or_error(e)?
                }
            };

            if updated_readme != current_readme {
                let patch_ids: Vec<u32> = (1..=patch_subjects.len() as u32).collect();
                let meta = ArtifactMeta::with_patches(
                    readme_path.clone(),
                    format!(
                        "Updated based on {} applied patches ({})",
                        patch_subjects.len(),
                        config.backend
                    ),
                    patch_ids,
                );

                let artifact = ArtifactFile::new("README.md".to_string(), meta, updated_readme);
                let path = self.create_artifact(&artifact).await?;
                created.push(path);
            }
        }

        let claude_md_path = git_root.join("CLAUDE.md");
        if claude_md_path.exists() {
            let current_claude_md = fs::read_to_string(&claude_md_path)
                .await
                .unwrap_or_default();

            let claude_input = ArtifactSynthesisInput::new(
                current_claude_md.clone(),
                input.patches_summary.clone(),
                input.session_context.clone(),
            );

            let updated_claude_md = match synthesize_claude_md(config, &claude_input).await {
                Ok(result) => {
                    tracing::debug!("CLAUDE.md synthesis using {} backend", result.backend);
                    result.content
                }
                Err(e) if config.uses_llm() => {
                    tracing::warn!(
                        "LLM synthesis failed for CLAUDE.md, falling back to template: {}",
                        e
                    );
                    generate_claude_md_update(&current_claude_md, session_context, patch_subjects)
                }
                Err(e) => {
                    tracing::warn!("Template synthesis failed for CLAUDE.md: {}", e);
                    continue_or_error(e)?
                }
            };

            if updated_claude_md != current_claude_md {
                let patch_ids: Vec<u32> = (1..=patch_subjects.len() as u32).collect();
                let meta = ArtifactMeta::with_patches(
                    claude_md_path.clone(),
                    format!(
                        "Updated conventions from {} patches ({})",
                        patch_subjects.len(),
                        config.backend
                    ),
                    patch_ids,
                );

                let artifact = ArtifactFile::new("CLAUDE.md".to_string(), meta, updated_claude_md);
                let path = self.create_artifact(&artifact).await?;
                created.push(path);
            }
        }

        if !created.is_empty() {
            tracing::info!(
                "Regenerated {} artifacts from {} patches using {} backend",
                created.len(),
                patch_subjects.len(),
                config.backend
            );
        }

        Ok(created)
    }
}
