//! Format/parse helpers for git format-patch style content.

use chrono::Utc;

/// Format diff content as a git format-patch style patch
pub(super) fn format_patch_content(message: &str, diff: &str) -> String {
    let now = Utc::now();
    let date = now.format("%a, %d %b %Y %H:%M:%S %z").to_string();

    let mut lines = message.lines();
    let subject = lines.next().unwrap_or("changes");
    let body: String = lines.collect::<Vec<_>>().join("\n");

    let file_stats = parse_diff_stats(diff);

    let mut patch = String::new();

    patch.push_str("From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001\n");
    patch.push_str("From: Golish Agent <agent@golish.dev>\n");
    patch.push_str(&format!("Date: {}\n", date));
    patch.push_str(&format!("Subject: [PATCH] {}\n", subject));
    patch.push('\n');

    if !body.trim().is_empty() {
        patch.push_str(body.trim());
        patch.push('\n');
    }

    patch.push_str("---\n");

    let mut total_insertions = 0;
    let mut total_deletions = 0;
    for (file, insertions, deletions) in &file_stats {
        total_insertions += insertions;
        total_deletions += deletions;
        let changes = insertions + deletions;
        let plus_signs = "+".repeat((*insertions).min(20) as usize);
        let minus_signs = "-".repeat((*deletions).min(20) as usize);
        patch.push_str(&format!(
            " {} | {} {}{}\n",
            file, changes, plus_signs, minus_signs
        ));
    }

    let files_changed = file_stats.len();
    patch.push_str(&format!(
        " {} file{} changed, {} insertion{}(+), {} deletion{}(-)\n",
        files_changed,
        if files_changed == 1 { "" } else { "s" },
        total_insertions,
        if total_insertions == 1 { "" } else { "s" },
        total_deletions,
        if total_deletions == 1 { "" } else { "s" }
    ));
    patch.push('\n');

    patch.push_str(diff);

    patch.push_str("--\n");
    patch.push_str("2.39.0\n");

    patch
}

/// Parse diff content to extract per-file statistics (file, insertions, deletions)
fn parse_diff_stats(diff: &str) -> Vec<(String, u32, u32)> {
    let mut stats = Vec::new();
    let mut current_file: Option<String> = None;
    let mut insertions = 0u32;
    let mut deletions = 0u32;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            if let Some(file) = current_file.take() {
                stats.push((file, insertions, deletions));
            }
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_file = Some(b_part.to_string());
            }
            insertions = 0;
            deletions = 0;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            insertions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }

    if let Some(file) = current_file {
        stats.push((file, insertions, deletions));
    }

    stats
}

/// Extract commit message from patch content
pub(super) fn extract_message_from_patch(patch_content: &str) -> String {
    let mut in_message = false;
    let mut message_lines = Vec::new();

    for line in patch_content.lines() {
        if line.starts_with("Subject: ") {
            let subject = line
                .strip_prefix("Subject: ")
                .unwrap_or("")
                .strip_prefix("[PATCH] ")
                .or_else(|| line.strip_prefix("Subject: [PATCH 1/1] "))
                .unwrap_or(line.strip_prefix("Subject: ").unwrap_or(""));
            message_lines.push(subject.to_string());
            in_message = true;
            continue;
        }

        if in_message {
            if line == "---" {
                break;
            }
            message_lines.push(line.to_string());
        }
    }

    message_lines.join("\n").trim().to_string()
}

/// Extract the diff portion from a patch file
///
/// The diff starts after the diffstat section (after "---" and file stats)
/// and ends before the "--" footer line.
pub(super) fn extract_diff_from_patch(patch_content: &str) -> String {
    let mut in_diff = false;
    let mut diff_lines = Vec::new();
    let mut found_separator = false;

    for line in patch_content.lines() {
        if line == "---" && !found_separator {
            found_separator = true;
            continue;
        }

        if found_separator && line.starts_with("diff --git") {
            in_diff = true;
        }

        if line == "--" {
            break;
        }

        if in_diff {
            diff_lines.push(line);
        }
    }

    diff_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_patch_content() {
        let message = "feat(auth): add authentication\n\nAdds JWT-based auth.";
        let diff = "diff --git a/src/auth.rs b/src/auth.rs\n+pub fn auth() {}";

        let patch = format_patch_content(message, diff);

        assert!(patch.contains("From: Golish Agent"));
        assert!(patch.contains("Subject: [PATCH] feat(auth): add authentication"));
        assert!(patch.contains("Adds JWT-based auth."));
        assert!(patch.contains("diff --git"));
    }
}
