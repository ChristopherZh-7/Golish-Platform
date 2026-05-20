//! Wire-format response structures for the Shodan API.

#![allow(dead_code)]

use serde::Deserialize;

/// `/shodan/host/search` envelope.
///
/// On error Shodan returns HTTP 200 with `{"error": "..."}`; non-2xx is
/// reserved for transport-level failures.
#[derive(Debug, Deserialize, Default)]
pub struct ShodanSearchEnvelope {
    /// Total matches across all pages.
    #[serde(default)]
    pub total: u64,
    /// Per-banner matches (one per service/port).
    #[serde(default)]
    pub matches: Vec<ShodanMatch>,
    /// Soft error message (present when the API rejected the request).
    #[serde(default)]
    pub error: Option<String>,
}

impl ShodanSearchEnvelope {
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
    pub fn error_msg(&self) -> &str {
        self.error.as_deref().unwrap_or("unknown")
    }
}

/// Minimal `{"error": "..."}` body used by `client::decode_envelope` to
/// pre-flight soft-error detection on both 2xx and 4xx responses.
#[derive(Debug, Deserialize, Default)]
pub struct ShodanErrorEnvelope {
    #[serde(default)]
    pub error: Option<String>,
}

/// Single banner match in a Shodan search response.
#[derive(Debug, Deserialize, Default)]
pub struct ShodanMatch {
    #[serde(default)]
    pub ip_str: Option<String>,
    #[serde(default)]
    pub port: Option<u32>,
    /// Free-form banner data (HTTP headers, SSH banner, etc.).
    #[serde(default)]
    pub data: Option<String>,
    /// Organization that owns the IP space (treated as organization_name).
    #[serde(default)]
    pub org: Option<String>,
    /// Internet Service Provider.
    #[serde(default)]
    pub isp: Option<String>,
    /// AS number (string with `AS` prefix, e.g. `AS13335`).
    #[serde(default)]
    pub asn: Option<String>,
    /// OS detected via service fingerprinting.
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub hostnames: Vec<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub location: Option<ShodanLocation>,
    /// Optional nested HTTP details (only present on web matches).
    #[serde(default)]
    pub http: Option<ShodanHttp>,
    /// Optional SSL cert info.
    #[serde(default)]
    pub ssl: Option<ShodanSsl>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShodanLocation {
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub country_name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShodanHttp {
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<i32>,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShodanSsl {
    #[serde(default)]
    pub cert: Option<ShodanCert>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShodanCert {
    /// Subject CN (often a domain).
    #[serde(default)]
    pub subject: Option<ShodanCertSubject>,
    /// Issuer info.
    #[serde(default)]
    pub issuer: Option<ShodanCertSubject>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShodanCertSubject {
    /// Common Name (CN) — e.g. "example.com".
    #[serde(rename = "CN", default)]
    pub cn: Option<String>,
}

/// `/api-info` envelope (returned by the connection-test probe).
#[derive(Debug, Deserialize, Default)]
pub struct ShodanApiInfo {
    #[serde(default)]
    pub query_credits: Option<u64>,
    #[serde(default)]
    pub scan_credits: Option<u64>,
    #[serde(default)]
    pub plan: Option<String>,
    /// Soft error (present when key is invalid).
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_envelope_success_parses() {
        let json = r#"{
            "total": 2,
            "matches": [
                {
                    "ip_str": "8.8.8.8",
                    "port": 53,
                    "org": "Google",
                    "asn": "AS15169",
                    "isp": "Google LLC",
                    "hostnames": ["dns.google"],
                    "domains": ["google.com"],
                    "transport": "udp",
                    "location": {"city": "Mountain View", "country_code": "US"},
                    "http": {"title": "Google", "server": "gws", "status": 200}
                }
            ]
        }"#;
        let env: ShodanSearchEnvelope = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        assert_eq!(env.total, 2);
        assert_eq!(env.matches.len(), 1);
        assert_eq!(env.matches[0].ip_str.as_deref(), Some("8.8.8.8"));
        assert_eq!(env.matches[0].port, Some(53));
        assert_eq!(
            env.matches[0].http.as_ref().unwrap().title.as_deref(),
            Some("Google")
        );
    }

    #[test]
    fn search_envelope_error_parses() {
        let json = r#"{"error": "Invalid API key"}"#;
        let env: ShodanSearchEnvelope = serde_json::from_str(json).unwrap();
        assert!(!env.is_ok());
        assert_eq!(env.error_msg(), "Invalid API key");
        assert!(env.matches.is_empty());
    }

    #[test]
    fn cert_subject_parses_cn() {
        let json = r#"{"CN": "example.com"}"#;
        let s: ShodanCertSubject = serde_json::from_str(json).unwrap();
        assert_eq!(s.cn.as_deref(), Some("example.com"));
    }

    #[test]
    fn api_info_parses() {
        let json = r#"{"query_credits": 80, "scan_credits": 100, "plan": "freelancer"}"#;
        let info: ShodanApiInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.query_credits, Some(80));
        assert_eq!(info.plan.as_deref(), Some("freelancer"));
    }

    #[test]
    fn match_default_when_fields_missing() {
        let json = r#"{}"#;
        let m: ShodanMatch = serde_json::from_str(json).unwrap();
        assert!(m.ip_str.is_none());
        assert!(m.hostnames.is_empty());
    }
}
