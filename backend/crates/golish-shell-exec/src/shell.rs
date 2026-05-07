//! Shell detection and rc-file-aware command wrapping.
//!
//! [`ShellType`] handles the user's preferred login shell (zsh / bash /
//! fish / sh / PowerShell / cmd) so we can `source` the right rc file
//! before running each command — that's how PATH, aliases and shell
//! functions become visible to spawned children.
//! [`get_shell_config`] is the single resolution point used by both the
//! streaming and tool execution paths.

use std::path::{Path, PathBuf};

pub(crate) use golish_platform::shell::ShellType;

/// Get the rc file path for this shell type.
pub(crate) fn rc_file(shell_type: ShellType, home: &Path) -> Option<PathBuf> {
    match shell_type {
        ShellType::Zsh => Some(home.join(".zshrc")),
        ShellType::Bash => {
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
        ShellType::Fish => Some(home.join(".config").join("fish").join("config.fish")),
        ShellType::PowerShell => {
            let profile = home
                .join("Documents")
                .join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1");
            if profile.exists() {
                Some(profile)
            } else {
                let profile_legacy = home
                    .join("Documents")
                    .join("WindowsPowerShell")
                    .join("Microsoft.PowerShell_profile.ps1");
                if profile_legacy.exists() {
                    Some(profile_legacy)
                } else {
                    None
                }
            }
        }
        ShellType::Sh | ShellType::Cmd | ShellType::Unknown => None,
    }
}

/// Build the command to execute with proper PATH loaded.
pub(crate) fn build_command(
    shell_type: ShellType,
    shell_path: &Path,
    user_command: &str,
    home: &Path,
) -> (String, String) {
    let shell_str = shell_path.to_string_lossy().to_string();
    match shell_type {
        ShellType::Zsh => {
            let rc_file = home.join(".zshrc");
            if rc_file.exists() {
                let wrapped = format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                (shell_str, wrapped)
            } else {
                (shell_str, user_command.to_string())
            }
        }
        ShellType::Bash => {
            if let Some(rc_file) = rc_file(shell_type, home) {
                let wrapped = format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                (shell_str, wrapped)
            } else {
                (shell_str, user_command.to_string())
            }
        }
        ShellType::Fish => {
            let rc_file = home.join(".config").join("fish").join("config.fish");
            if rc_file.exists() {
                let wrapped = format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                (shell_str, wrapped)
            } else {
                (shell_str, user_command.to_string())
            }
        }
        ShellType::Sh | ShellType::Unknown => (shell_str, user_command.to_string()),
        ShellType::PowerShell => {
            if let Some(profile) = rc_file(shell_type, home) {
                let wrapped = format!(
                    ". '{}' -ErrorAction SilentlyContinue; {}",
                    profile.display(),
                    user_command
                );
                (shell_str, wrapped)
            } else {
                (shell_str, user_command.to_string())
            }
        }
        ShellType::Cmd => (shell_str, user_command.to_string()),
    }
}

/// Get shell configuration.
///
/// Shell resolution order:
/// 1. `shell_override` parameter (from `settings.toml` `terminal.shell`).
/// 2. `$SHELL` environment variable (Unix) or `$COMSPEC` / PowerShell (Windows).
/// 3. Fall back to `/bin/sh` (Unix) or `powershell.exe` (Windows).
///
/// Returns `(shell_path, shell_type, home_dir)`.
pub(crate) fn get_shell_config(shell_override: Option<&str>) -> (PathBuf, ShellType, PathBuf) {
    let shell_env = std::env::var("SHELL").ok();
    let shell_info = golish_platform::shell::detect_shell(shell_override, shell_env.as_deref());

    let home = dirs::home_dir().unwrap_or_else(|| {
        if golish_platform::Platform::current().is_windows() {
            PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string()))
        } else {
            PathBuf::from("/")
        }
    });

    let shell_type = shell_info.shell_type();
    (shell_info.path, shell_type, home)
}
