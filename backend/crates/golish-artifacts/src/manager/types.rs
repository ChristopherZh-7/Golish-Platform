//! Data types for artifact files and metadata.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Metadata for an artifact file (stored in HTML comment header)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactMeta {
    pub target: PathBuf,
    pub created_at: DateTime<Utc>,
    pub reason: String,
    #[serde(default)]
    pub based_on_patches: Vec<u32>,
}

impl ArtifactMeta {
    #[cfg(test)]
    pub fn new(target: PathBuf, reason: String) -> Self {
        Self {
            target,
            created_at: Utc::now(),
            reason,
            based_on_patches: Vec::new(),
        }
    }

    pub fn with_patches(target: PathBuf, reason: String, patches: Vec<u32>) -> Self {
        Self {
            target,
            created_at: Utc::now(),
            reason,
            based_on_patches: patches,
        }
    }

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

    pub fn from_header(header: &str) -> Result<Self> {
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
    pub meta: ArtifactMeta,
    pub filename: String,
    pub content: String,
}

impl ArtifactFile {
    pub fn new(filename: String, meta: ArtifactMeta, content: String) -> Self {
        Self {
            meta,
            filename,
            content,
        }
    }

    pub fn to_file_content(&self) -> String {
        format!("{}\n\n{}", self.meta.to_header(), self.content)
    }

    pub fn from_file_content(filename: &str, content: &str) -> Result<Self> {
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
