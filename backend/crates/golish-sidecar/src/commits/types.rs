//! Patch types and lightweight helpers shared across the `commits` module.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Metadata for a staged patch (stored alongside the .patch file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchMeta {
    /// Unique patch ID (sequence number)
    pub id: u32,
    /// When this patch was created
    pub created_at: DateTime<Utc>,
    /// Why this boundary was detected
    pub boundary_reason: BoundaryReason,
    /// Git SHA after applying (only set after applied)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_sha: Option<String>,
}

/// Reason for commit boundary detection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryReason {
    /// Agent signaled completion in reasoning
    CompletionSignal,
    /// User approved changes
    UserApproval,
    /// Session ended
    SessionEnd,
    /// Pause in activity
    ActivityPause,
    /// User explicitly requested commit
    UserRequest,
}

impl std::fmt::Display for BoundaryReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryReason::CompletionSignal => write!(f, "completion_signal"),
            BoundaryReason::UserApproval => write!(f, "user_approval"),
            BoundaryReason::SessionEnd => write!(f, "session_end"),
            BoundaryReason::ActivityPause => write!(f, "activity_pause"),
            BoundaryReason::UserRequest => write!(f, "user_request"),
        }
    }
}

/// A staged patch with its metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedPatch {
    /// Patch metadata
    pub meta: PatchMeta,
    /// Subject line (first line of commit message)
    pub subject: String,
    /// Full commit message body
    pub message: String,
    /// Files changed (parsed from diffstat)
    pub files: Vec<String>,
}

impl StagedPatch {
    /// Generate filename for this patch (e.g., "0001-feat-auth-add-jwt.patch")
    pub fn filename(&self) -> String {
        let slug = slugify(&self.subject);
        format!("{:04}-{}.patch", self.meta.id, slug)
    }

    /// Generate metadata filename
    pub fn meta_filename(&self) -> String {
        format!("{:04}.meta.toml", self.meta.id)
    }

    /// Parse subject from patch content
    pub fn parse_subject(patch_content: &str) -> Option<String> {
        for line in patch_content.lines() {
            if let Some(subject) = line.strip_prefix("Subject: ") {
                let subject = subject
                    .strip_prefix("[PATCH] ")
                    .or_else(|| subject.strip_prefix("[PATCH 1/1] "))
                    .unwrap_or(subject);
                return Some(subject.to_string());
            }
        }
        None
    }

    /// Parse files changed from patch content
    ///
    /// Tries to extract from diffstat first, falls back to parsing `diff --git` lines.
    pub fn parse_files(patch_content: &str) -> Vec<String> {
        let mut files = Vec::new();
        let mut in_diffstat = false;

        for line in patch_content.lines() {
            if line == "---" {
                in_diffstat = true;
                continue;
            }

            if in_diffstat {
                if line.is_empty() || line.starts_with("diff --git") {
                    break;
                }
                if let Some(file) = line.split('|').next() {
                    let file = file.trim();
                    if !file.is_empty() && !file.contains("changed") && !file.contains("file(s)") {
                        files.push(file.to_string());
                    }
                }
            }
        }

        if files.is_empty() {
            for line in patch_content.lines() {
                if line.starts_with("diff --git ") {
                    if let Some(b_part) = line.split(" b/").nth(1) {
                        files.push(b_part.to_string());
                    }
                }
            }
        }

        files
    }
}

/// Convert a title to a URL-friendly slug
pub(super) fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(50)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("feat(auth): add JWT"), "feat-auth-add-jwt");
        assert_eq!(slugify("Fix bug #123"), "fix-bug-123");
    }

    #[test]
    fn test_parse_subject() {
        let patch = "From: Test\nSubject: [PATCH] feat: add feature\n\nbody";
        assert_eq!(
            StagedPatch::parse_subject(patch),
            Some("feat: add feature".to_string())
        );
    }
}
