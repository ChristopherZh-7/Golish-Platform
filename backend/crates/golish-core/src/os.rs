use std::io;
use std::path::Path;

/// Open a URL in the system default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    open_with_system(url)?;
    Ok(())
}

/// Open a directory in the system file manager.
pub fn reveal_path(path: &Path) -> io::Result<()> {
    let arg = path.to_string_lossy();
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(arg.as_ref()).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(arg.as_ref())
            .spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(arg.as_ref())
            .spawn()?;
    }
    Ok(())
}

fn open_with_system(target: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(target).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", target])
            .spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(target).spawn()?;
    }
    Ok(())
}
