//! Shell detection and rc-file-aware command wrapping.
//!
//! [`ShellType`] handles the user's preferred login shell (zsh / bash /
//! fish / sh) so we can `source` the right rc file before running each
//! command — that's how PATH, aliases and shell functions become visible
//! to spawned children. [`get_shell_config`] is the single resolution
//! point used by both the streaming and tool execution paths.

use std::path::{Path, PathBuf};

/// Supported shell types for PATH inheritance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellType {
    Zsh,
    Bash,
    Fish,
    Sh,
}

impl ShellType {
    /// Detect shell type from path.
    pub(crate) fn from_path(path: &Path) -> Self {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match file_name {
            "zsh" => ShellType::Zsh,
            "bash" => ShellType::Bash,
            "fish" => ShellType::Fish,
            _ => ShellType::Sh,
        }
    }

    /// Get the rc file path for this shell type.
    pub(crate) fn rc_file(&self, home: &Path) -> Option<PathBuf> {
        match self {
            ShellType::Zsh => Some(home.join(".zshrc")),
            ShellType::Bash => {
                // Bash uses .bashrc for interactive non-login shells.
                // Check .bashrc first, then .bash_profile.
                let bashrc = home.join(".bashrc");
                if bashrc.exists() {
                    Some(bashrc)
                } else {
                    let bash_profile = home.join(".bash_profile");
                    if bash_profile.exists() {
                        Some(bash_profile)
                    } else {
                        None
                    }
                }
            }
            ShellType::Fish => Some(home.join(".config/fish/config.fish")),
            ShellType::Sh => None,
        }
    }

    /// Build the command to execute with proper PATH loaded.
    ///
    /// Strategy:
    /// 1. zsh / bash: Source the rc file explicitly before running the command.
    /// 2. fish: Use `fish -c` with a `source` directive.
    /// 3. sh: Just run directly (no rc file).
    pub(crate) fn build_command(
        &self,
        shell_path: &Path,
        user_command: &str,
        home: &Path,
    ) -> (String, String) {
        match self {
            ShellType::Zsh => {
                let rc_file = home.join(".zshrc");
                if rc_file.exists() {
                    // Source .zshrc then run the command. `emulate sh -c` would
                    // avoid issues with zsh-specific syntax in sourced files,
                    // but that breaks when users have explicit zsh syntax in
                    // their command — so we just suppress source errors.
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_path.to_string_lossy().to_string(), wrapped)
                } else {
                    (
                        shell_path.to_string_lossy().to_string(),
                        user_command.to_string(),
                    )
                }
            }
            ShellType::Bash => {
                if let Some(rc_file) = self.rc_file(home) {
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_path.to_string_lossy().to_string(), wrapped)
                } else {
                    (
                        shell_path.to_string_lossy().to_string(),
                        user_command.to_string(),
                    )
                }
            }
            ShellType::Fish => {
                let rc_file = home.join(".config/fish/config.fish");
                if rc_file.exists() {
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_path.to_string_lossy().to_string(), wrapped)
                } else {
                    (
                        shell_path.to_string_lossy().to_string(),
                        user_command.to_string(),
                    )
                }
            }
            ShellType::Sh => {
                // For sh, just run the command directly.
                ("/bin/sh".to_string(), user_command.to_string())
            }
        }
    }
}

/// Get shell configuration.
///
/// Shell resolution order:
/// 1. `shell_override` parameter (from `settings.toml` `terminal.shell`).
/// 2. `$SHELL` environment variable.
/// 3. Fall back to `/bin/sh`.
///
/// Returns `(shell_path, shell_type, home_dir)`.
pub(crate) fn get_shell_config(shell_override: Option<&str>) -> (PathBuf, ShellType, PathBuf) {
    let shell_path = shell_override
        .map(PathBuf::from)
        .or_else(|| std::env::var("SHELL").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));

    let shell_type = ShellType::from_path(&shell_path);

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));

    (shell_path, shell_type, home)
}
