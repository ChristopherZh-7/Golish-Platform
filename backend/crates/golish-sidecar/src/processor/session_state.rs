//! Per-session processor state: file tracking, dedup detection, boundary state.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::events::CommitBoundaryDetector;

/// Threshold for duplicate event warning (same event type in 1 second window)
pub(super) const DUPLICATE_EVENT_THRESHOLD: u32 = 5;
/// Window size for tracking duplicate events
pub(super) const DUPLICATE_WINDOW_SECS: u64 = 1;

/// Tracks file changes for patch generation
#[derive(Debug, Default)]
pub(super) struct FileChangeTracker {
    /// Files changed since last commit boundary
    files: Vec<PathBuf>,
}

impl FileChangeTracker {
    pub(super) fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub(super) fn record_change(&mut self, path: PathBuf) {
        if !self.files.contains(&path) {
            self.files.push(path);
        }
    }

    pub(super) fn get_files(&self) -> Vec<PathBuf> {
        self.files.clone()
    }

    pub(super) fn clear(&mut self) {
        self.files.clear();
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// State for a single session's processing
pub(super) struct SessionProcessorState {
    /// Commit boundary detector
    pub(super) boundary_detector: CommitBoundaryDetector,
    /// File change tracker for patch generation
    pub(super) file_tracker: FileChangeTracker,
    /// All files modified during session (for state.md updates)
    pub(super) all_modified_files: Vec<PathBuf>,
    /// Tool calls completed during session (for progress tracking)
    pub(super) completed_tools: Vec<String>,
    /// Event count for this session
    pub(super) event_count: u32,
    /// Recent events for deduplication detection (event_type -> count in window)
    pub(super) recent_event_counts: HashMap<String, u32>,
    /// Last reset timestamp for recent_event_counts
    last_event_count_reset: std::time::Instant,
}

impl SessionProcessorState {
    pub(super) fn new() -> Self {
        Self {
            boundary_detector: CommitBoundaryDetector::new(),
            file_tracker: FileChangeTracker::new(),
            all_modified_files: Vec::new(),
            completed_tools: Vec::new(),
            event_count: 0,
            recent_event_counts: HashMap::new(),
            last_event_count_reset: std::time::Instant::now(),
        }
    }

    /// Track event for deduplication detection
    /// Returns true if this might be a duplicate (high frequency of same event type)
    pub(super) fn track_event_frequency(&mut self, event_type: &str) -> bool {
        let now = std::time::Instant::now();

        if now.duration_since(self.last_event_count_reset).as_secs() >= DUPLICATE_WINDOW_SECS {
            self.recent_event_counts.clear();
            self.last_event_count_reset = now;
        }

        let count = self
            .recent_event_counts
            .entry(event_type.to_string())
            .or_insert(0);
        *count += 1;

        *count > DUPLICATE_EVENT_THRESHOLD
    }

    /// Record a modified file (deduplicates)
    pub(super) fn record_modified_file(&mut self, path: PathBuf) {
        if !self.all_modified_files.contains(&path) {
            self.all_modified_files.push(path);
        }
    }

    /// Record a completed tool call
    pub(super) fn record_tool_call(&mut self, tool_name: &str, success: bool) {
        let status = if success { "✓" } else { "✗" };
        self.completed_tools
            .push(format!("{} {}", tool_name, status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_change_tracker_records_unique_paths() {
        let mut tracker = FileChangeTracker::new();

        tracker.record_change(PathBuf::from("src/main.rs"));
        tracker.record_change(PathBuf::from("src/lib.rs"));
        tracker.record_change(PathBuf::from("src/main.rs"));

        assert_eq!(tracker.get_files().len(), 2);
        assert!(tracker.get_files().contains(&PathBuf::from("src/main.rs")));
        assert!(tracker.get_files().contains(&PathBuf::from("src/lib.rs")));
    }

    #[test]
    fn test_file_change_tracker_clear() {
        let mut tracker = FileChangeTracker::new();

        tracker.record_change(PathBuf::from("src/main.rs"));
        assert!(!tracker.is_empty());

        tracker.clear();
        assert!(tracker.is_empty());
    }
}
