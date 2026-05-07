//! Desktop system proxy helpers.

use std::io;

use crate::Platform;

/// Enable HTTP/HTTPS system proxy for active network services.
pub fn set_system_proxy(host: &str, port: u16) -> io::Result<()> {
    if !Platform::current().is_macos() {
        return Err(unsupported());
    }
    macos::set_system_proxy(host, port)
}

/// Disable HTTP/HTTPS system proxy for active network services.
pub fn clear_system_proxy() -> io::Result<()> {
    if !Platform::current().is_macos() {
        return Err(unsupported());
    }
    macos::clear_system_proxy()
}

/// Return the active HTTP proxy, if one is configured.
pub fn get_system_proxy() -> io::Result<Option<(String, u16)>> {
    if !Platform::current().is_macos() {
        return Ok(None);
    }
    macos::get_system_proxy()
}

/// Parse `networksetup -getwebproxy` output.
pub fn parse_macos_web_proxy_output(text: &str) -> Option<(String, u16)> {
    let enabled = text
        .lines()
        .any(|line| line.starts_with("Enabled:") && line.contains("Yes"));
    if !enabled {
        return None;
    }

    let host = text
        .lines()
        .find(|line| line.starts_with("Server:"))
        .and_then(|line| line.split(':').nth(1))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let port = text
        .lines()
        .find(|line| line.starts_with("Port:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(0);

    if host.is_empty() || port == 0 {
        None
    } else {
        Some((host, port))
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "System proxy control is only supported on macOS",
    )
}

#[cfg(target_os = "macos")]
mod macos {
    use std::io;

    pub fn set_system_proxy(host: &str, port: u16) -> io::Result<()> {
        let services = active_network_services()?;
        for service in &services {
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxy", service, host, &port.to_string()])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxystate", service, "on"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxy", service, host, &port.to_string()])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxystate", service, "on"])
                .output();
        }
        Ok(())
    }

    pub fn clear_system_proxy() -> io::Result<()> {
        let services = active_network_services()?;
        for service in &services {
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxystate", service, "off"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxystate", service, "off"])
                .output();
        }
        Ok(())
    }

    pub fn get_system_proxy() -> io::Result<Option<(String, u16)>> {
        let services = active_network_services()?;
        let Some(service) = services.first() else {
            return Ok(None);
        };
        let output = std::process::Command::new("networksetup")
            .args(["-getwebproxy", service])
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(super::parse_macos_web_proxy_output(&text))
    }

    fn active_network_services() -> io::Result<Vec<String>> {
        let output = std::process::Command::new("networksetup")
            .arg("-listallnetworkservices")
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text
            .lines()
            .skip(1)
            .filter(|line| !line.starts_with('*'))
            .map(|line| line.to_string())
            .collect())
    }
}

#[cfg(not(target_os = "macos"))]
mod macos {
    use std::io;

    pub fn set_system_proxy(_host: &str, _port: u16) -> io::Result<()> {
        Err(super::unsupported())
    }

    pub fn clear_system_proxy() -> io::Result<()> {
        Err(super::unsupported())
    }

    pub fn get_system_proxy() -> io::Result<Option<(String, u16)>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enabled_macos_proxy_output() {
        let text = "Enabled: Yes\nServer: 127.0.0.1\nPort: 8080\n";
        assert_eq!(
            parse_macos_web_proxy_output(text),
            Some(("127.0.0.1".to_string(), 8080))
        );
    }

    #[test]
    fn ignores_disabled_or_incomplete_macos_proxy_output() {
        assert_eq!(
            parse_macos_web_proxy_output("Enabled: No\nServer: 127.0.0.1\nPort: 8080\n"),
            None
        );
        assert_eq!(
            parse_macos_web_proxy_output("Enabled: Yes\nPort: 0\n"),
            None
        );
    }
}
