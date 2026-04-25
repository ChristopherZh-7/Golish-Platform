use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

use super::event_type::{EventType, FeedbackType};
use super::session_event::SessionEvent;

/// Commit boundary detector
///
/// Detects logical boundaries where a commit would make sense based on:
/// - File save patterns (many edits followed by pause)
/// - Completion signals in reasoning
/// - User feedback events
pub struct CommitBoundaryDetector {
    /// File edits since last boundary
    recent_edits: Vec<PathBuf>,
    /// Last event timestamp
    last_event_time: Option<DateTime<Utc>>,
    /// Minimum events before considering a boundary
    min_events: usize,
    /// Pause threshold in seconds
    pause_threshold_secs: u64,
}

impl Default for CommitBoundaryDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl CommitBoundaryDetector {
    /// Create a new detector with default settings
    pub fn new() -> Self {
        Self {
            recent_edits: Vec::new(),
            last_event_time: None,
            min_events: 3,
            pause_threshold_secs: 60,
        }
    }

    /// Create with custom thresholds
    #[cfg(test)]
    pub fn with_thresholds(min_events: usize, pause_threshold_secs: u64) -> Self {
        Self {
            recent_edits: Vec::new(),
            last_event_time: None,
            min_events,
            pause_threshold_secs,
        }
    }

    /// Check if an event suggests a commit boundary
    pub fn check_boundary(&mut self, event: &SessionEvent) -> Option<CommitBoundaryInfo> {
        let now = event.timestamp;

        // Track file edits
        if let EventType::FileEdit { path, .. } = &event.event_type {
            if !self.recent_edits.contains(path) {
                self.recent_edits.push(path.clone());
            }
        }

        // Check for explicit completion signals
        if let Some(boundary) = self.check_completion_signals(event) {
            return Some(boundary);
        }

        // Check for pause-based boundary
        if let Some(boundary) = self.check_pause_boundary(now) {
            return Some(boundary);
        }

        self.last_event_time = Some(now);
        None
    }

    /// Check for completion signals in reasoning
    fn check_completion_signals(&mut self, event: &SessionEvent) -> Option<CommitBoundaryInfo> {
        match &event.event_type {
            EventType::AgentReasoning { content, .. } => {
                let lower = content.to_lowercase();
                let is_completion = lower.contains("done")
                    || lower.contains("complete")
                    || lower.contains("finished")
                    || lower.contains("implemented")
                    || lower.contains("ready to commit")
                    || lower.contains("ready for review");

                if is_completion && self.recent_edits.len() >= self.min_events {
                    return Some(self.create_boundary("Completion signal detected"));
                }
            }
            EventType::UserFeedback {
                feedback_type: FeedbackType::Approve,
                ..
            } => {
                if self.recent_edits.len() >= self.min_events {
                    return Some(self.create_boundary("User approved changes"));
                }
            }
            EventType::SessionEnd { .. } => {
                if !self.recent_edits.is_empty() {
                    return Some(self.create_boundary("Session ended"));
                }
            }
            _ => {}
        }
        None
    }

    /// Check for pause-based boundary
    fn check_pause_boundary(&mut self, now: DateTime<Utc>) -> Option<CommitBoundaryInfo> {
        if let Some(last) = self.last_event_time {
            let pause_duration = (now - last).num_seconds() as u64;

            if pause_duration >= self.pause_threshold_secs
                && self.recent_edits.len() >= self.min_events
            {
                return Some(self.create_boundary("Pause in activity detected"));
            }
        }
        None
    }

    /// Create a boundary info and reset state
    fn create_boundary(&mut self, reason: &str) -> CommitBoundaryInfo {
        let files = std::mem::take(&mut self.recent_edits);
        CommitBoundaryInfo {
            files_in_scope: files,
            reason: reason.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Get files edited since last boundary (without creating a boundary)
    pub fn pending_files(&self) -> &[PathBuf] {
        &self.recent_edits
    }

    /// Clear pending edits (e.g., after user commits manually)
    pub fn clear(&mut self) {
        self.recent_edits.clear();
        self.last_event_time = None;
    }
}

/// Information about a detected commit boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitBoundaryInfo {
    /// Files that should be in this commit
    pub files_in_scope: Vec<PathBuf>,
    /// Why this boundary was detected
    pub reason: String,
    /// When the boundary was detected
    pub timestamp: DateTime<Utc>,
}
