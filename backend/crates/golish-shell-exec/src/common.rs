//! Shared constants and helpers used by both [`super::streaming`] and
//! [`super::tool`].

use std::path::{Path, PathBuf};

/// Default timeout in seconds for shell commands.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum output size in bytes (10 MB).
pub(crate) const MAX_OUTPUT_SIZE: usize = 10 * 1024 * 1024;

/// Resolve working directory relative to the workspace root.
///
/// - `Some(path)` absolute → returned as-is.
/// - `Some(path)` relative → joined onto `workspace`.
/// - `None` → workspace root.
pub(crate) fn resolve_cwd(cwd: Option<&str>, workspace: &Path) -> PathBuf {
    match cwd {
        Some(dir) => {
            let path = Path::new(dir);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            }
        }
        None => workspace.to_path_buf(),
    }
}

/// Truncate output to a maximum size, preferring the end of the output.
///
/// Output longer than `max_size` is replaced with a header followed by the
/// last `max_size` bytes, since the most recent output is usually more
/// relevant.
pub(crate) fn truncate_output(buf: &[u8], max_size: usize) -> String {
    let content = String::from_utf8_lossy(buf);

    if content.len() <= max_size {
        return content.to_string();
    }

    let truncated_start = content.len() - max_size;
    format!(
        "[Output truncated, showing last {} bytes]\n{}",
        max_size,
        &content[truncated_start..]
    )
}
