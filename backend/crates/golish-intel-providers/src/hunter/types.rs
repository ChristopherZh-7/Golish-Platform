//! Wire-format response structures for the Hunter API.

#![allow(dead_code)]

use serde::Deserialize;

fn null_to_default_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Common envelope from `https://hunter.qianxin.com/openApi/search`.
#[derive(Debug, Deserialize, Default)]
pub struct HunterEnvelope {
    /// 200 on success, non-200 on failure (Hunter uses HTTP 200 with body code).
    #[serde(default)]
    pub code: i32,
    /// Human-readable message (filled on error).
    #[serde(default)]
    pub message: Option<String>,
    /// Alias some endpoints use.
    #[serde(default)]
    pub msg: Option<String>,
    /// Result body — `None` on error.
    #[serde(default)]
    pub data: Option<HunterData>,
}

impl HunterEnvelope {
    pub fn is_ok(&self) -> bool {
        self.code == 200
    }
    pub fn error_msg(&self) -> &str {
        self.message
            .as_deref()
            .or(self.msg.as_deref())
            .unwrap_or("unknown")
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct HunterData {
    #[serde(default)]
    pub total: u64,
    /// Per-doc fields. `arr` is the documented key.
    #[serde(default)]
    pub arr: Vec<HunterRow>,
    /// Remaining quota for this account (Hunter exposes this in `rest_quota`).
    #[serde(default)]
    pub rest_quota: Option<String>,
    /// Quota consumed by this request.
    #[serde(default)]
    pub consume_quota: Option<String>,
}

/// Single match row from Hunter `/openApi/search`.
#[derive(Debug, Deserialize, Default)]
pub struct HunterRow {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub port: Option<u32>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub web_title: Option<String>,
    #[serde(default)]
    pub status_code: Option<i32>,
    #[serde(default, deserialize_with = "null_to_default_vec")]
    pub component: Vec<HunterComponent>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub province: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    /// Company that owns the asset — used to populate `organization_name`.
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub isp: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub header_server: Option<String>,
    #[serde(default)]
    pub as_org: Option<String>,
    #[serde(default)]
    pub base_protocol: Option<String>,
    #[serde(default)]
    pub ssl_certificate: Option<String>,
    #[serde(default)]
    pub cert_sha256: Option<String>,
    #[serde(default, rename = "number")]
    pub icp_number: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct HunterComponent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_success_parses() {
        let json = r#"{
            "code": 200,
            "message": "ok",
            "data": {
                "total": 1,
                "consume_quota": "1",
                "rest_quota": "499",
                "arr": [
                    {
                        "url": "https://example.com",
                        "ip": "1.2.3.4",
                        "port": 443,
                        "domain": "example.com",
                        "protocol": "https",
                        "web_title": "Example",
                        "status_code": 200,
                        "company": "Example Corp",
                        "component": [{"name": "nginx", "version": "1.21.4"}]
                    }
                ]
            }
        }"#;
        let env: HunterEnvelope = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        let data = env.data.unwrap();
        assert_eq!(data.total, 1);
        assert_eq!(data.arr.len(), 1);
        assert_eq!(data.arr[0].port, Some(443));
        assert_eq!(data.arr[0].company.as_deref(), Some("Example Corp"));
    }

    #[test]
    fn envelope_error_parses() {
        let json = r#"{"code": 401, "message": "api-key 错误"}"#;
        let env: HunterEnvelope = serde_json::from_str(json).unwrap();
        assert!(!env.is_ok());
        assert_eq!(env.code, 401);
        assert!(env.data.is_none());
        assert_eq!(env.error_msg(), "api-key 错误");
    }

    #[test]
    fn row_default_when_fields_missing() {
        let json = r#"{}"#;
        let row: HunterRow = serde_json::from_str(json).unwrap();
        assert!(row.ip.is_none());
        assert!(row.company.is_none());
        assert!(row.component.is_empty());
    }

    #[test]
    fn row_accepts_real_null_component() {
        let json = r#"{
            "ip": "1.1.1.1",
            "component": null,
            "header_server": "cloudflare",
            "as_org": "CLOUDFLARENET",
            "number": "京ICP证030173号",
            "ssl_certificate": "Subject: CN=example.com",
            "cert_sha256": "abc123"
        }"#;
        let row: HunterRow = serde_json::from_str(json).unwrap();
        assert!(row.component.is_empty());
        assert_eq!(row.header_server.as_deref(), Some("cloudflare"));
        assert_eq!(row.as_org.as_deref(), Some("CLOUDFLARENET"));
        assert_eq!(row.icp_number.as_deref(), Some("京ICP证030173号"));
    }

    #[test]
    fn component_pair_parses() {
        let json = r#"{"name": "nginx", "version": "1.21.4"}"#;
        let c: HunterComponent = serde_json::from_str(json).unwrap();
        assert_eq!(c.name.as_deref(), Some("nginx"));
        assert_eq!(c.version.as_deref(), Some("1.21.4"));
    }
}
