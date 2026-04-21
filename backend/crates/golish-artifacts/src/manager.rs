use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::synthesis::{ArtifactSynthesisConfig, ArtifactSynthesisInput, synthesize_readme, synthesize_claude_md};
use crate::generators::{generate_readme_update, generate_claude_md_update};

/// Metadata for an artifact file (stored in HTML comment header)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactMeta {
    /// Target file path in the project (e.g., /Users/xlyk/Code/golish/README.md)
    pub target: PathBuf,
    /// When this artifact was created
    pub created_at: DateTime<Utc>,
    /// Reason for the artifact (what changed)
    pub reason: String,
    /// Patch IDs this artifact is based on (if any)
    #[serde(default)]
    pub based_on_patches: Vec<u32>,
}

impl ArtifactMeta {
    /// Create new artifact metadata
    #[cfg(test)]
    pub fn new(target: PathBuf, reason: String) -> Self {
        Self {
            target,
            created_at: Utc::now(),
            reason,
            based_on_patches: Vec::new(),
        }
    }

    /// Create metadata with patch references
    pub fn with_patches(target: PathBuf, reason: String, patches: Vec<u32>) -> Self {
        Self {
            target,
            created_at: Utc::now(),
            reason,
            based_on_patches: patches,
        }
    }

    /// Format metadata as HTML comment header
    pub fn to_header(&self) -> String {
        let date_str = self.created_at.format("%Y-%m-%d %H:%M").to_string();
        let patches_str = if self.based_on_patches.is_empty() {
            String::new()
        } else {
            let patches: Vec<String> = self
                .based_on_patches
                .iter()
                .map(|id| format!("{:04}", id))
                .collect();
            format!("\nBased on patches: {}", patches.join(", "))
        };

        format!(
            "<!--\nTarget: {}\nCreated: {}\nReason: {}{}\n-->",
            self.target.display(),
            date_str,
            self.reason,
            patches_str
        )
    }

    /// Parse metadata from HTML comment header
    pub fn from_header(header: &str) -> Result<Self> {
        // Extract content between <!-- and -->
        let content = header
            .strip_prefix("<!--")
            .and_then(|s| s.strip_suffix("-->"))
            .map(|s| s.trim())
            .context("Invalid header format: missing <!-- --> delimiters")?;

        let mut target: Option<PathBuf> = None;
        let mut created_at: Option<DateTime<Utc>> = None;
        let mut reason: Option<String> = None;
        let mut based_on_patches: Vec<u32> = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(value) = line.strip_prefix("Target:") {
                target = Some(PathBuf::from(value.trim()));
            } else if let Some(value) = line.strip_prefix("Created:") {
                let date_str = value.trim();
                // Parse "YYYY-MM-DD HH:MM" format
                let naive = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M")
                    .context("Invalid date format, expected YYYY-MM-DD HH:MM")?;
                created_at = Some(DateTime::from_naive_utc_and_offset(naive, Utc));
            } else if let Some(value) = line.strip_prefix("Reason:") {
                reason = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("Based on patches:") {
                based_on_patches = value
                    .split(',')
                    .filter_map(|s| s.trim().parse::<u32>().ok())
                    .collect();
            }
        }

        Ok(Self {
            target: target.context("Missing Target field in header")?,
            created_at: created_at.context("Missing Created field in header")?,
            reason: reason.context("Missing Reason field in header")?,
            based_on_patches,
        })
    }
}

/// An artifact file with its metadata and content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactFile {
    /// Artifact metadata
    pub meta: ArtifactMeta,
    /// The artifact filename (e.g., "README.md", "CLAUDE.md")
    pub filename: String,
    /// The artifact content (without the metadata header)
    pub content: String,
}

impl ArtifactFile {
    /// Create a new artifact file
    pub fn new(filename: String, meta: ArtifactMeta, content: String) -> Self {
        Self {
            meta,
            filename,
            content,
        }
    }

    /// Format the full file content with metadata header
    pub fn to_file_content(&self) -> String {
        format!("{}\n\n{}", self.meta.to_header(), self.content)
    }

