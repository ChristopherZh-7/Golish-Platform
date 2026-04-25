//! Diff generation helpers (string-based and git-backed).

use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

/// Generate a diff for a new (untracked) file
async fn generate_new_file_diff(git_root: &Path, file_path: &str) -> Result<String> {
    let full_path = git_root.join(file_path);

    let content = match tokio::fs::read_to_string(&full_path).await {
        Ok(c) => c,
        Err(_) => return Ok(String::new()),
    };

    let mut diff = String::new();
    diff.push_str(&format!("diff --git a/{} b/{}\n", file_path, file_path));
    diff.push_str("new file mode 100644\n");
    diff.push_str("index 0000000..0000000\n");
    diff.push_str("--- /dev/null\n");
    diff.push_str(&format!("+++ b/{}\n", file_path));

    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    if line_count > 0 {
        diff.push_str(&format!("@@ -0,0 +1,{} @@\n", line_count));
        for line in lines {
            diff.push_str(&format!("+{}\n", line));
        }
    }

    Ok(diff)
}

/// Generate diff for a single file (comparing to HEAD or as new file)
pub(super) async fn generate_diff_for_single_file(git_root: &Path, file: &Path) -> Result<String> {
    let file_str = match file.to_str() {
        Some(s) => s,
        None => return Ok(String::new()),
    };

    let is_tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", file_str])
        .current_dir(git_root)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_tracked {
        let output = Command::new("git")
            .args(["diff", "HEAD", "--", file_str])
            .current_dir(git_root)
            .output()
            .await
            .context("Failed to run git diff")?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        generate_new_file_diff(git_root, file_str).await
    }
}

/// Generate a unified diff from two string contents
pub(super) fn generate_diff_from_strings(
    file_path: &str,
    old_content: &str,
    new_content: &str,
) -> String {
    use similar::TextDiff;
    use std::fmt::Write;

    let text_diff = TextDiff::from_lines(old_content, new_content);

    if text_diff.ratio() == 1.0 {
        return String::new();
    }

    let mut diff = String::new();

    diff.push_str(&format!("diff --git a/{} b/{}\n", file_path, file_path));
    diff.push_str("index 0000000..0000000 100644\n");
    diff.push_str(&format!("--- a/{}\n", file_path));
    diff.push_str(&format!("+++ b/{}\n", file_path));

    for hunk in text_diff.unified_diff().context_radius(3).iter_hunks() {
        writeln!(diff, "{}", hunk.header()).unwrap();
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => "-",
                similar::ChangeTag::Insert => "+",
                similar::ChangeTag::Equal => " ",
            };
            write!(diff, "{}{}", sign, change.value()).unwrap();
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_diff_from_strings() {
        let old_content = "line 1\nline 2\nline 3\n";
        let new_content = "line 1\nline 2 modified\nline 3\n";

        let diff = generate_diff_from_strings("test.txt", old_content, new_content);

        assert!(diff.contains("diff --git a/test.txt b/test.txt"));
        assert!(diff.contains("--- a/test.txt"));
        assert!(diff.contains("+++ b/test.txt"));

        assert!(diff.contains("-line 2"));
        assert!(diff.contains("+line 2 modified"));

        assert!(diff.contains(" line 1"));
        assert!(diff.contains(" line 3"));
    }

    #[test]
    fn test_generate_diff_from_strings_no_changes() {
        let content = "line 1\nline 2\nline 3\n";

        let diff = generate_diff_from_strings("test.txt", content, content);

        assert!(diff.is_empty());
    }

    #[test]
    fn test_generate_diff_from_strings_new_file() {
        let old_content = "";
        let new_content = "line 1\nline 2\nline 3\n";

        let diff = generate_diff_from_strings("test.txt", old_content, new_content);

        assert!(diff.contains("diff --git a/test.txt b/test.txt"));

        assert!(diff.contains("+line 1"));
        assert!(diff.contains("+line 2"));
        assert!(diff.contains("+line 3"));
    }
}
