//! Session management for simplified sidecar storage.
//!
//! Each session is stored as a directory containing:
//! - `state.md`: YAML frontmatter (metadata) + markdown body (context)
//! - `patches/staged/`: Pending patches in git format-patch style
//! - `patches/applied/`: Applied patches (moved after git am)

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

#[cfg(test)]
mod tests;

/// Session status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Completed,
    Abandoned,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Active => write!(f, "active"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Abandoned => write!(f, "abandoned"),
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(SessionStatus::Active),
            "completed" => Ok(SessionStatus::Completed),
            "abandoned" => Ok(SessionStatus::Abandoned),
            _ => anyhow::bail!("Invalid session status: {}", s),
        }
    }
}

/// Session metadata (stored in YAML frontmatter of state.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub cwd: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_root: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    pub initial_request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl SessionMeta {
    pub fn new(session_id: String, cwd: PathBuf, initial_request: String) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
            cwd,
            git_root: None,
            git_branch: None,
            initial_request,
            title: None,
        }
    }
}

/// Manages a single session's files
pub struct Session {
    dir: PathBuf,
    meta: SessionMeta,
}

impl Session {
    const STATE_FILE: &'static str = "state.md";
    const LOG_FILE: &'static str = "log.md";
    const PATCHES_DIR: &'static str = "patches";
    const ARTIFACTS_DIR: &'static str = "artifacts";
    const STAGED_DIR: &'static str = "staged";
    const PENDING_DIR: &'static str = "pending";
    const APPLIED_DIR: &'static str = "applied";

    pub async fn create(
        sessions_dir: &Path,
        session_id: String,
        cwd: PathBuf,
        initial_request: String,
    ) -> Result<Self> {
        let dir = sessions_dir.join(&session_id);

        fs::create_dir_all(&dir)
            .await
            .context("Failed to create session directory")?;

        fs::create_dir_all(dir.join(Self::PATCHES_DIR).join(Self::STAGED_DIR))
            .await
            .context("Failed to create staged patches directory")?;
        fs::create_dir_all(dir.join(Self::PATCHES_DIR).join(Self::APPLIED_DIR))
            .await
            .context("Failed to create applied patches directory")?;

        fs::create_dir_all(dir.join(Self::ARTIFACTS_DIR).join(Self::PENDING_DIR))
            .await
            .context("Failed to create pending artifacts directory")?;
        fs::create_dir_all(dir.join(Self::ARTIFACTS_DIR).join(Self::APPLIED_DIR))
            .await
            .context("Failed to create applied artifacts directory")?;

        let meta = SessionMeta::new(session_id.clone(), cwd, initial_request.clone());

        let state_content = Self::format_state_file(&meta, &initial_state_body(&initial_request));
        fs::write(dir.join(Self::STATE_FILE), &state_content)
            .await
            .context("Failed to write state.md")?;

        let log_content = format!(
            "# Session Log\n\n> Session started: {}\n\n",
            meta.created_at.format("%Y-%m-%d %H:%M:%S UTC")
        );
        fs::write(dir.join(Self::LOG_FILE), &log_content)
            .await
            .context("Failed to write log.md")?;

        tracing::info!("Created new session: {}", session_id);

        Ok(Self { dir, meta })
    }

