//! Fuzzy line-window matching using `similar::TextDiff` similarity scores.
//!
//! Slides a window of `old_lines.len()` over the content lines, computes a
//! similarity score for each candidate position, and either applies the
//! replacement (single match above threshold) or returns a structured
//! [`FuzzyMatchResult`] describing the failure mode (multiple matches /
//! best-similarity-too-low) so the caller can produce a helpful diagnostic.

use similar::TextDiff;

use crate::parser::ParsedHunk;

use super::UdiffApplier;

const SIMILARITY_EPSILON: f32 = 0.02;

/// Result of fuzzy matching attempt
#[derive(Debug)]
#[allow(dead_code)] // Fields used in Debug output for diagnostics
pub(super) enum FuzzyMatchResult {
    /// Found a single match above threshold
    Match {
        new_content: String,
        similarity: f32,
    },
    /// Found multiple ambiguous matches above threshold
    MultipleMatches { count: usize, best_similarity: f32 },
    /// No match found above threshold
    NoMatch {
        best_similarity: f32,
        best_location: Option<usize>,
    },
}

impl UdiffApplier {
    /// Try to apply hunk with fuzzy matching using similarity threshold
    ///
    /// This method slides a window over the content lines and computes
    /// similarity scores for each candidate position. If exactly one
    /// position meets the threshold, the replacement is applied.
    pub(super) fn try_fuzzy_apply(content: &str, hunk: &ParsedHunk, threshold: f32) -> FuzzyMatchResult {
        let old_lines = &hunk.old_lines;
        let new_lines = &hunk.new_lines;

        // Handle empty old_lines (pure insertion) - can't fuzzy match
        if old_lines.is_empty() {
            return FuzzyMatchResult::NoMatch {
                best_similarity: 0.0,
                best_location: None,
            };
        }

        let content_lines: Vec<&str> = content.lines().collect();
        let window_size = old_lines.len();

        // Can't match if content has fewer lines than the hunk
        if content_lines.len() < window_size {
            return FuzzyMatchResult::NoMatch {
                best_similarity: 0.0,
                best_location: None,
            };
        }

        let old_text = old_lines.join("\n");
        let mut candidates: Vec<(usize, f32)> = Vec::new();
        let mut best_similarity: f32 = 0.0;
        let mut best_location: Option<usize> = None;

        // Slide window over content and compute similarity
        for i in 0..=content_lines.len() - window_size {
            let window = &content_lines[i..i + window_size];
            let window_text = window.join("\n");

            // Use character-level comparison for better fuzzy matching
            // Line-level is too coarse (entire line must match or it's "different")
            let diff = TextDiff::from_chars(&old_text, &window_text);
            let similarity = diff.ratio();

            // Track best match
            if similarity > best_similarity {
                best_similarity = similarity;
                best_location = Some(i);
            }

            // Collect candidates above threshold
            if similarity >= threshold {
                candidates.push((i, similarity));
            }
        }

        match candidates.len() {
            0 => FuzzyMatchResult::NoMatch {
                best_similarity,
                best_location,
            },
            1 => {
                // Exactly one match - apply the replacement
                let (match_idx, similarity) = candidates[0];
                let new_content =
                    Self::apply_replacement_at(&content_lines, match_idx, window_size, new_lines);
                FuzzyMatchResult::Match {
                    new_content,
                    similarity,
                }
            }
            _ => {
                // Multiple candidates - check if one is clearly better
                candidates
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let best = candidates[0].1;
                let second_best = candidates[1].1;

                if best - second_best > SIMILARITY_EPSILON {
                    // Best match is clearly better - use it
                    let (match_idx, similarity) = candidates[0];
                    let new_content = Self::apply_replacement_at(
                        &content_lines,
                        match_idx,
                        window_size,
                        new_lines,
                    );
                    FuzzyMatchResult::Match {
                        new_content,
                        similarity,
                    }
                } else {
                    // Ambiguous - multiple similar matches
                    FuzzyMatchResult::MultipleMatches {
                        count: candidates.len(),
                        best_similarity: best,
                    }
                }
            }
        }
    }

    /// Apply replacement at a specific line index
    fn apply_replacement_at(
        content_lines: &[&str],
        match_idx: usize,
        old_len: usize,
        new_lines: &[String],
    ) -> String {
        let mut result_lines: Vec<String> = Vec::new();

        // Add lines before match
        result_lines.extend(content_lines[..match_idx].iter().map(|s| s.to_string()));

        // Preserve indentation from the first matched line
        let indent = content_lines
            .get(match_idx)
            .map(|l| Self::get_indentation(l))
            .unwrap_or_default();

        // Add new lines with adjusted indentation
        for new_line in new_lines {
            let trimmed = new_line.trim_start();
            if trimmed.is_empty() {
                result_lines.push(String::new());
            } else {
                // Preserve relative indentation from the new_line
                let new_line_indent = Self::get_indentation(new_line);
                if new_line_indent.is_empty() {
                    result_lines.push(format!("{}{}", indent, trimmed));
                } else {
                    // Keep the original line's indentation
                    result_lines.push(new_line.clone());
                }
            }
        }

        // Add lines after match
        let after_match = match_idx + old_len;
        if after_match < content_lines.len() {
            result_lines.extend(content_lines[after_match..].iter().map(|s| s.to_string()));
        }

        result_lines.join("\n")
    }
}
