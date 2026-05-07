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

use golish_settings::schema::TerminalSettings;

mod integration;
mod scripts;

#[cfg(test)]
mod tests;

pub use golish_platform::shell::{ShellInfo, ShellType};
pub use integration::ShellIntegration;

/// Detect shell from settings or environment.
///
/// Priority:
/// 1. `settings.terminal.shell` (user override)
/// 2. `shell_env` (`$SHELL` environment variable, Unix only)
/// 3. Fallback: `/bin/sh` (Unix) or `powershell.exe` (Windows)
pub fn detect_shell(settings: Option<&TerminalSettings>, shell_env: Option<&str>) -> ShellInfo {
    let shell_override = settings.and_then(|settings| settings.shell.as_deref());
    golish_platform::shell::detect_shell(shell_override, shell_env)
}
