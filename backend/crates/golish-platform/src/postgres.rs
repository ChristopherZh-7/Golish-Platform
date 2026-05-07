//! Platform helpers for embedded PostgreSQL and pgvector integration.

use std::path::{Path, PathBuf};

use crate::{Platform, PlatformKind};

/// System locations that may contain pgvector extension artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgvectorCandidates {
    pub library_files: Vec<PathBuf>,
    pub extension_dirs: Vec<PathBuf>,
}

/// Return `(os, arch)` strings expected by `pg-embed` binary downloads.
pub fn pg_embed_fetch_tag() -> (&'static str, &'static str) {
    Platform::current().fetch_tag()
}

/// Native library filename for the pgvector extension.
pub fn pgvector_library_name() -> String {
    format!("vector.{}", Platform::current().shared_lib_extension())
}

/// Common system locations for pgvector files on the current platform.
pub fn system_pgvector_candidates() -> PgvectorCandidates {
    let library_files = match Platform::current().kind {
        PlatformKind::MacOs => vec![
            PathBuf::from("/opt/homebrew/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/lib/postgresql/vector.dylib"),
            PathBuf::from("/usr/local/lib/postgresql@17/vector.dylib"),
            PathBuf::from("/usr/local/opt/postgresql@17/lib/postgresql/vector.dylib"),
            PathBuf::from("/opt/homebrew/lib/postgresql/vector.dylib"),
            PathBuf::from("/usr/local/lib/postgresql/vector.dylib"),
        ],
        PlatformKind::Linux | PlatformKind::OtherUnix => vec![
            PathBuf::from("/usr/lib/postgresql/17/lib/vector.so"),
            PathBuf::from("/usr/lib64/pgsql/vector.so"),
        ],
        PlatformKind::Windows => vec![],
    };

    let extension_dirs = match Platform::current().kind {
        PlatformKind::MacOs => vec![
            PathBuf::from("/opt/homebrew/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/share/postgresql@17/extension"),
            PathBuf::from("/usr/local/opt/postgresql@17/share/postgresql@17/extension"),
            PathBuf::from("/opt/homebrew/share/postgresql/extension"),
            PathBuf::from("/usr/local/share/postgresql/extension"),
        ],
        PlatformKind::Linux | PlatformKind::OtherUnix => vec![
            PathBuf::from("/usr/share/postgresql/17/extension"),
            PathBuf::from("/usr/share/pgsql/extension"),
        ],
        PlatformKind::Windows => vec![],
    };

    PgvectorCandidates {
        library_files,
        extension_dirs,
    }
}

/// Clear macOS quarantine metadata from embedded PostgreSQL executable directories.
///
/// This is a no-op on non-macOS platforms.
pub fn clear_quarantine_dirs(root: &Path, subdirs: &[&str]) {
    #[cfg(target_os = "macos")]
    {
        for subdir in subdirs {
            let dir = root.join(subdir);
            if dir.exists() {
                tracing::info!(dir = %dir.display(), "Clearing macOS quarantine attributes");
                let _ = std::process::Command::new("xattr")
                    .args(["-cr", &dir.to_string_lossy()])
                    .output();
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, subdirs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Platform;

    #[test]
    fn pg_embed_fetch_tag_matches_platform() {
        assert_eq!(pg_embed_fetch_tag(), Platform::current().fetch_tag());
    }

    #[test]
    fn pgvector_library_name_uses_shared_library_extension() {
        assert_eq!(
            pgvector_library_name(),
            format!("vector.{}", Platform::current().shared_lib_extension())
        );
    }

    #[test]
    fn system_pgvector_candidates_are_platform_specific() {
        let candidates = system_pgvector_candidates();
        if Platform::current().is_windows() {
            assert!(candidates.library_files.is_empty());
            assert!(candidates.extension_dirs.is_empty());
        } else {
            assert!(!candidates.library_files.is_empty());
            assert!(!candidates.extension_dirs.is_empty());
        }
    }
}
