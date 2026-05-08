//! Cross-platform file-permission helpers.
//!
//! On Unix these manipulate the executable bit (`chmod +x`); on
//! Windows the executable concept is decided by the file extension
//! (`.exe`, `.bat`, …) so the helpers degrade to no-ops or filename
//! checks.

use std::io;
use std::path::Path;

/// Returns true if `path` should be considered an executable on the
/// current platform.
pub fn has_execute_bit(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| has_execute_bit_from_mode(meta.permissions().mode()))
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => matches!(
                ext.to_ascii_lowercase().as_str(),
                "exe" | "bat" | "cmd" | "ps1" | "com" | "msi"
            ),
            None => false,
        }
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        false
    }
}

/// Unix-only: check whether any of the user/group/other execute bits are set
/// in `mode`. Useful when callers already have a `Metadata` and don't want
/// to re-stat the path inside [`has_execute_bit`].
#[cfg(unix)]
#[inline]
pub fn has_execute_bit_from_mode(mode: u32) -> bool {
    mode & 0o111 != 0
}

/// Make a single file executable.
///
/// On Unix this is `chmod 0o755`; on Windows it is a no-op (returns
/// `Ok(())` because the OS has no equivalent operation).
pub fn set_executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !path.is_file() {
            return Ok(());
        }
        let meta = std::fs::metadata(path)?;
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Recursively walk `path` and run [`set_executable`] on every regular
/// file. Returns the number of files touched.
pub fn set_executable_recursive(path: &Path) -> io::Result<u64> {
    let mut count = 0_u64;
    walk(path, &mut count)?;
    Ok(count)
}

/// Return true when `dir` contains at least one executable file.
pub fn has_any_executable_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if has_any_executable_file(&path) {
                return true;
            }
        } else if path.is_file() && has_execute_bit(&path) {
            return true;
        }
    }

    false
}

fn walk(path: &Path, count: &mut u64) -> io::Result<()> {
    if path.is_file() {
        set_executable(path)?;
        *count += 1;
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)?.flatten() {
        walk(&entry.path(), count)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn smoke_set_executable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "hi").unwrap();
        drop(f);
        set_executable(&p).expect("set_executable ok");
        if cfg!(unix) {
            assert!(has_execute_bit(&p));
        }
    }

    #[test]
    fn smoke_recursive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("nested/b.bin");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let n = set_executable_recursive(dir.path()).unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn detects_executable_file_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();

        let file_name = if cfg!(target_os = "windows") {
            "tool.exe"
        } else {
            "tool"
        };
        let executable = nested.join(file_name);
        std::fs::write(&executable, b"x").unwrap();
        set_executable(&executable).unwrap();

        assert!(has_any_executable_file(dir.path()));
    }
}
