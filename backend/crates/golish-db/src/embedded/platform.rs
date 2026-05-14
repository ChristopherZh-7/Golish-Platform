//! Platform-specific helpers for embedded PostgreSQL.

use std::path::{Path, PathBuf};
use tracing::info;

use super::EmbeddedPg;

/// Search common system paths for pgvector extension files.
/// Returns paths to the shared library (.dylib/.so) and the control + SQL files.
pub(super) fn find_system_pgvector() -> Vec<PathBuf> {
    let candidates = golish_platform::postgres::system_pgvector_candidates();
    let lib_ext = golish_platform::Platform::current().shared_lib_extension();

    let mut files = Vec::new();

    let lib_found = candidates.library_files.iter().find(|p| p.exists());
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

    let ext_found = candidates
        .extension_dirs
        .iter()
        .find(|p| p.join("vector.control").exists());
    if let Some(ext_dir) = ext_found {
        if let Ok(entries) = std::fs::read_dir(ext_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("vector")
                    && (name_str.ends_with(".control") || name_str.ends_with(".sql"))
                {
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
                            if name_str.starts_with("vector")
                                && (name_str.ends_with(".control") || name_str.ends_with(".sql"))
                            {
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
            f.extension()
                .is_some_and(|e| e == "dylib" || e == "so" || e == "dll")
        });
        let has_control = files
            .iter()
            .any(|f| f.extension().is_some_and(|e| e == "control"));
        if !has_lib || !has_control {
            info!(
                has_lib,
                has_control, "Incomplete pgvector installation found, skipping"
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
    golish_platform::fs_perms::set_executable(dst)
}

impl Drop for EmbeddedPg {
    fn drop(&mut self) {
        // [PG-DIAG] Bumped from debug→info so it surfaces in Windows release
        // builds. NB: golish bootstrap currently `std::mem::forget`s the
        // GolishDb wrapper (see `app/bootstrap.rs::spawn_embedded_pg`), so
        // in the GUI path this log line should NEVER fire — its presence
        // would itself be a smoking gun.
        tracing::info!("[PG-DIAG] EmbeddedPg instance dropped (graceful shutdown path)");
    }
}
