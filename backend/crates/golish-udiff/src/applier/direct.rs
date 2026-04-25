//! Direct + normalized whitespace matching strategies.
//!
//! - [`UdiffApplier::try_direct_apply`]: exact substring match. Single hit
//!   wins; multiple hits → ambiguous, try the next strategy.
//! - [`UdiffApplier::try_normalized_apply`]: line-by-line match with
//!   leading/trailing whitespace trimmed, preserving the original
//!   indentation when substituting new lines.

use crate::parser::ParsedHunk;

use super::UdiffApplier;

impl UdiffApplier {
    /// Try to apply hunk with exact string matching
    pub(super) fn try_direct_apply(content: &str, hunk: &ParsedHunk) -> Option<String> {
        let old_text = hunk.old_lines.join("\n");
        let new_text = hunk.new_lines.join("\n");

        let matches: Vec<usize> = content.match_indices(&old_text).map(|(i, _)| i).collect();

        if matches.len() == 1 {
            // Exactly one match - apply the replacement
            let result = content.replacen(&old_text, &new_text, 1);
            Some(result)
        } else {
            None
        }
    }

    /// Try to apply hunk with normalized whitespace matching
    pub(super) fn try_normalized_apply(content: &str, hunk: &ParsedHunk) -> Option<String> {
        let old_text = hunk.old_lines.join("\n");
        let new_text = hunk.new_lines.join("\n");

        // Normalize by trimming each line
        let normalized_old: Vec<&str> = old_text.lines().map(|l| l.trim()).collect();
        let normalized_new: Vec<&str> = new_text.lines().map(|l| l.trim()).collect();

        let content_lines: Vec<&str> = content.lines().collect();
        let mut matches = Vec::new();

        // Search for matching sequences in content
        for i in 0..=content_lines.len().saturating_sub(normalized_old.len()) {
            let window = &content_lines[i..i + normalized_old.len()];
            let normalized_window: Vec<&str> = window.iter().map(|l| l.trim()).collect();

            if normalized_window == normalized_old {
                matches.push(i);
            }
        }

        if matches.len() == 1 {
            // Exactly one match - apply the replacement
            let match_idx = matches[0];
            let mut result_lines: Vec<String> = Vec::new();

            // Add lines before match
            result_lines.extend(content_lines[..match_idx].iter().map(|s| s.to_string()));

            // Add new lines (preserving original indentation of first matched line)
            if let Some(first_line) = content_lines.get(match_idx) {
                let indent = Self::get_indentation(first_line);
                for new_line in &normalized_new {
                    if new_line.is_empty() {
                        result_lines.push(String::new());
                    } else {
                        result_lines.push(format!("{}{}", indent, new_line));
                    }
                }
            }

            // Add lines after match
            let after_match = match_idx + normalized_old.len();
            if after_match < content_lines.len() {
                result_lines.extend(content_lines[after_match..].iter().map(|s| s.to_string()));
            }

            Some(result_lines.join("\n"))
        } else {
            None
        }
    }

    /// Extract indentation from a line
    pub(super) fn get_indentation(line: &str) -> String {
        line.chars()
            .take_while(|c| c.is_whitespace())
            .collect::<String>()
    }
}