    pub async fn load(sessions_dir: &Path, session_id: &str) -> Result<Self> {
        let dir = sessions_dir.join(session_id);

        if !dir.exists() {
            anyhow::bail!("Session directory does not exist: {}", session_id);
        }

        let state_path = dir.join(Self::STATE_FILE);
        let content = fs::read_to_string(&state_path)
            .await
            .context("Failed to read state.md")?;

        let (meta, _body) = Self::parse_state_file(&content)?;

        Ok(Self { dir, meta })
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn read_state(&self) -> Result<String> {
        let path = self.dir.join(Self::STATE_FILE);
        if path.exists() {
            let content = fs::read_to_string(&path)
                .await
                .context("Failed to read state.md")?;

            let (_meta, body) = Self::parse_state_file(&content)?;
            Ok(body)
        } else {
            Ok(String::new())
        }
    }

    pub async fn read_log(&self) -> Result<String> {
        let path = self.dir.join(Self::LOG_FILE);
        if path.exists() {
            fs::read_to_string(&path)
                .await
                .context("Failed to read log.md")
        } else {
            Ok(String::new())
        }
    }

    pub async fn append_log(&self, entry: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let path = self.dir.join(Self::LOG_FILE);
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let formatted = format!("\n---\n\n**{}**\n\n{}\n", timestamp, entry);

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .context("Failed to open log.md")?;

        file.write_all(formatted.as_bytes())
            .await
            .context("Failed to append to log.md")?;

        Ok(())
    }

    pub async fn update_state(&mut self, new_body: &str) -> Result<()> {
        self.meta.updated_at = Utc::now();

        let content = Self::format_state_file(&self.meta, new_body);
        fs::write(self.dir.join(Self::STATE_FILE), &content)
            .await
            .context("Failed to write state.md")?;

        Ok(())
    }

    pub async fn update_meta(&mut self, new_meta: &SessionMeta) -> Result<()> {
        self.meta = new_meta.clone();

        let body = self.read_state().await.unwrap_or_default();
        let content = Self::format_state_file(&self.meta, &body);
        fs::write(self.dir.join(Self::STATE_FILE), &content)
            .await
            .context("Failed to write state.md")?;

        Ok(())
    }

    pub async fn complete(&mut self) -> Result<()> {
        self.meta.status = SessionStatus::Completed;
        self.meta.updated_at = Utc::now();

        let body = self.read_state().await.unwrap_or_default();
        let content = Self::format_state_file(&self.meta, &body);
        fs::write(self.dir.join(Self::STATE_FILE), &content)
            .await
            .context("Failed to write state.md")?;

        tracing::info!("Session completed: {}", self.meta.session_id);
        Ok(())
    }

    pub async fn set_title(&mut self, title: String) -> Result<()> {
        self.meta.title = Some(title);
        self.meta.updated_at = Utc::now();

        let body = self.read_state().await.unwrap_or_default();
        let content = Self::format_state_file(&self.meta, &body);
        fs::write(self.dir.join(Self::STATE_FILE), &content)
            .await
            .context("Failed to write state.md")?;

        tracing::debug!("Session title set: {}", self.meta.session_id);
        Ok(())
    }

    fn format_state_file(meta: &SessionMeta, body: &str) -> String {
        let yaml = serde_yaml::to_string(meta).unwrap_or_default();
        format!("---\n{}---\n\n{}", yaml, body)
    }

    pub(crate) fn parse_state_file(content: &str) -> Result<(SessionMeta, String)> {
        if !content.starts_with("---\n") {
            anyhow::bail!("state.md missing YAML frontmatter");
        }

        let rest = &content[4..];
        let end_idx = rest
            .find("\n---")
            .context("state.md missing frontmatter closing delimiter")?;

        let yaml_content = &rest[..end_idx];
        let body_start = end_idx + 4;

        let body = rest[body_start..].trim_start_matches('\n').to_string();

        let meta: SessionMeta =
            serde_yaml::from_str(yaml_content).context("Failed to parse state.md frontmatter")?;

        Ok((meta, body))
    }
}

fn strip_context_tags(request: &str) -> String {
    let mut result = request.to_string();

    if let Some(start) = result.find("<context>") {
        if let Some(end) = result.find("</context>") {
            let end_tag_len = "</context>".len();
            result = format!("{}{}", &result[..start], &result[end + end_tag_len..]);
        }
    }

    result.trim().to_string()
}

fn initial_state_body(initial_request: &str) -> String {
    let clean_request = strip_context_tags(initial_request);
    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    format!(
        r#"# Session State
Updated: {}

## Goals
- {}

## Changes
(none yet)
"#,
        timestamp, clean_request
    )
}

/// List all sessions in the sessions directory
pub async fn list_sessions(sessions_dir: &Path) -> Result<Vec<SessionMeta>> {
    let mut sessions = Vec::new();

    if !sessions_dir.exists() {
        return Ok(sessions);
    }

    let mut entries = fs::read_dir(sessions_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            let session_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            match Session::load(sessions_dir, session_id).await {
                Ok(session) => sessions.push(session.meta().clone()),
                Err(e) => {
                    tracing::warn!("Failed to load session {}: {}", session_id, e);
                }
            }
        }
    }

    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));

    Ok(sessions)
}

/// Get the default sessions directory.
pub fn default_sessions_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VT_SESSION_DIR") {
        return PathBuf::from(dir);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("sessions")
}

/// Ensure sessions directory exists
pub async fn ensure_sessions_dir(sessions_dir: &Path) -> Result<()> {
    fs::create_dir_all(sessions_dir)
        .await
        .context("Failed to create sessions directory")?;
    Ok(())
}
