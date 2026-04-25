//! Capture tests.

use std::path::PathBuf;
use std::sync::Arc;

use golish_core::events::AiEvent;

use crate::events::{DecisionType, FeedbackType, FileOperation, SessionEvent};
use crate::state::SidecarState;

use super::context::CaptureContext;
use super::diff::generate_unified_diff;
use super::extractors::{extract_files_modified, extract_path_from_args};
use super::format::{infer_decision_type, summarize_args, truncate_path};
use super::tool_classification::{is_read_tool, is_write_tool};

#[test]
fn test_summarize_args() {
    let args = serde_json::json!({
        "path": "src/main.rs",
        "display_description": "Add main function"
    });
    let summary = summarize_args(&args);
    assert!(summary.contains("path="));
    assert!(summary.contains("desc="));
}

#[test]
fn test_infer_decision_type() {
    assert_eq!(
        infer_decision_type("I'll use the tokio runtime for async"),
        Some(DecisionType::ApproachChoice)
    );
    assert_eq!(
        infer_decision_type("There's a tradeoff between speed and safety"),
        Some(DecisionType::Tradeoff)
    );
    assert_eq!(
        infer_decision_type("Using a fallback approach"),
        Some(DecisionType::Fallback)
    );
    assert_eq!(
        infer_decision_type("Assuming the API returns JSON"),
        Some(DecisionType::Assumption)
    );
    assert_eq!(infer_decision_type("Just reading the file"), None);
}

#[test]
fn test_truncate_path() {
    assert_eq!(truncate_path("short.rs", 20), "short.rs");
    assert_eq!(
        truncate_path("very/long/path/to/file.rs", 15),
        "...h/to/file.rs"
    );
}

#[test]
fn test_is_read_tool() {
    assert!(is_read_tool("read_file"));
    assert!(is_read_tool("grep"));
    assert!(!is_read_tool("write_file"));
}

#[test]
fn test_is_write_tool() {
    assert!(is_write_tool("write_file"));
    assert!(is_write_tool("edit_file"));
    assert!(!is_write_tool("read_file"));
}

#[test]
fn test_extract_path_from_args() {
    let args = serde_json::json!({"path": "src/main.rs"});
    assert_eq!(
        extract_path_from_args(&args),
        Some(PathBuf::from("src/main.rs"))
    );

    let args = serde_json::json!({"file_path": "lib.rs"});
    assert_eq!(extract_path_from_args(&args), Some(PathBuf::from("lib.rs")));

    let args = serde_json::json!({"other": "value"});
    assert_eq!(extract_path_from_args(&args), None);
}

#[test]
fn test_extract_files_modified_single_path() {
    let args = serde_json::json!({"path": "src/main.rs"});
    let files = extract_files_modified("write_file", Some(&args));
    assert_eq!(files, vec![PathBuf::from("src/main.rs")]);
}

#[test]
fn test_extract_files_modified_rename() {
    let args = serde_json::json!({
        "source_path": "old.rs",
        "destination_path": "new.rs"
    });
    let files = extract_files_modified("move_file", Some(&args));
    assert_eq!(files.len(), 2);
    assert!(files.contains(&PathBuf::from("old.rs")));
    assert!(files.contains(&PathBuf::from("new.rs")));
}

#[test]
fn test_generate_unified_diff_simple() {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nmodified\nline3\n";
    let diff = generate_unified_diff(old, new, "test.txt");
    assert!(diff.contains("--- a/test.txt"));
    assert!(diff.contains("+++ b/test.txt"));
    assert!(diff.contains("-line2"));
    assert!(diff.contains("+modified"));
}

// =========================================================================
// Integration tests for AI event -> SessionEvent flow
// =========================================================================

mod integration {
    use super::*;
    use crate::config::SidecarConfig;
    use crate::state::SidecarState;
    use tempfile::TempDir;

    fn test_config(temp_dir: &std::path::Path) -> SidecarConfig {
        SidecarConfig {
            enabled: true,
            sessions_dir: Some(temp_dir.to_path_buf()),
            retention_days: 0,
            max_state_size: 16 * 1024,
            write_raw_events: false,
            use_llm_for_state: false,
            capture_tool_calls: true,
            capture_reasoning: true,
            synthesis_enabled: false,
            synthesis_backend: golish_synthesis::SynthesisBackend::Template,
            artifact_synthesis_backend: golish_artifacts::ArtifactSynthesisBackend::Template,
            synthesis_vertex: Default::default(),
            synthesis_openai: Default::default(),
            synthesis_grok: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_tool_request_stores_state_for_result() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let sidecar = Arc::new(SidecarState::with_config(config));

        sidecar.initialize(temp.path().to_path_buf()).await.unwrap();
        let _session_id = sidecar.start_session("Test session").unwrap();

        // Give time for async session creation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut capture = CaptureContext::new(sidecar.clone());

        // Process tool request first
        capture.process(&AiEvent::ToolRequest {
            request_id: "test-1".to_string(),
            tool_name: "write_file".to_string(),
            args: serde_json::json!({"path": "src/test.rs", "content": "fn main() {}"}),
            source: golish_core::events::ToolSource::Main,
        });

        // Verify state was stored
        assert_eq!(capture.last_tool_name, Some("write_file".to_string()));
        assert!(capture.last_tool_args.is_some());
    }

    #[tokio::test]
    async fn test_tool_result_clears_state() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let sidecar = Arc::new(SidecarState::with_config(config));

        sidecar.initialize(temp.path().to_path_buf()).await.unwrap();
        let _session_id = sidecar.start_session("Test session").unwrap();

        // Give time for async session creation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut capture = CaptureContext::new(sidecar.clone());

        // Process tool request then result
        capture.process(&AiEvent::ToolRequest {
            request_id: "test-1".to_string(),
            tool_name: "read_file".to_string(),
            args: serde_json::json!({"path": "src/test.rs"}),
            source: golish_core::events::ToolSource::Main,
        });

        capture.process(&AiEvent::ToolResult {
            tool_name: "read_file".to_string(),
            result: serde_json::json!({"content": "file contents"}),
            success: true,
            request_id: "test-1".to_string(),
            source: golish_core::events::ToolSource::Main,
        });

        // State should be cleared after result
        assert!(capture.last_tool_name.is_none());
        assert!(capture.last_tool_args.is_none());
    }

