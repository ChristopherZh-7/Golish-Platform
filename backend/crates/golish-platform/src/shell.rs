//! Cross-platform shell invocation.
//!
//! Existing crates inside this workspace had many ad-hoc spots that
//! hard-coded `Command::new("sh").arg("-c")` or `Command::new("which")`.
//! Both of those are Unix-only and break on Windows.
//!
//! This module centralises those two operations so that callers can
//! write a single line that works on macOS, Linux, and Windows.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::detect::Platform;

const PATH_MARKER: &str = "__GOLISH_PATH_MARKER__=";

/// Supported interactive shell types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    Zsh,
    Bash,
    Fish,
    Sh,
    PowerShell,
    Cmd,
    Unknown,
}

impl ShellType {
    /// Detect shell type from an executable path.
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let file_name = path
            .as_ref()
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();

        match file_name.as_str() {
            "zsh" => Self::Zsh,
            "bash" => Self::Bash,
            "fish" => Self::Fish,
            "sh" => Self::Sh,
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Unknown,
        }
    }

    /// Login/startup args for interactive PTY shells.
    pub fn login_args(self) -> Vec<&'static str> {
        match self {
            Self::Zsh | Self::Bash | Self::Fish => vec!["-l"],
            Self::PowerShell => vec!["-NoLogo"],
            Self::Sh | Self::Cmd | Self::Unknown => vec![],
        }
    }

    /// Shell argument used to execute one command string.
    pub const fn command_arg(self) -> &'static str {
        match self {
            Self::Cmd => "/C",
            Self::PowerShell => "-Command",
            _ => "-c",
        }
    }
}

/// Shell executable + detected type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInfo {
    pub path: PathBuf,
    shell_type: ShellType,
}

impl ShellInfo {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let shell_type = ShellType::from_path(&path);
        Self { path, shell_type }
    }

    pub fn shell_type(&self) -> ShellType {
        self.shell_type
    }

    pub fn login_args(&self) -> Vec<&'static str> {
        self.shell_type.login_args()
    }
}

/// Detect an interactive shell from a user override, environment value, or
/// platform default.
pub fn detect_shell(shell_override: Option<&str>, shell_env: Option<&str>) -> ShellInfo {
    if let Some(shell) = shell_override {
        return ShellInfo::new(shell);
    }

    if let Some(shell) = shell_env {
        return ShellInfo::new(shell);
    }

    ShellInfo::new(default_interactive_shell())
}

/// Default interactive shell path/name for the current platform.
pub fn default_interactive_shell() -> PathBuf {
    if Platform::current().is_windows() {
        find_windows_shell()
    } else {
        PathBuf::from("/bin/sh")
    }
}

/// Shell used to resolve a user's login PATH.
///
/// On macOS, GUI-launched apps often miss the shell-initialized PATH, so zsh
/// is preferred when `$SHELL` is not set.
pub fn login_shell_for_path_resolution(shell_env: Option<&str>) -> Option<PathBuf> {
    if Platform::current().is_windows() {
        return None;
    }
    if let Some(shell) = shell_env.filter(|s| !s.trim().is_empty()) {
        return Some(PathBuf::from(shell));
    }
    if Platform::current().is_macos() {
        Some(PathBuf::from("/bin/zsh"))
    } else {
        Some(PathBuf::from("/bin/sh"))
    }
}

