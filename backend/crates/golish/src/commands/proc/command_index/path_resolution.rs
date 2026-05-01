//! PATH discovery and input-tokenisation helpers used by [`super::CommandIndex`].
//!
//! Lives in its own module so the security-relevant string parsing
//! (`contains_shell_operator`) and the platform-specific PATH probes
//! (`resolve_shell_path` / `is_executable`) can be unit-tested independently
//! of the index struct itself.

/// Check if a file is executable (Unix).
#[cfg(unix)]
pub(super) fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
pub(super) fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// Resolve the user's full shell PATH by spawning a login shell.
///
/// On macOS/Linux, GUI apps don't inherit PATH entries added by shell rc
/// files (e.g. `~/.local/bin` from `.zshrc`/`.bashrc`).
#[cfg(unix)]
pub(super) fn resolve_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".to_string()
        } else {
            "/bin/sh".to_string()
        }
    });

    let output = std::process::Command::new(&shell)
        .args(["-lic", "echo __QBIT_CMD_IDX_PATH__=$PATH"])
        .output()
        .ok()?;

    if !output.status.success() {
        tracing::warn!(
            "[command-index] Login shell exited with status {} while resolving PATH",
            output.status
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("__QBIT_CMD_IDX_PATH__=") {
            let path = path.trim().to_string();
            tracing::debug!("[command-index] Resolved shell PATH: {}", path);
            return Some(path);
        }
    }

    tracing::warn!("[command-index] Failed to extract PATH from login shell output");
    None
}

#[cfg(not(unix))]
pub(super) fn resolve_shell_path() -> Option<String> {
    None
}

/// Extract the first whitespace-delimited token from input.
pub(super) fn first_token(input: &str) -> &str {
    input.split_whitespace().next().unwrap_or("")
}

/// Check if input contains common shell operators.
///
/// Operators inside single or double-quoted strings are ignored using a
/// simple state machine — sufficient for routing decisions, but **not** a
/// full shell parser.
pub(super) fn contains_shell_operator(input: &str) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();

    for i in 0..len {
        let c = chars[i];
        match c {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            _ if in_single_quote || in_double_quote => continue,
            '|' => return true,
            ';' => return true,
            '>' => return true,
            '<' => return true,
            '&' if i + 1 < len && chars[i + 1] == '&' => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_token_handles_empty_and_whitespace() {
        assert_eq!(first_token(""), "");
        assert_eq!(first_token("   "), "");
        assert_eq!(first_token("ls"), "ls");
        assert_eq!(first_token("  ls -la  "), "ls");
    }

    #[test]
    fn detects_pipe_redirect_logical_semicolon() {
        assert!(contains_shell_operator("a | b"));
        assert!(contains_shell_operator("a > b"));
        assert!(contains_shell_operator("a < b"));
        assert!(contains_shell_operator("a && b"));
        assert!(contains_shell_operator("a; b"));
    }

    #[test]
    fn ignores_operators_inside_quotes() {
        assert!(!contains_shell_operator("echo \"a | b\""));
        assert!(!contains_shell_operator("echo 'a > b'"));
        assert!(!contains_shell_operator("echo \"hello && world\""));
    }

    #[test]
    fn lone_ampersand_is_not_operator() {
        assert!(!contains_shell_operator("a & b"));
    }
}
