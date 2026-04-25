//! Shell detection and configuration for multi-shell support.
//!
//! Provides shell-type detection from paths and settings (zsh, bash,
//! fish), plus an automatic shell-integration injector that emits OSC 133
//! sequences without requiring users to edit their rc files.
//!
//! ## Layout
//!
//! - [`scripts`]: embedded shell-script blobs (zsh + bash integration,
//!   ZDOTDIR-wrapper `.zshrc`).
//! - [`integration`]: [`ShellIntegration`] — installs the scripts onto
//!   disk + computes the env vars / shell args needed to inject them.
//!
//! Detection types ([`ShellType`], [`ShellInfo`], [`detect_shell`]) live
//! in this `mod.rs`.

use std::path::{Path, PathBuf};

use golish_settings::schema::TerminalSettings;

mod integration;
mod scripts;

#[cfg(test)]
mod tests;

pub use integration::ShellIntegration;

/// Supported shell types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    Zsh,
    Bash,
    Fish,
    Unknown,
}

impl ShellType {
    /// Get login shell arguments for this shell type.
    pub fn login_args(&self) -> Vec<&'static str> {
        match self {
            ShellType::Zsh | ShellType::Bash | ShellType::Fish => vec!["-l"],
            ShellType::Unknown => vec![],
        }
    }
}

/// Shell detection and configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInfo {
    /// Path to the shell executable.
    pub path: PathBuf,
    shell_type: ShellType,
}

impl ShellInfo {
    /// Create a new [`ShellInfo`] from a shell path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let shell_type = Self::detect_type(&path);
        Self { path, shell_type }
    }

    /// Get the detected shell type.
    pub fn shell_type(&self) -> ShellType {
        self.shell_type
    }

    /// Get login shell arguments.
    pub fn login_args(&self) -> Vec<&'static str> {
        self.shell_type.login_args()
    }

    /// Detect shell type from path.
    fn detect_type(path: &Path) -> ShellType {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        match file_name {
            "zsh" => ShellType::Zsh,
            "bash" => ShellType::Bash,
            "fish" => ShellType::Fish,
            _ => ShellType::Unknown,
        }
    }
}

/// Detect shell from settings or environment.
///
/// Priority:
/// 1. `settings.terminal.shell` (user override)
/// 2. `shell_env` (`$SHELL` environment variable)
/// 3. Fallback to `/bin/sh`
pub fn detect_shell(settings: Option<&TerminalSettings>, shell_env: Option<&str>) -> ShellInfo {
    if let Some(settings) = settings {
        if let Some(ref shell) = settings.shell {
            return ShellInfo::new(shell);
        }
    }

    if let Some(shell) = shell_env {
        return ShellInfo::new(shell);
    }

    ShellInfo::new("/bin/sh")
}