/// Resolve PATH by spawning a login shell and echoing a marker-delimited value.
pub fn resolve_login_shell_path() -> Option<String> {
    let shell = login_shell_for_path_resolution(std::env::var("SHELL").ok().as_deref())?;
    let output = std::process::Command::new(&shell)
        .args(["-lic", &format!("echo {PATH_MARKER}$PATH")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_login_shell_path_output(&output.stdout)
}

/// Extract the PATH value from login shell output.
pub fn parse_login_shell_path_output(stdout: &[u8]) -> Option<String> {
    let stdout = String::from_utf8_lossy(stdout);
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix(PATH_MARKER) {
            let path = path.trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

/// Returns `(program, first_arg)` for the platform's default shell.
///
/// - Windows: `("cmd", "/C")` (cmd is always available; PowerShell
///   would require a different escaping discipline).
/// - Unix: `("/bin/sh", "-c")`.
pub fn default_shell_invocation() -> (&'static str, &'static str) {
    if Platform::current().is_windows() {
        ("cmd", "/C")
    } else {
        ("/bin/sh", "-c")
    }
}

/// Build a synchronous [`std::process::Command`] that will run
/// `command_line` through the platform's default shell.
pub fn build_shell_command<S: AsRef<OsStr>>(command_line: S) -> std::process::Command {
    let (program, first_arg) = default_shell_invocation();
    let mut cmd = std::process::Command::new(program);
    cmd.arg(first_arg).arg(command_line);
    cmd
}

/// Build an async [`tokio::process::Command`] that will run
/// `command_line` through the platform's default shell.
pub fn build_tokio_shell_command<S: AsRef<OsStr>>(command_line: S) -> tokio::process::Command {
    let (program, first_arg) = default_shell_invocation();
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg(first_arg).arg(command_line);
    cmd
}

/// Returns the executable lookup program for the current platform —
/// `"where"` on Windows, `"which"` on Unix.
pub fn lookup_program() -> &'static str {
    if Platform::current().is_windows() {
        "where"
    } else {
        "which"
    }
}

/// Best available Windows interactive shell.
#[cfg(target_os = "windows")]
fn find_windows_shell() -> PathBuf {
    for candidate in ["pwsh.exe", "powershell.exe"] {
        if let Some(path) = which_executable(candidate) {
            return path;
        }
    }

    std::env::var("COMSPEC")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("cmd.exe"))
}

#[cfg(not(target_os = "windows"))]
fn find_windows_shell() -> PathBuf {
    PathBuf::from("/bin/sh")
}

/// Resolve a command name to an absolute executable path using the
/// platform's standard lookup tool.
pub fn which_executable(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let output = std::process::Command::new(lookup_program())
        .arg(name)
        .output()
        .ok()?;
    parse_lookup_output(output.status.success(), &output.stdout)
}

/// Async variant of [`which_executable`].
pub async fn which_executable_async(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let output = tokio::process::Command::new(lookup_program())
        .arg(name)
        .output()
        .await
        .ok()?;
    parse_lookup_output(output.status.success(), &output.stdout)
}

fn parse_lookup_output(success: bool, stdout: &[u8]) -> Option<PathBuf> {
    if !success {
        return None;
    }
    let stdout = String::from_utf8_lossy(stdout);
    let line = stdout.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    Some(PathBuf::from(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_invocation_is_platform_appropriate() {
        let (program, first_arg) = default_shell_invocation();
        if cfg!(target_os = "windows") {
            assert_eq!(program, "cmd");
            assert_eq!(first_arg, "/C");
        } else {
            assert_eq!(program, "/bin/sh");
            assert_eq!(first_arg, "-c");
        }
    }

    #[test]
    fn lookup_program_matches_platform() {
        if cfg!(target_os = "windows") {
            assert_eq!(lookup_program(), "where");
        } else {
            assert_eq!(lookup_program(), "which");
        }
    }

    #[test]
    fn build_shell_command_smoke() {
        let _ = build_shell_command("echo hi");
    }

    #[test]
    fn build_tokio_shell_command_smoke() {
        let _ = build_tokio_shell_command("echo hi");
    }

    #[test]
    fn shell_type_detects_interactive_shells() {
        assert_eq!(ShellType::from_path("/bin/zsh"), ShellType::Zsh);
        assert_eq!(ShellType::from_path("/usr/bin/bash"), ShellType::Bash);
        assert_eq!(
            ShellType::from_path("/opt/homebrew/bin/fish"),
            ShellType::Fish
        );
        assert_eq!(ShellType::from_path("/bin/sh"), ShellType::Sh);
        assert_eq!(ShellType::from_path("pwsh.exe"), ShellType::PowerShell);
        assert_eq!(ShellType::from_path("cmd.exe"), ShellType::Cmd);
        assert_eq!(ShellType::from_path("/bin/unknown"), ShellType::Unknown);
    }

    #[test]
    fn shell_type_exposes_login_and_command_args() {
        assert_eq!(ShellType::Zsh.login_args(), vec!["-l"]);
        assert_eq!(ShellType::Bash.login_args(), vec!["-l"]);
        assert_eq!(ShellType::Fish.login_args(), vec!["-l"]);
        assert_eq!(ShellType::PowerShell.login_args(), vec!["-NoLogo"]);
        assert_eq!(ShellType::Cmd.login_args(), Vec::<&str>::new());

        assert_eq!(ShellType::Cmd.command_arg(), "/C");
        assert_eq!(ShellType::PowerShell.command_arg(), "-Command");
        assert_eq!(ShellType::Bash.command_arg(), "-c");
    }

    #[test]
    fn detect_shell_prefers_override_then_env_then_platform_default() {
        let override_info = detect_shell(Some("/usr/local/bin/fish"), Some("/bin/zsh"));
        assert_eq!(override_info.path, PathBuf::from("/usr/local/bin/fish"));
        assert_eq!(override_info.shell_type(), ShellType::Fish);

        let env_info = detect_shell(None, Some("/bin/zsh"));
        assert_eq!(env_info.path, PathBuf::from("/bin/zsh"));
        assert_eq!(env_info.shell_type(), ShellType::Zsh);

        let fallback = detect_shell(None, None);
        if cfg!(windows) {
            assert!(matches!(
                fallback.shell_type(),
                ShellType::PowerShell | ShellType::Cmd | ShellType::Unknown
            ));
        } else {
            assert_eq!(fallback.path, PathBuf::from("/bin/sh"));
            assert_eq!(fallback.shell_type(), ShellType::Sh);
        }
    }

    #[test]
    fn login_shell_path_resolution_defaults_to_platform_shell() {
        let shell = login_shell_for_path_resolution(None);
        if Platform::current().is_windows() {
            assert!(shell.is_none());
        } else if Platform::current().is_macos() {
            assert_eq!(shell, Some(PathBuf::from("/bin/zsh")));
        } else {
            assert_eq!(shell, Some(PathBuf::from("/bin/sh")));
        }

        assert_eq!(
            login_shell_for_path_resolution(Some("/custom/shell")),
            Some(PathBuf::from("/custom/shell"))
        );
    }

    #[test]
    fn parses_login_shell_path_marker() {
        let output = b"noise\n__GOLISH_PATH_MARKER__=/usr/local/bin:/usr/bin\n";
        assert_eq!(
            parse_login_shell_path_output(output),
            Some("/usr/local/bin:/usr/bin".to_string())
        );
        assert_eq!(parse_login_shell_path_output(b"no marker"), None);
    }
}
