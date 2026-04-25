use std::path::Path;
use golish_udiff::{ApplyResult, UdiffApplier, UdiffParser};

/// Process udiff output from the coder sub-agent, applying file changes.
/// Mutates `files_modified` in-place and returns the response with appended summary.
pub(crate) fn process_coder_udiff(
    response: &str,
    workspace: &Path,
    files_modified: &mut Vec<String>,
) -> String {
    let mut final_response = response.to_string();
    let diffs = UdiffParser::parse(response);

    if diffs.is_empty() {
        return final_response;
    }

    let mut applied_files = Vec::new();
    let mut errors = Vec::new();

    for diff in diffs {
        let file_path = workspace.join(&diff.file_path);

        if diff.is_new_file {
            if let Some(parent) = file_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(format!(
                        "Failed to create directories for {}: {}",
                        diff.file_path.display(),
                        e
                    ));
                    continue;
                }
            }

            let new_content: String = diff
                .hunks
                .iter()
                .flat_map(|h| h.new_lines.iter())
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            if let Err(e) = std::fs::write(&file_path, &new_content) {
                errors.push(format!(
                    "Failed to create {}: {}",
                    diff.file_path.display(),
                    e
                ));
            } else {
                let path_str = diff.file_path.display().to_string();
                applied_files.push(path_str.clone());
                if !files_modified.contains(&path_str) {
                    files_modified.push(path_str);
                }
                tracing::info!("[coder] Created new file: {}", diff.file_path.display());
            }
            continue;
        }

        match std::fs::read_to_string(&file_path) {
            Ok(content) => {
                match UdiffApplier::apply_hunks(&content, &diff.hunks) {
                    ApplyResult::Success { new_content } => {
                        if let Err(e) = std::fs::write(&file_path, new_content) {
                            errors.push(format!(
                                "Failed to write {}: {}",
                                diff.file_path.display(),
                                e
                            ));
                        } else {
                            let path_str = diff.file_path.display().to_string();
                            applied_files.push(path_str.clone());
                            if !files_modified.contains(&path_str) {
                                files_modified.push(path_str);
                            }
                        }
                    }
                    ApplyResult::PartialSuccess {
                        new_content,
                        applied,
                        failed,
                    } => {
                        let failed_hunks = failed.clone();
                        if let Err(e) = std::fs::write(&file_path, new_content) {
                            errors.push(format!(
                                "Failed to write {}: {}",
                                diff.file_path.display(),
                                e
                            ));
                        } else {
                            let path_str = diff.file_path.display().to_string();
                            applied_files.push(path_str.clone());
                            if !files_modified.contains(&path_str) {
                                files_modified.push(path_str);
                            }
                            for (idx, reason) in failed {
                                errors.push(format!(
                                    "Hunk {} in {}: {}",
                                    idx,
                                    diff.file_path.display(),
                                    reason
                                ));
                            }
                        }
                        tracing::info!(
                            "[coder] Partial success: applied hunks {:?}, failed: {:?}",
                            applied,
                            failed_hunks
                        );
                    }
                    ApplyResult::NoMatch {
                        hunk_idx,
                        suggestion,
                    } => {
                        errors.push(format!(
                            "{} (hunk {}): {}",
                            diff.file_path.display(),
                            hunk_idx,
                            suggestion
                        ));
                    }
                    ApplyResult::MultipleMatches { hunk_idx, count } => {
                        errors.push(format!(
                            "{} (hunk {}): Found {} matches, add more context",
                            diff.file_path.display(),
                            hunk_idx,
                            count
                        ));
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Cannot read {}: {}", diff.file_path.display(), e));
            }
        }
    }

    if !applied_files.is_empty() || !errors.is_empty() {
        final_response.push_str("\n\n---\n**Applied Changes:**\n");

        if !applied_files.is_empty() {
            final_response.push_str(&format!(
                "\nSuccessfully modified {} file(s):\n",
                applied_files.len()
            ));
            for file in &applied_files {
                final_response.push_str(&format!("- {}\n", file));
            }
        }

        if !errors.is_empty() {
            final_response.push_str(&format!("\n{} error(s) occurred:\n", errors.len()));
            for error in &errors {
                final_response.push_str(&format!("- {}\n", error));
            }
        }
    }

    final_response
}
