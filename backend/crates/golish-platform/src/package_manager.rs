//! Per-platform native package-manager identification + install hint
//! helpers.
//!
//! Lets callers ask "what's the native equivalent of `brew install
//! nmap` on this OS?" without scattering `cfg!` checks all over the
//! place.

use crate::detect::Platform;

/// Native package-manager classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageManager {
    Homebrew,
    /// macOS Homebrew Cask — installs GUI apps via `brew install --cask`.
    /// Tracked separately because it has its own list/install/uninstall
    /// surface that doesn't intersect with formulas.
    HomebrewCask,
    Apt,
    Yum,
    Pacman,
    Winget,
    Scoop,
    Chocolatey,
    /// No idiomatic system-wide package manager (e.g. plain Linux
    /// without root). Callers should fall back to `cargo install` /
    /// GitHub release / `pip install --user`.
    None,
}

impl PackageManager {
    /// Return the manager that the platform abstraction layer
    /// **assumes is the default** for the current platform. This is
    /// best-effort: if the user is on Linux without `apt`, callers
    /// must handle the resulting "command not found" error.
    pub fn default_for_platform() -> Self {
        match Platform::current().kind {
            crate::detect::PlatformKind::MacOs => PackageManager::Homebrew,
            crate::detect::PlatformKind::Windows => PackageManager::Winget,
            crate::detect::PlatformKind::Linux | crate::detect::PlatformKind::OtherUnix => {
                PackageManager::Apt
            }
        }
    }

    /// Pretty-print the install command snippet a human would type.
    /// Used for "we can't auto-install on this platform — try …"
    /// hints.
    pub fn install_command(self, package: &str) -> String {
        match self {
            PackageManager::Homebrew => format!("brew install {package}"),
            PackageManager::HomebrewCask => format!("brew install --cask {package}"),
            PackageManager::Apt => format!("sudo apt install {package}"),
            PackageManager::Yum => format!("sudo yum install {package}"),
            PackageManager::Pacman => format!("sudo pacman -S {package}"),
            PackageManager::Winget => format!("winget install -e --id {package}"),
            PackageManager::Scoop => format!("scoop install {package}"),
            PackageManager::Chocolatey => format!("choco install {package}"),
            PackageManager::None => {
                format!("(no system package manager known — install '{package}' manually)")
            }
        }
    }

    /// Display label for UI surfaces.
    pub const fn label(self) -> &'static str {
        match self {
            PackageManager::Homebrew => "Homebrew",
            PackageManager::HomebrewCask => "Homebrew Cask",
            PackageManager::Apt => "APT",
            PackageManager::Yum => "YUM",
            PackageManager::Pacman => "pacman",
            PackageManager::Winget => "winget",
            PackageManager::Scoop => "Scoop",
            PackageManager::Chocolatey => "Chocolatey",
            PackageManager::None => "manual",
        }
    }

    /// Return the list of installed packages tracked by this manager.
    ///
    /// Only `Homebrew` and `HomebrewCask` are currently implemented; other
    /// managers return an empty set. Spawning the underlying CLI (`brew`)
    /// also fails gracefully on platforms where it's not available, so
    /// callers don't need to add their own `cfg!(target_os = …)` guards.
    pub fn installed_packages(self) -> std::collections::HashSet<String> {
        match self {
            PackageManager::Homebrew => list_brew_packages("formula"),
            PackageManager::HomebrewCask => list_brew_packages("cask"),
            _ => std::collections::HashSet::new(),
        }
    }
}

fn list_brew_packages(kind: &str) -> std::collections::HashSet<String> {
    let arg = match kind {
        "cask" => "--cask",
        _ => "--formula",
    };
    std::process::Command::new("brew")
        .args(["list", arg, "-1"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_for_platform_is_consistent() {
        let pm = PackageManager::default_for_platform();
        if cfg!(target_os = "macos") {
            assert_eq!(pm, PackageManager::Homebrew);
        } else if cfg!(target_os = "windows") {
            assert_eq!(pm, PackageManager::Winget);
        } else if cfg!(target_os = "linux") {
            assert_eq!(pm, PackageManager::Apt);
        }
    }

    #[test]
    fn install_command_format() {
        assert_eq!(
            PackageManager::Homebrew.install_command("nmap"),
            "brew install nmap"
        );
        assert_eq!(
            PackageManager::Winget.install_command("Insecure.Nmap"),
            "winget install -e --id Insecure.Nmap"
        );
    }
}
