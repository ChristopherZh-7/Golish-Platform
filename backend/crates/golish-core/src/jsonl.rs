use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Generic JSONL (line-delimited JSON) file appender.
///
/// Thread-safe: uses an async `Mutex` for atomic writes.
/// Each call to `append` serializes the value as a single JSON line.
#[derive(Debug)]
pub struct JsonlAppender {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonlAppender {
    /// Create a new appender targeting `path`.
    /// Parent directories are created automatically.
    pub async fn new(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(Self {
            path,
            write_lock: Mutex::new(()),
        })
    }

    /// Serialize `value` to JSON and append as a single line.
    pub async fn append<T: Serialize>(&self, value: &T) -> anyhow::Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');

        let _guard = self.write_lock.lock().await;
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;

        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Timestamped wrapper for event-style transcript entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampedEntry<E: Clone> {
    pub _timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub event: E,
}

/// Generic transcript writer that wraps `JsonlAppender` and auto-timestamps entries.
///
/// Used by both the main agent transcript and sub-agent transcripts.
#[derive(Debug)]
pub struct EventTranscriptWriter {
    inner: JsonlAppender,
}

impl EventTranscriptWriter {
    pub async fn new(path: PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            inner: JsonlAppender::new(path).await?,
        })
    }

    pub async fn append<E: Serialize + Clone>(&self, event: &E) -> anyhow::Result<()> {
        let entry = TimestampedEntry {
            _timestamp: Utc::now(),
            event: event.clone(),
        };
        self.inner.append(&entry).await
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}
