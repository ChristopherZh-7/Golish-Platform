//! OWASP ZAP platform helpers.

use std::io;
use std::path::{Path, PathBuf};

use crate::{paths, Platform, PlatformKind};

/// ZAP launcher filename for the current platform.
pub fn zap_launcher_name() -> &'static str {
    if Platform::current().is_windows() {
        "zap.bat"
    } else {
        "zap.sh"
    }
}

/// Well-known ZAP installation launcher paths for the current platform.
pub fn zap_installation_candidates() -> Vec<PathBuf> {
    match Platform::current().kind {
        PlatformKind::MacOs => {
            let mut candidates = vec![
                PathBuf::from("/Applications/ZAP.app/Contents/Java/zap.sh"),
                PathBuf::from("/Applications/OWASP ZAP.app/Contents/Java/zap.sh"),
            ];
            if let Some(base) = paths::app_data_base("golish-platform") {
                candidates.push(base.join("tools").join("ZAP").join("zap.sh"));
            }
            candidates
        }
        PlatformKind::Windows => {
            // Well-known paths from the official NSIS installer.
            // The default install dir is `C:\Program Files\ZAP\Zed Attack Proxy\`;
            // older / custom installs may land in `C:\Program Files\ZAP\`,
            // `C:\Program Files\OWASP ZAP\`, or the WOW64 (x86) variants.
            let mut candidates = vec![
                PathBuf::from(r"C:\Program Files\ZAP\Zed Attack Proxy\zap.bat"),
                PathBuf::from(r"C:\Program Files\ZAP\zap.bat"),
                PathBuf::from(r"C:\Program Files\OWASP ZAP\zap.bat"),
                PathBuf::from(r"C:\Program Files (x86)\ZAP\Zed Attack Proxy\zap.bat"),
                PathBuf::from(r"C:\Program Files (x86)\ZAP\zap.bat"),
                PathBuf::from(r"C:\Program Files (x86)\OWASP ZAP\zap.bat"),
            ];
            // Also cover ZAP unpacked into our managed tools directory
            // (Crossplatform.zip extracted by the in-app installer).
            if let Some(base) = paths::app_data_base("golish-platform") {
                candidates.push(base.join("tools").join("ZAP").join("zap.bat"));
            }
            candidates
        }
        _ => {
            let mut candidates = vec![
                PathBuf::from("/usr/share/zaproxy/zap.sh"),
                PathBuf::from("/opt/zaproxy/zap.sh"),
            ];
            if let Some(base) = paths::app_data_base("golish-platform") {
                candidates.push(base.join("tools").join("ZAP").join("zap.sh"));
            }
            candidates
        }
    }
}

/// Install a ZAP root CA certificate into the platform trust store.
pub fn install_root_cert(cert_path: &Path) -> io::Result<()> {
    if !Platform::current().is_macos() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Auto-install not supported on this OS",
        ));
    }

    macos::install_root_cert(cert_path)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io;
    use std::path::{Path, PathBuf};

    pub fn install_root_cert(cert_path: &Path) -> io::Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Cannot find home directory"))?;
        let keychain_path = login_keychain_path(&home);

        let output = std::process::Command::new("security")
            .args([
                "add-trusted-cert",
                "-r",
                "trustRoot",
                "-k",
                &keychain_path.to_string_lossy(),
                &cert_path.to_string_lossy(),
            ])
            .output()?;

        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }

    fn login_keychain_path(home: &Path) -> PathBuf {
        let login_keychain = home.join("Library/Keychains/login.keychain-db");
        if login_keychain.exists() {
            login_keychain
        } else {
            home.join("Library/Keychains/login.keychain")
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos {
    use std::io;
    use std::path::Path;

    pub fn install_root_cert(_cert_path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Auto-install not supported on this OS",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Platform;

    #[test]
    fn zap_launcher_name_matches_platform() {
        if Platform::current().is_windows() {
            assert_eq!(zap_launcher_name(), "zap.bat");
        } else {
            assert_eq!(zap_launcher_name(), "zap.sh");
        }
    }

    #[test]
    fn zap_installation_candidates_are_non_empty() {
        let candidates = zap_installation_candidates();
        assert!(!candidates.is_empty());
        let launcher = zap_launcher_name();
        assert!(candidates
            .iter()
            .any(|path| path.to_string_lossy().ends_with(launcher)));
    }

    #[test]
    fn zap_installation_candidates_include_managed_tools_dir() {
        // On every platform the in-app installer extracts ZAP into
        // `<app_data_base>/tools/ZAP/`, so the detector must include
        // that path (using the platform-appropriate launcher).
        let candidates = zap_installation_candidates();
        let launcher = zap_launcher_name();
        let has_managed = candidates.iter().any(|path| {
            let s = path.to_string_lossy();
            // Use forward-slash check that also matches Windows back-slashes.
            s.replace('\\', "/").contains("/tools/ZAP/") && s.ends_with(launcher)
        });
        assert!(
            has_managed,
            "candidates should include managed tools dir: {:?}",
            candidates
        );
    }
}
