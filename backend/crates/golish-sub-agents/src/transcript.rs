//! Sub-agent transcript writer for capturing internal sub-agent events.
//!
//! This module provides functionality to persist sub-agent internal events
//! (tool requests and results) to separate transcript files, keeping them
//! separate from the main agent transcript.

use std::path::{Path, PathBuf};

use golish_core::events::AiEvent;
use golish_core::jsonl::EventTranscriptWriter;

/// Thread-safe writer for sub-agent transcript files.
///
/// Events are stored in JSONL format (one JSON object per line) with timestamps.
#[derive(Debug)]
pub struct SubAgentTranscriptWriter {
    inner: EventTranscriptWriter,
}

impl SubAgentTranscriptWriter {
    /// Creates a new `SubAgentTranscriptWriter` for a specific sub-agent execution.
    ///
    /// Path format: `{base_dir}/{session_id}/subagents/{agent_id}-{request_id}/transcript.json`
    pub async fn new(
        base_dir: &Path,
        session_id: &str,
        agent_id: &str,
        parent_request_id: &str,
    ) -> anyhow::Result<Self> {
        let path = sub_agent_transcript_path(base_dir, session_id, agent_id, parent_request_id);
        Ok(Self {
            inner: EventTranscriptWriter::new(path).await?,
        })
    }

    /// Appends an AI event to the sub-agent transcript.
    pub async fn append(&self, event: &AiEvent) -> anyhow::Result<()> {
        self.inner.append(event).await
    }

    /// Returns the path to the transcript file.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}

/// Constructs the transcript file path for a sub-agent execution.
///
/// # Arguments
///
/// * `base_dir` - The base directory for transcripts (e.g., `~/.golish/transcripts`)
/// * `session_id` - The main session ID
/// * `agent_id` - The sub-agent identifier
/// * `parent_request_id` - The request ID that triggered this sub-agent
///
/// # Returns
///
/// A `PathBuf` pointing to `{base_dir}/{session_id}/subagents/{agent_id}-{request_id}/transcript.json`
pub fn sub_agent_transcript_path(
    base_dir: &Path,
    session_id: &str,
    agent_id: &str,
    parent_request_id: &str,
) -> PathBuf {
    base_dir
        .join(session_id)
        .join("subagents")
        .join(format!("{}-{}", agent_id, parent_request_id))
        .join("transcript.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::TempDir;

    /// Helper to parse JSONL format for tests
    fn parse_jsonl(content: &str) -> Vec<Value> {
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("Invalid JSONL line"))
            .collect()
    }

    #[test]
    fn test_sub_agent_transcript_path() {
        let base_dir = Path::new("/var/log/golish/transcripts");
        let session_id = "session-123";
        let agent_id = "coder";
        let request_id = "req-456";

        let path = sub_agent_transcript_path(base_dir, session_id, agent_id, request_id);

        assert_eq!(
            path,
            PathBuf::from(
                "/var/log/golish/transcripts/session-123/subagents/coder-req-456/transcript.json"
            )
        );
    }

    #[tokio::test]
    async fn test_sub_agent_transcript_writer_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let writer =
            SubAgentTranscriptWriter::new(temp_dir.path(), "session-001", "analyzer", "req-001")
                .await
                .expect("Failed to create writer");

        // Append an event
        let event = AiEvent::SubAgentToolRequest {
            agent_id: "analyzer".to_string(),
            tool_name: "read_file".to_string(),
            args: serde_json::json!({"path": "/tmp/test.rs"}),
            request_id: "tool-001".to_string(),
            parent_request_id: "req-001".to_string(),
        };
        writer.append(&event).await.expect("Failed to append");

        // Verify file was created
        assert!(writer.path().exists());

        // Verify content
        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read");
        let entries = parse_jsonl(&content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["type"], "sub_agent_tool_request");
    }

    #[tokio::test]
    async fn test_sub_agent_transcript_writer_appends_multiple() {
        let temp_dir = TempDir::new().unwrap();
        let writer =
            SubAgentTranscriptWriter::new(temp_dir.path(), "session-002", "coder", "req-002")
                .await
                .expect("Failed to create writer");

        // Append tool request
        let request_event = AiEvent::SubAgentToolRequest {
            agent_id: "coder".to_string(),
            tool_name: "write_file".to_string(),
            args: serde_json::json!({"path": "/tmp/new.rs", "content": "fn main() {}"}),
            request_id: "tool-002".to_string(),
            parent_request_id: "req-002".to_string(),
        };
        writer
            .append(&request_event)
            .await
            .expect("Failed to append request");

        // Append tool result
        let result_event = AiEvent::SubAgentToolResult {
            agent_id: "coder".to_string(),
            tool_name: "write_file".to_string(),
            success: true,
            result: serde_json::json!({"written": true}),
            request_id: "tool-002".to_string(),
            parent_request_id: "req-002".to_string(),
        };
        writer
            .append(&result_event)
            .await
            .expect("Failed to append result");

        // Verify both entries are present
        let content = tokio::fs::read_to_string(writer.path())
            .await
            .expect("Failed to read");
        let entries = parse_jsonl(&content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["type"], "sub_agent_tool_request");
        assert_eq!(entries[1]["type"], "sub_agent_tool_result");
    }
}
