//! Top-level dispatcher: walks each hunk through direct → normalized →
//! fuzzy, aggregating per-hunk outcomes into [`ApplyResult`].

use crate::parser::ParsedHunk;

use super::errors::{ApplyResult, HunkApplyError};
use super::fuzzy::FuzzyMatchResult;
use super::UdiffApplier;

const DEFAULT_FUZZY_THRESHOLD: f32 = 0.85;

impl UdiffApplier {
    /// Apply hunks to file content
    ///
    /// Tries multiple matching strategies in order:
    /// 1. Direct exact match
    /// 2. Normalized match (ignoring leading/trailing whitespace)
    /// 3. Fuzzy match (using similarity threshold)
    pub fn apply_hunks(content: &str, hunks: &[ParsedHunk]) -> ApplyResult {
        let mut current_content = content.to_string();
        let mut applied = Vec::new();
        let mut failed = Vec::new();

        for (idx, hunk) in hunks.iter().enumerate() {
            match Self::apply_single_hunk(&current_content, hunk) {
                Ok(new_content) => {
                    current_content = new_content;
                    applied.push(idx);
                }
                Err(HunkApplyError::NoMatch { suggestion }) => {
                    if applied.is_empty() {
                        // No hunks applied yet, return NoMatch
                        return ApplyResult::NoMatch {
                            hunk_idx: idx,
                            suggestion,
                        };
                    } else {
                        // Some hunks already applied
                        failed.push((idx, suggestion));
                    }
                }
                Err(HunkApplyError::MultipleMatches { count }) => {
                    if applied.is_empty() {
                        return ApplyResult::MultipleMatches {
                            hunk_idx: idx,
                            count,
                        };
                    } else {
                        failed.push((idx, format!("Found {} matches, need more context", count)));
                    }
                }
            }
        }

        if failed.is_empty() {
            ApplyResult::Success {
                new_content: current_content,
            }
        } else {
            ApplyResult::PartialSuccess {
                applied,
                failed,
                new_content: current_content,
            }
        }
    }

    /// Apply a single hunk to content
    fn apply_single_hunk(content: &str, hunk: &ParsedHunk) -> Result<String, HunkApplyError> {
        // Try direct match first
        if let Some(result) = Self::try_direct_apply(content, hunk) {
            return Ok(result);
        }

        // Try normalized match
        if let Some(result) = Self::try_normalized_apply(content, hunk) {
            return Ok(result);
        }

        // Try fuzzy match
        match Self::try_fuzzy_apply(content, hunk, DEFAULT_FUZZY_THRESHOLD) {
            FuzzyMatchResult::Match { new_content, .. } => Ok(new_content),
            FuzzyMatchResult::MultipleMatches { count, .. } => {
                Err(HunkApplyError::MultipleMatches { count })
            }
            FuzzyMatchResult::NoMatch { best_similarity, .. } => {
                Err(HunkApplyError::NoMatch {
                    suggestion: format!(
                        "Could not find context lines (best fuzzy match: {:.0}%, threshold: {:.0}%). Expected to find:\n{}",
                        best_similarity * 100.0,
                        DEFAULT_FUZZY_THRESHOLD * 100.0,
                        hunk.old_lines.join("\n")
                    ),
                })
            }
        }
    }
}
