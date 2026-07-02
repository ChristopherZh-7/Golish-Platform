//! Normalization helpers for first-class surface identity rows.
//!
//! Phase 2.1 keeps this local to `golish-db`: no legacy table writes, no app
//! commands, and no collector integration yet.

use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWebOrigin {
    pub scheme: String,
    pub host: String,
    pub host_type: String,
    pub port: i32,
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedNetworkEndpoint {
    pub ip: String,
    pub port: i32,
    pub transport: String,
}

fn default_port_for_scheme(scheme: &str) -> Option<i32> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn normalize_host(host: &str) -> Option<String> {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        Some(ip.to_string())
    } else {
        Some(normalized)
    }
}

fn host_type(host: &str) -> String {
    if host.parse::<IpAddr>().is_ok() {
        "ip".to_string()
    } else {
        "domain".to_string()
    }
}

fn normalize_scheme(scheme: &str) -> Option<String> {
    let normalized = scheme.trim_end_matches(':').to_ascii_lowercase();
    if normalized == "http" || normalized == "https" {
        Some(normalized)
    } else {
        None
    }
}

fn normalize_port(port: Option<i32>, scheme: &str) -> Option<i32> {
    match port {
        Some(port) if (1..=65535).contains(&port) => Some(port),
        Some(_) => None,
        None => default_port_for_scheme(scheme),
    }
}

pub fn normalize_origin_parts(
    scheme: &str,
    host: &str,
    port: Option<i32>,
) -> Option<NormalizedWebOrigin> {
    let scheme = normalize_scheme(scheme)?;
    let host = normalize_host(host)?;
    let port = normalize_port(port, &scheme)?;
    let host_type = host_type(&host);
    let origin_host = if host_type == "ip" && host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let origin = format!("{scheme}://{origin_host}:{port}");
    Some(NormalizedWebOrigin {
        scheme,
        host_type,
        host,
        port,
        origin,
    })
}

pub fn normalize_web_origin(url: &str) -> Option<NormalizedWebOrigin> {
    let trimmed = url.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    let scheme = normalize_scheme(scheme)?;
    let authority = rest.split(['/', '?', '#']).next()?.trim();
    if authority.is_empty() {
        return None;
    }

    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = if host_port.starts_with('[') {
        let end = host_port.find(']')?;
        let host = &host_port[1..end];
        let port = host_port[end + 1..]
            .strip_prefix(':')
            .and_then(|raw| raw.parse::<i32>().ok());
        (host, port)
    } else if let Some((host, raw_port)) = host_port.rsplit_once(':') {
        if raw_port.chars().all(|ch| ch.is_ascii_digit()) {
            (host, raw_port.parse::<i32>().ok())
        } else {
            (host_port, None)
        }
    } else {
        (host_port, None)
    };

    normalize_origin_parts(&scheme, host, port)
}

pub fn normalize_network_endpoint(
    ip: &str,
    port: i32,
    transport: &str,
) -> Option<NormalizedNetworkEndpoint> {
    let raw_ip = ip.trim().trim_start_matches('[').trim_end_matches(']');
    if raw_ip.is_empty() || !(1..=65535).contains(&port) {
        return None;
    }
    let ip = raw_ip.parse::<IpAddr>().ok()?.to_string();
    let transport = match transport.trim().to_ascii_lowercase().as_str() {
        "tcp" => "tcp",
        "udp" => "udp",
        "unknown" | "" => "unknown",
        _ => return None,
    }
    .to_string();

    Some(NormalizedNetworkEndpoint {
        ip,
        port,
        transport,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_origin_parts_with_default_ports() {
        let http = normalize_origin_parts("http", "Example.COM.", None).unwrap();
        assert_eq!(http.origin, "http://example.com:80");
        assert_eq!(http.port, 80);

        let https = normalize_origin_parts("https", "Example.COM", None).unwrap();
        assert_eq!(https.origin, "https://example.com:443");
        assert_eq!(https.port, 443);
    }

    #[test]
    fn keeps_explicit_ports_distinct() {
        let default_https = normalize_web_origin("https://a.example.com/login").unwrap();
        let alt_https = normalize_web_origin("https://a.example.com:8443/login").unwrap();
        assert_eq!(default_https.origin, "https://a.example.com:443");
        assert_eq!(alt_https.origin, "https://a.example.com:8443");
        assert_ne!(default_https.origin, alt_https.origin);
    }

    #[test]
    fn parses_ip_literal_url() {
        let origin = normalize_web_origin("https://1.1.1.1/login").unwrap();
        assert_eq!(origin.scheme, "https");
        assert_eq!(origin.host, "1.1.1.1");
        assert_eq!(origin.host_type, "ip");
        assert_eq!(origin.port, 443);
        assert_eq!(origin.origin, "https://1.1.1.1:443");
    }

    #[test]
    fn rejects_relative_urls() {
        assert!(normalize_web_origin("/login").is_none());
        assert!(normalize_web_origin("app.js").is_none());
        assert!(normalize_web_origin("//example.com/login").is_none());
    }

    #[test]
    fn normalizes_network_endpoint_identity() {
        let endpoint = normalize_network_endpoint(" 1.1.1.1 ", 443, "TCP").unwrap();
        assert_eq!(endpoint.ip, "1.1.1.1");
        assert_eq!(endpoint.port, 443);
        assert_eq!(endpoint.transport, "tcp");

        let ipv6 = normalize_network_endpoint(" [2001:0DB8::1] ", 443, "tcp").unwrap();
        assert_eq!(ipv6.ip, "2001:db8::1");
    }

    #[test]
    fn rejects_non_ip_network_endpoint_identity() {
        assert!(normalize_network_endpoint("example.com", 443, "tcp").is_none());
    }
}
