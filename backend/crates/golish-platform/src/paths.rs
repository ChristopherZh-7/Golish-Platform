//! Cross-platform path / extension constants.
//!
//! Lightweight wrappers around the `dirs` crate plus a few constants
//! commonly needed by Python / Conda integration.

use std::path::PathBuf;

use crate::detect::Platform;

/// Subdirectory under a Python venv / conda env where binaries live.
/// `bin` on Unix, `Scripts` on Windows.
pub const fn python_bin_dir() -> &'static str {
    if cfg!(target_os = "windows") {
        "Scripts"
    } else {
        "bin"
    }
}

/// Filename of the Python executable in a venv.
pub const fn python_exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "python.exe"
    } else {
        "python"
    }
}

/// Filename of the conda executable.
pub const fn conda_exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "conda.exe"
    } else {
        "conda"
    }
}

/// Subdirectory under a conda env where binaries live.
pub const fn conda_bin_dir() -> &'static str {
    python_bin_dir()
}

/// Per-app data directory.
///
/// - macOS: `~/Library/Application Support/<app_id>`
/// - Windows: `%LOCALAPPDATA%\<app_id>` (≈ `~/AppData/Local/<app_id>`)
/// - Linux: `~/.<app_id>` (legacy convention used by this project)
pub fn app_data_base(app_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let base = match Platform::current().kind {
        crate::detect::PlatformKind::MacOs => home
            .join("Library")
            .join("Application Support")
            .join(app_id),
        crate::detect::PlatformKind::Windows => home.join("AppData").join("Local").join(app_id),
        _ => home.join(format!(".{}", app_id)),
    };
    Some(base)
}

/// User cache directory.
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_bin_dir_matches_platform() {
        if cfg!(target_os = "windows") {
            assert_eq!(python_bin_dir(), "Scripts");
            assert_eq!(python_exe_name(), "python.exe");
            assert_eq!(conda_exe_name(), "conda.exe");
        } else {
            assert_eq!(python_bin_dir(), "bin");
            assert_eq!(python_exe_name(), "python");
            assert_eq!(conda_exe_name(), "conda");
        }
    }

    #[test]
    fn app_data_base_resolves() {
        let p = app_data_base("golish-platform-test");
        assert!(p.is_some());
    }
}
