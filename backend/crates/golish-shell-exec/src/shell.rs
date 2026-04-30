//! Shell detection and rc-file-aware command wrapping.
//!
//! [`ShellType`] handles the user's preferred login shell (zsh / bash /
//! fish / sh / PowerShell / cmd) so we can `source` the right rc file
//! before running each command — that's how PATH, aliases and shell
//! functions become visible to spawned children.
//! [`get_shell_config`] is the single resolution point used by both the
//! streaming and tool execution paths.

use std::path::{Path, PathBuf};

/// Supported shell types for PATH inheritance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellType {
    Zsh,
    Bash,
    Fish,
    Sh,
    PowerShell,
    Cmd,
}

impl ShellType {
    /// Detect shell type from path.
    pub(crate) fn from_path(path: &Path) -> Self {
        let file_name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        match file_name.as_str() {
            "zsh" => ShellType::Zsh,
            "bash" => ShellType::Bash,
            "fish" => ShellType::Fish,
            "pwsh" | "powershell" => ShellType::PowerShell,
            "cmd" => ShellType::Cmd,
            _ => {
                if cfg!(windows) {
                    ShellType::PowerShell
                } else {
                    ShellType::Sh
                }
            }
        }
    }

    /// Get the rc file path for this shell type.
    pub(crate) fn rc_file(&self, home: &Path) -> Option<PathBuf> {
        match self {
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
            ShellType::Sh | ShellType::Cmd => None,
        }
    }

    /// Build the command to execute with proper PATH loaded.
    ///
    /// Strategy:
    /// - zsh / bash: Source the rc file explicitly before running the command.
    /// - fish: Use `fish -c` with a `source` directive.
    /// - sh: Just run directly (no rc file).
    /// - PowerShell: Dot-source the profile, then run the command.
    /// - cmd: Run directly via `cmd /C`.
    pub(crate) fn build_command(
        &self,
        shell_path: &Path,
        user_command: &str,
        home: &Path,
    ) -> (String, String) {
        let shell_str = shell_path.to_string_lossy().to_string();
        match self {
            ShellType::Zsh => {
                let rc_file = home.join(".zshrc");
                if rc_file.exists() {
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_str, wrapped)
                } else {
                    (shell_str, user_command.to_string())
                }
            }
            ShellType::Bash => {
                if let Some(rc_file) = self.rc_file(home) {
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_str, wrapped)
                } else {
                    (shell_str, user_command.to_string())
                }
            }
            ShellType::Fish => {
                let rc_file = home.join(".config").join("fish").join("config.fish");
                if rc_file.exists() {
                    let wrapped =
                        format!("source {} 2>/dev/null; {}", rc_file.display(), user_command);
                    (shell_str, wrapped)
                } else {
                    (shell_str, user_command.to_string())
                }
            }
            ShellType::Sh => {
                let sh = if cfg!(windows) { shell_str } else { "/bin/sh".to_string() };
                (sh, user_command.to_string())
            }
            ShellType::PowerShell => {
                if let Some(profile) = self.rc_file(home) {
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
            ShellType::Cmd => {
                (shell_str, user_command.to_string())
            }
        }
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
    let shell_path = if let Some(s) = shell_override {
        PathBuf::from(s)
    } else if let Ok(s) = std::env::var("SHELL") {
        PathBuf::from(s)
    } else if cfg!(windows) {
        find_windows_shell()
    } else {
        PathBuf::from("/bin/sh")
    };

    let shell_type = ShellType::from_path(&shell_path);

    let home = dirs::home_dir().unwrap_or_else(|| {
        if cfg!(windows) {
            PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string()))
        } else {
            PathBuf::from("/")
        }
    });

    (shell_path, shell_type, home)
}

/// Find the best available shell on Windows.
/// Prefers pwsh (PowerShell 7+) over powershell.exe (Windows PowerShell 5.x) over cmd.exe.
#[cfg(windows)]
fn find_windows_shell() -> PathBuf {
    for candidate in &["pwsh.exe", "powershell.exe"] {
        if let Ok(output) = std::process::Command::new("where").arg(candidate).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = path.lines().next() {
                    return PathBuf::from(line.trim());
                }
            }
        }
    }
    std::env::var("COMSPEC")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("cmd.exe"))
}

#[cfg(not(windows))]
fn find_windows_shell() -> PathBuf {
    PathBuf::from("/bin/sh")
}
