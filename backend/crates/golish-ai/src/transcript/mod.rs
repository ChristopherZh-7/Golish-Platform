//! Transcript writer for capturing AI events to JSON files.
//!
//! This module provides functionality to persist AI events to disk in a
//! JSONL (line-delimited JSON) format, enabling replay, debugging, and analysis
//! of agent sessions.

mod summarizer;
#[cfg(test)]
mod tests;

pub use summarizer::{format_for_summarizer, build_summarizer_input, save_summarizer_input, save_summary};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use golish_core::events::AiEvent;
use golish_core::jsonl::{EventTranscriptWriter, TimestampedEntry};

/// Returns true if this event should be written to the transcript file.
///
/// Filters out streaming and sub-agent internal events (captured elsewhere).
pub fn should_transcript(event: &AiEvent) -> bool {
    !matches!(
        event,
        AiEvent::TextDelta { .. }
            | AiEvent::Reasoning { .. }
            | AiEvent::ToolOutputChunk { .. }
            | AiEvent::SubAgentToolRequest { .. }
            | AiEvent::SubAgentToolResult { .. }
            | AiEvent::SubAgentTextDelta { .. }
    )
}

#[derive(Debug, Clone)]
pub struct TranscriptEvent {
    pub timestamp: DateTime<Utc>,
    pub event: AiEvent,
}

impl From<TimestampedEntry<AiEvent>> for TranscriptEvent {
    fn from(entry: TimestampedEntry<AiEvent>) -> Self {
        Self {
            timestamp: entry._timestamp,
            event: entry.event,
        }
    }
}

#[derive(Debug)]
pub struct TranscriptWriter {
    inner: EventTranscriptWriter,
}

impl TranscriptWriter {
    pub async fn new(base_dir: &Path, session_id: &str) -> anyhow::Result<Self> {
        let path = transcript_path(base_dir, session_id);
        Ok(Self {
            inner: EventTranscriptWriter::new(path).await?,
        })
    }

    pub async fn append(&self, event: &AiEvent) -> anyhow::Result<()> {
        self.inner.append(event).await
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}

/// Constructs the transcript file path for a given base directory and session ID.
///
/// # Arguments
///
/// * `base_dir` - The base directory for transcripts (e.g., `~/.golish/transcripts`)
/// * `session_id` - A unique identifier for the session
///
/// # Returns
///
/// A `PathBuf` pointing to `{base_dir}/{session_id}/transcript.json`
pub fn transcript_path(base_dir: &Path, session_id: &str) -> PathBuf {
    base_dir.join(session_id).join("transcript.json")
}

/// Read all events from a transcript file.
///
/// Returns events in chronological order (the order they were written).
///
/// # Arguments
///
/// * `base_dir` - The base directory for transcripts (e.g., `~/.golish/transcripts`)
/// * `session_id` - The unique identifier for the session
///
/// # Returns
///
/// A vector of [`TranscriptEvent`]s in chronological order.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or cannot be read.
/// Empty files and files containing an empty JSON array (`[]`) return an empty `Vec`.
///
/// # Example
///
/// ```ignore
/// use golish_ai::transcript::read_transcript;
/// use std::path::Path;
///
/// let events = read_transcript(Path::new("/tmp/transcripts"), "session-123")?;
/// for event in events {
///     println!("{}: {:?}", event.timestamp, event.event);
/// }
/// ```
pub async fn read_transcript(
    base_dir: &Path,
    session_id: &str,
) -> anyhow::Result<Vec<TranscriptEvent>> {
    let path = transcript_path(base_dir, session_id);

    let content = tokio::fs::read_to_string(&path).await?;

    // Handle empty file
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    // Try JSONL first (one JSON object per line)
    let mut entries = Vec::new();
    let mut jsonl_failed = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<TimestampedEntry<AiEvent>>(trimmed) {
            Ok(entry) => entries.push(entry),
            Err(_) => {
                jsonl_failed = true;
                break;
            }
        }
    }

    if jsonl_failed {
        // Fall back to JSON array format (legacy transcripts)
        entries = serde_json::from_str(&content)?;
    }

    // Convert to public TranscriptEvent type
    Ok(entries.into_iter().map(TranscriptEvent::from).collect())
}
