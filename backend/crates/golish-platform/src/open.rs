//! Cross-platform "open URL / reveal path in file manager" helpers.
//!
//! Replaces the previous `golish-core::os` implementation. Callers
//! should use these instead of building per-OS `Command` objects.

use std::io;
use std::path::Path;

/// Open a URL in the user's default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    open_with_system(url)
}

/// Reveal a path in the system file manager (Finder / Explorer /
/// `xdg-open`'d default).
pub fn reveal_path(path: &Path) -> io::Result<()> {
    let arg = path.to_string_lossy();
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(arg.as_ref())
            .spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(arg.as_ref())
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(arg.as_ref())
            .spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reveal_path: unsupported platform",
        ))
    }
}

fn open_with_system(target: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(target).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", target])
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(target).spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "open_url: unsupported platform",
        ))
    }
}
