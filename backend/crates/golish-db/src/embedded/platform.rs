//! Platform-specific helpers for embedded PostgreSQL.

use std::path::{Path, PathBuf};
use tracing::info;

use super::EmbeddedPg;

/// Search common system paths for pgvector extension files.
/// Returns paths to the shared library (.dylib/.so) and the control + SQL files.
pub(super) fn find_system_pgvector() -> Vec<PathBuf> {
    let lib_ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let lib_candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            // Homebrew Apple Silicon
            PathBuf::from("/opt/homebrew/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/lib/postgresql/vector.dylib"),
            // Homebrew Intel
            PathBuf::from("/usr/local/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/usr/local/opt/postgresql@17/lib/postgresql/vector.dylib"),
            // Unversioned Homebrew
            PathBuf::from("/opt/homebrew/lib/postgresql/vector.dylib"),
            PathBuf::from("/usr/local/lib/postgresql/vector.dylib"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            PathBuf::from("/usr/lib/postgresql/17/lib/vector.so"),
            PathBuf::from("/usr/lib64/pgsql/vector.so"),
        ]
    } else {
        vec![]
    };

    let ext_candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/opt/homebrew/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/share/postgresql/extension"),
            PathBuf::from("/usr/local/share/postgresql/extension"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            PathBuf::from("/usr/share/postgresql/17/extension"),
            PathBuf::from("/usr/share/pgsql/extension"),
        ]
    } else {
        vec![]
    };

    let mut files = Vec::new();

    let lib_found = lib_candidates.iter().find(|p| p.exists());
    if lib_found.is_none() {
        // Also try pg_config --pkglibdir if available
        if let Ok(output) = std::process::Command::new("pg_config")
            .arg("--pkglibdir")
            .output()
        {
            if output.status.success() {
                let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let candidate = PathBuf::from(&dir).join(format!("vector.{lib_ext}"));
                if candidate.exists() {
                    files.push(candidate);
                }
            }
        }
    } else if let Some(path) = lib_found {
        files.push(path.clone());
    }

    let ext_found = ext_candidates.iter().find(|p| p.join("vector.control").exists());
    if let Some(ext_dir) = ext_found {
        if let Ok(entries) = std::fs::read_dir(ext_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("vector") && (name_str.ends_with(".control") || name_str.ends_with(".sql")) {
                    files.push(entry.path());
                }
            }
        }
    } else {
        // Fallback: pg_config --sharedir
        if let Ok(output) = std::process::Command::new("pg_config")
            .arg("--sharedir")
            .output()
        {
            if output.status.success() {
                let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let ext_dir = PathBuf::from(&dir).join("extension");
                if ext_dir.join("vector.control").exists() {
                    if let Ok(entries) = std::fs::read_dir(&ext_dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name();
                            let name_str = name.to_string_lossy();
                            if name_str.starts_with("vector") && (name_str.ends_with(".control") || name_str.ends_with(".sql")) {
                                files.push(entry.path());
                            }
                        }
                    }
                }
            }
        }
    }

    if !files.is_empty() {
        let has_lib = files.iter().any(|f| {
            f.extension().map_or(false, |e| e == "dylib" || e == "so" || e == "dll")
        });
        let has_control = files.iter().any(|f| {
            f.extension().map_or(false, |e| e == "control")
        });
        if !has_lib || !has_control {
            info!(
                has_lib,
                has_control,
                "Incomplete pgvector installation found, skipping"
            );
            return vec![];
        }
    }

    files
}

/// Copy a binary file using read+write instead of fs::copy to avoid macOS
/// quarantine (`com.apple.provenance`) attributes blocking the operation.
pub(super) fn copy_binary(src: &Path, dst: &Path) -> std::io::Result<()> {
    let data = std::fs::read(src)?;
    std::fs::write(dst, &data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub(super) fn platform_strings() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64v8"
    } else {
        "amd64"
    };
    (os, arch)
}

impl Drop for EmbeddedPg {
    fn drop(&mut self) {
        // pg_embed handles cleanup on drop, but we log it
        tracing::debug!("EmbeddedPg instance dropped");
    }
}
