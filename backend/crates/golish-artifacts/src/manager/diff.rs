//! Diff and helper utilities for artifact management.

use anyhow::Result;

pub(super) fn continue_or_error<T>(e: anyhow::Error) -> Result<T> {
    Err(e)
}

/// Generate a simple unified diff between two strings
pub(crate) fn generate_simple_diff(old: &str, new: &str) -> String {
    use std::fmt::Write;

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff = String::new();
    let _ = writeln!(diff, "--- current");
    let _ = writeln!(diff, "+++ proposed");

    let max_len = old_lines.len().max(new_lines.len());

    for i in 0..max_len {
        let old_line = old_lines.get(i).copied();
        let new_line = new_lines.get(i).copied();

        match (old_line, new_line) {
            (Some(o), Some(n)) if o == n => {
                let _ = writeln!(diff, " {}", o);
            }
            (Some(o), Some(n)) => {
                let _ = writeln!(diff, "-{}", o);
                let _ = writeln!(diff, "+{}", n);
            }
            (Some(o), None) => {
                let _ = writeln!(diff, "-{}", o);
            }
            (None, Some(n)) => {
                let _ = writeln!(diff, "+{}", n);
            }
            (None, None) => {}
        }
    }

    diff
}