    #[tokio::test]
    async fn test_write_tool_captures_file_edit_event() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let sidecar = Arc::new(SidecarState::with_config(config));

        sidecar.initialize(temp.path().to_path_buf()).await.unwrap();
        let _session_id = sidecar.start_session("Test session").unwrap();

        // Give time for async session creation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut capture = CaptureContext::new(sidecar.clone());

        // Process write_file tool
        capture.process(&AiEvent::ToolRequest {
            request_id: "test-1".to_string(),
            tool_name: "write_file".to_string(),
            args: serde_json::json!({"path": "/tmp/test.rs", "content": "fn main() {}"}),
            source: golish_core::events::ToolSource::Main,
        });

        capture.process(&AiEvent::ToolResult {
            tool_name: "write_file".to_string(),
            result: serde_json::json!({"success": true}),
            success: true,
            request_id: "test-1".to_string(),
            source: golish_core::events::ToolSource::Main,
        });

        // The capture should have processed both events
        // State should be cleared
        assert!(capture.last_tool_name.is_none());
    }

    #[tokio::test]
    async fn test_reasoning_event_accumulated_and_flushed() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let sidecar = Arc::new(SidecarState::with_config(config));

        sidecar.initialize(temp.path().to_path_buf()).await.unwrap();
        let _session_id = sidecar.start_session("Test session").unwrap();

        // Give time for async session creation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut capture = CaptureContext::new(sidecar.clone());

        // Process multiple reasoning chunks (simulating streaming)
        capture.process(&AiEvent::Reasoning {
            content: "First chunk. ".to_string(),
        });
        capture.process(&AiEvent::Reasoning {
            content: "Second chunk. ".to_string(),
        });
        capture.process(&AiEvent::Reasoning {
            content: "I've completed the implementation.".to_string(),
        });

        // Verify reasoning is accumulated but not yet flushed
        assert_eq!(
            capture.accumulated_reasoning,
            "First chunk. Second chunk. I've completed the implementation."
        );

        // Process completed event to flush accumulated reasoning
        capture.process(&AiEvent::Completed {
            response: "Done!".to_string(),
            reasoning: None,
            input_tokens: None,
            output_tokens: None,
            duration_ms: None,
        });

        // Accumulated reasoning should now be cleared
        assert!(capture.accumulated_reasoning.is_empty());
    }

    #[tokio::test]
    async fn test_edit_tool_captures_diff() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let sidecar = Arc::new(SidecarState::with_config(config));

        sidecar.initialize(temp.path().to_path_buf()).await.unwrap();
        let _session_id = sidecar.start_session("Test session").unwrap();

        // Give time for async session creation
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Create a test file to edit
        let test_file = temp.path().join("test.rs");
        std::fs::write(&test_file, "fn original() {}").unwrap();

        let mut capture = CaptureContext::new(sidecar.clone());

        // Process edit_file tool request (captures old content)
        capture.process(&AiEvent::ToolRequest {
            request_id: "test-1".to_string(),
            tool_name: "edit_file".to_string(),
            args: serde_json::json!({
                "path": test_file.to_string_lossy(),
                "display_description": "Update function"
            }),
            source: golish_core::events::ToolSource::Main,
        });

        // Verify old content was captured for diff
        assert!(capture.pending_old_content.is_some());
        let (path, content) = capture.pending_old_content.as_ref().unwrap();
        assert_eq!(*path, test_file);
        assert_eq!(content, "fn original() {}");

        // Simulate file modification
        std::fs::write(&test_file, "fn modified() {}").unwrap();

        // Process result
        capture.process(&AiEvent::ToolResult {
            tool_name: "edit_file".to_string(),
            result: serde_json::json!({"success": true}),
            success: true,
            request_id: "test-1".to_string(),
            source: golish_core::events::ToolSource::Main,
        });

        // State should be cleared
        assert!(capture.pending_old_content.is_none());
    }
}
