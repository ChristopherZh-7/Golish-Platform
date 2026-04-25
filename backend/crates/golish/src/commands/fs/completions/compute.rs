//! Path completion engine: parse input, expand tilde, score with nucleo,
//! build display text.

use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::app::workspace::expand_tilde_string;

use super::{PathCompletion, PathCompletionResponse, PathEntryType};


/// Compute path completions for a partial path.
///
/// This is the core completion logic, separated from the Tauri command for easier testing.
pub fn compute_path_completions(
    partial_path: &str,
    working_dir: &Path,
    limit: usize,
) -> PathCompletionResponse {
    let (search_dir, prefix) = parse_path_input(partial_path, working_dir);

    // Read directory entries
    let entries = match std::fs::read_dir(&search_dir) {
        Ok(entries) => entries,
        Err(_) => {
            return PathCompletionResponse {
                completions: Vec::new(),
                total_count: 0,
            }
        }
    };

    // Check if we should include hidden files
    let show_hidden = prefix.starts_with('.');

    // Collect raw entries first
    let raw_entries: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy().to_string();

            // Skip hidden files unless prefix starts with '.'
            if name.starts_with('.') && !show_hidden {
                return None;
            }

            // Determine entry type
            let metadata = entry.metadata().ok()?;
            let file_type = entry.file_type().ok()?;

            let entry_type = if file_type.is_symlink() {
                PathEntryType::Symlink
            } else if metadata.is_dir() {
                PathEntryType::Directory
            } else {
                PathEntryType::File
            };

            Some((name, entry_type))
        })
        .collect();

    // If no prefix, return all entries with score=0 (no fuzzy matching needed)
    let mut completions: Vec<PathCompletion> = if prefix.is_empty() {
        raw_entries
            .into_iter()
            .map(|(name, entry_type)| {
                let (display_name, insert_text) =
                    build_completion_text(&name, &entry_type, partial_path, &prefix);
                PathCompletion {
                    name: display_name,
                    insert_text,
                    entry_type,
                    score: 0,
                    match_indices: Vec::new(),
                }
            })
            .collect()
    } else {
        // Use fuzzy matching with nucleo
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let pattern = Pattern::parse(&prefix, CaseMatching::Smart, Normalization::Smart);

        raw_entries
            .into_iter()
            .filter_map(|(name, entry_type)| {
                let mut indices = Vec::new();
                let mut haystack_buf = Vec::new();
                let haystack = Utf32Str::new(&name, &mut haystack_buf);

                let score = pattern.indices(haystack.slice(..), &mut matcher, &mut indices)?;

                let (display_name, insert_text) =
                    build_completion_text(&name, &entry_type, partial_path, &prefix);

                Some(PathCompletion {
                    name: display_name,
                    insert_text,
                    entry_type,
                    score,
                    match_indices: indices.iter().map(|&i| i as usize).collect(),
                })
            })
            .collect()
    };

    let total_count = completions.len();

    // Sort: by score descending, then directories first, then alphabetically by name
    completions.sort_by(|a, b| {
        // Sort by score descending first (higher score = better match)
        b.score
            .cmp(&a.score)
            .then_with(|| {
                // Then directories first
                let a_is_dir = matches!(a.entry_type, PathEntryType::Directory);
                let b_is_dir = matches!(b.entry_type, PathEntryType::Directory);
                b_is_dir.cmp(&a_is_dir)
            })
            .then_with(|| {
                // Then alphabetically
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            })
    });

    // Apply limit
    completions.truncate(limit);

    PathCompletionResponse {
        completions,
        total_count,
    }
}

/// Parse the partial path input and return (search_directory, prefix_to_match).
fn parse_path_input(partial_path: &str, working_dir: &Path) -> (PathBuf, String) {
    if partial_path.is_empty() {
        // Empty input: list current directory
        return (working_dir.to_path_buf(), String::new());
    }

    // Expand tilde
    let expanded = expand_tilde(partial_path);
    let path = Path::new(&expanded);

    if expanded.ends_with('/') || expanded.ends_with(std::path::MAIN_SEPARATOR) {
        // Path ends with separator: search inside this directory
        let search_dir = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_dir.join(path)
        };
        (search_dir, String::new())
    } else if let Some(parent) = path.parent() {
        // Path has components: search in parent, match against file name
        let search_dir = if parent.as_os_str().is_empty() {
            if path.is_absolute() {
                PathBuf::from("/")
            } else {
                working_dir.to_path_buf()
            }
        } else if path.is_absolute() || expanded.starts_with('/') {
            parent.to_path_buf()
        } else {
            working_dir.join(parent)
        };

        // Note: path.file_name() returns None for "." and ".." special paths.
        // In that case, treat the entire expanded string as the prefix to match
        // hidden files (e.g., "." matches ".hidden", ".." matches "..foo").
        let prefix = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| expanded.clone());

        (search_dir, prefix)
    } else {
        // Just a prefix (e.g., "Doc")
        (working_dir.to_path_buf(), expanded)
    }
}

/// Expand tilde to home directory. Thin wrapper around the canonical
/// helper in [`crate::app::workspace`] kept for backwards-compatible
/// `&str -> String` ergonomics inside this module's matchers and tests.
fn expand_tilde(path: &str) -> String {
    expand_tilde_string(path)
}

/// Build the display name and insert text for a completion.
fn build_completion_text(
    name: &str,
    entry_type: &PathEntryType,
    original_input: &str,
    _prefix: &str,
) -> (String, String) {
    // Display name: append "/" for directories
    let display_name = match entry_type {
        PathEntryType::Directory => format!("{}/", name),
        _ => name.to_string(),
    };

    // Insert text: replace the last component of the original input with the full name
    let insert_text = if original_input.is_empty() {
        display_name.clone()
    } else if original_input.ends_with('/') || original_input.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{}{}", original_input, display_name)
    } else if let Some(last_sep_pos) = original_input.rfind(['/', std::path::MAIN_SEPARATOR]) {
        format!("{}{}", &original_input[..=last_sep_pos], display_name)
    } else {
        display_name.clone()
    };

    (display_name, insert_text)
}