    /// Parse an artifact file from its content
    pub fn from_file_content(filename: &str, content: &str) -> Result<Self> {
        // Find the header end
        let header_end = content
            .find("-->")
            .context("Missing header end delimiter (-->)")?;

        let header = &content[..header_end + 3];
        let body = content[header_end + 3..].trim_start();

        let meta = ArtifactMeta::from_header(header)?;

        Ok(Self {
            meta,
            filename: filename.to_string(),
            content: body.to_string(),
        })
    }
}

/// Manages artifacts for a session
pub struct ArtifactManager {
    /// Session directory
    session_dir: PathBuf,
}

impl ArtifactManager {
    /// Subdirectory names
    const ARTIFACTS_DIR: &'static str = "artifacts";
    const PENDING_DIR: &'static str = "pending";
    const APPLIED_DIR: &'static str = "applied";

    /// Create a new artifact manager for a session
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    /// Get the path to pending artifacts directory
    pub fn pending_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::ARTIFACTS_DIR)
            .join(Self::PENDING_DIR)
    }

    /// Get the path to applied artifacts directory
    pub fn applied_dir(&self) -> PathBuf {
        self.session_dir
            .join(Self::ARTIFACTS_DIR)
            .join(Self::APPLIED_DIR)
    }

    /// Ensure artifact directories exist
    pub async fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.pending_dir())
            .await
            .context("Failed to create pending artifacts directory")?;
        fs::create_dir_all(self.applied_dir())
            .await
            .context("Failed to create applied artifacts directory")?;
        Ok(())
    }

    /// Create a pending artifact
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

    /// List all pending artifacts
    pub async fn list_pending(&self) -> Result<Vec<ArtifactFile>> {
        self.list_artifacts_in_dir(&self.pending_dir()).await
    }

    /// List all applied artifacts
    pub async fn list_applied(&self) -> Result<Vec<ArtifactFile>> {
        self.list_artifacts_in_dir(&self.applied_dir()).await
    }

    /// List artifacts in a directory
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

        // Sort by filename
        artifacts.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(artifacts)
    }

    /// Load an artifact from a file
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

    /// Get a specific pending artifact by filename
    pub async fn get_pending(&self, filename: &str) -> Result<Option<ArtifactFile>> {
        let path = self.pending_dir().join(filename);
        if !path.exists() {
            return Ok(None);
        }
        self.load_artifact(&path).await.map(Some)
    }

    /// Discard a pending artifact
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

    /// Apply an artifact (copy to target, move to applied)
    pub async fn apply_artifact(&self, filename: &str, git_root: &Path) -> Result<PathBuf> {
        let artifact = self
            .get_pending(filename)
            .await?
            .context(format!("Artifact {} not found in pending", filename))?;

        // Copy content (without metadata header) to target
        let target_path = &artifact.meta.target;

        // Ensure target directory exists
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .await
                .context("Failed to create target directory")?;
        }

        // Write content to target
        fs::write(target_path, &artifact.content)
            .await
            .context("Failed to write to target file")?;

        // Git add the file
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

        // Move to applied directory
        let pending_path = self.pending_dir().join(filename);
        let applied_path = self.applied_dir().join(filename);

        self.ensure_dirs().await?;
        fs::rename(&pending_path, &applied_path)
            .await
            .context("Failed to move artifact to applied")?;

        tracing::info!("Applied artifact {} to {}", filename, target_path.display());
        Ok(target_path.clone())
    }

    /// Apply all pending artifacts
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

    /// Generate a diff between pending artifact and current target file
    pub async fn preview_artifact(&self, filename: &str) -> Result<String> {
        let artifact = self
            .get_pending(filename)
            .await?
            .context(format!("Artifact {} not found in pending", filename))?;

        let target_path = &artifact.meta.target;

        // Read current file content (if exists)
        let current_content = if target_path.exists() {
            fs::read_to_string(target_path).await.unwrap_or_default()
        } else {
            String::new()
        };

        // Generate a simple diff
        Ok(generate_simple_diff(&current_content, &artifact.content))
    }

    /// Regenerate artifacts based on applied patches (L2 -> L3 cascade)
    ///
    /// This method is called after patches are applied to update project documentation.
    /// Uses template-based generation by default. Call `regenerate_from_patches_with_config`
    /// to use LLM-based synthesis.
    pub async fn regenerate_from_patches(
        &self,
        git_root: &Path,
        patch_subjects: &[String],
        session_context: &str,
    ) -> Result<Vec<PathBuf>> {
        // Use default template-based config
        let config = ArtifactSynthesisConfig::default();
        self.regenerate_from_patches_with_config(git_root, patch_subjects, session_context, &config)
            .await
    }

    /// Regenerate artifacts based on applied patches with explicit config (L2 -> L3 cascade)
    ///
    /// This method is called after patches are applied to update project documentation.
    /// - `Template` backend uses rule-based generation (fast, no API calls)
    /// - Other backends use LLM synthesis (better quality, requires API access)
    ///
    /// If LLM synthesis fails, falls back to template-based generation.
    pub async fn regenerate_from_patches_with_config(
        &self,
        git_root: &Path,
        patch_subjects: &[String],
        session_context: &str,
        config: &ArtifactSynthesisConfig,
    ) -> Result<Vec<PathBuf>> {
        self.ensure_dirs().await?;

        let mut created = Vec::new();

        // Build synthesis input
        let input = ArtifactSynthesisInput::new(
            String::new(), // Will be set per-artifact
            patch_subjects.to_vec(),
            session_context.to_string(),
        );

        // Try to update README.md if it exists
        let readme_path = git_root.join("README.md");
        if readme_path.exists() {
            let current_readme = fs::read_to_string(&readme_path).await.unwrap_or_default();

            let readme_input = ArtifactSynthesisInput::new(
                current_readme.clone(),
                input.patches_summary.clone(),
                input.session_context.clone(),
            );

            // Try LLM synthesis, fall back to template on failure
            let updated_readme = match synthesize_readme(config, &readme_input).await {
                Ok(result) => {
                    tracing::debug!("README synthesis using {} backend", result.backend);
                    result.content
                }
                Err(e) if config.uses_llm() => {
                    // Fall back to template if LLM fails
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

            // Only create artifact if there are actual changes
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

        // Try to update CLAUDE.md if it exists
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

            // Try LLM synthesis, fall back to template on failure
            let updated_claude_md = match synthesize_claude_md(config, &claude_input).await {
                Ok(result) => {
                    tracing::debug!("CLAUDE.md synthesis using {} backend", result.backend);
                    result.content
                }
                Err(e) if config.uses_llm() => {
                    // Fall back to template if LLM fails
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

            // Only create artifact if there are actual changes
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

/// Helper to continue or propagate error (for template fallback)
fn continue_or_error<T>(e: anyhow::Error) -> Result<T> {
    Err(e)
}

/// Generate a simple unified diff between two strings
fn generate_simple_diff(old: &str, new: &str) -> String {
    use std::fmt::Write;

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff = String::new();
    let _ = writeln!(diff, "--- current");
    let _ = writeln!(diff, "+++ proposed");

    // Simple line-by-line comparison (not a real diff algorithm)
    let max_len = old_lines.len().max(new_lines.len());

    for i in 0..max_len {
        let old_line = old_lines.get(i).copied();
        let new_line = new_lines.get(i).copied();

        match (old_line, new_line) {
            (Some(o), Some(n)) if o == n => {
                let _ = writeln!(diff, " {}", o);
            }
            (Some(o), Some(n)) => {
                let _ = writeln!(diff, "-{}", o);
                let _ = writeln!(diff, "+{}", n);
            }
            (Some(o), None) => {
                let _ = writeln!(diff, "-{}", o);
            }
            (None, Some(n)) => {
                let _ = writeln!(diff, "+{}", n);
            }
            (None, None) => {}
        }
    }

    diff
}

