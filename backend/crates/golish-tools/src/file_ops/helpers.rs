//! Shared helpers for file_ops: binary-file detection and unified diff
//! formatting.

/// Check if a path is likely a binary file by examining the first bytes.
pub(super) fn is_binary_file(content: &[u8]) -> bool {
    // Check first 8000 bytes for null bytes (common indicator of binary)
    let check_len = content.len().min(8000);
    content[..check_len].contains(&0)
}

