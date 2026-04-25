//! Public + internal error types for hunk application.
//!
//! - [`ApplyResult`] is the public outcome of
//!   [`super::UdiffApplier::apply_hunks`].
//! - [`HunkApplyError`] is the per-hunk failure type used internally to
//!   distinguish "no match" from "ambiguous (multiple matches)".

/// Result of applying hunks to a file
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyResult {
    /// All hunks applied successfully
    Success {
        /// The new content after applying all hunks
        new_content: String,
    },
    /// Some hunks applied, some failed
    PartialSuccess {
        /// Indices of successfully applied hunks
        applied: Vec<usize>,
        /// Indices and error messages of failed hunks
        failed: Vec<(usize, String)>,
        /// The content after applying successful hunks
        new_content: String,
    },
    /// A hunk could not be matched
    NoMatch {
        /// Index of the hunk that failed
        hunk_idx: usize,
        /// Suggestion for fixing the issue
        suggestion: String,
    },
    /// Multiple matches found for a hunk
    MultipleMatches {
        /// Index of the hunk that failed
        hunk_idx: usize,
        /// Number of matches found
        count: usize,
    },
}

/// Internal error type for hunk application
#[derive(Debug)]
pub(super) enum HunkApplyError {
    NoMatch {
        suggestion: String,
    },
    #[allow(dead_code)]
    MultipleMatches {
        count: usize,
    },
}
