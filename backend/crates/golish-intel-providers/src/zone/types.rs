//! Wire-format response structures for the 0.zone API.
//!
//! API: `POST https://0.zone/api/data/`
//!
//! All responses share a common envelope:
//! ```json
//! { "code": 0, "data": [...], "message": "..." }
//! ```
//! where `code == 0` indicates success and any non-zero value is an error
//! (typically auth / quota issues, message contains the human-readable reason).

use serde::Deserialize;

/// Common envelope for every 0.zone API response.
///
/// `data` is kept as a generic [`serde_json::Value`] vector so we can route
/// to the correct per-query_type struct in [`super::mapper`].
#[derive(Debug, Deserialize)]
pub struct ZoneEnvelope {
    pub code: i32,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Vec<serde_json::Value>,
}

impl ZoneEnvelope {
    /// Returns true when the API reports success.
    pub fn is_ok(&self) -> bool {
        self.code == 0
    }
}

/// `query_type=site` — 信息系统.
#[derive(Debug, Deserialize, Default)]
pub struct SiteEntry {
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status_code: Option<i32>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub cms: Option<String>,
}

/// `query_type=domain` — 子域名.
///
/// 0.zone's domain response is similar to site but without the operator/cms
/// detail. We accept both shapes (`url` or `host`) defensively.
#[derive(Debug, Deserialize, Default)]
pub struct DomainEntry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

/// `query_type=email` — 邮箱.
#[derive(Debug, Deserialize, Default)]
pub struct EmailEntry {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_type: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

/// `query_type=apk` — 移动端应用.
#[derive(Debug, Deserialize, Default)]
pub struct ApkEntry {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    /// Nested object containing WeChat info etc.
    #[serde(default)]
    pub msg: Option<ApkMsg>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ApkMsg {
    #[serde(default)]
    pub wechat_id: Option<String>,
    #[serde(default, rename = "iconUrl")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

/// `query_type=sensitive` — 敏感目录.
#[derive(Debug, Deserialize, Default)]
pub struct SensitiveEntry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub device_type: Option<String>,
}

/// `query_type=code` — 代码/文档泄漏.
#[derive(Debug, Deserialize, Default)]
pub struct CodeEntry {
    #[serde(default)]
    pub code_url: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
}

/// `query_type=member` — 人员.
#[derive(Debug, Deserialize, Default)]
pub struct MemberEntry {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_success_parses() {
        let json = r#"{"code": 0, "data": [{"ip": "1.2.3.4"}], "message": "ok"}"#;
        let env: ZoneEnvelope = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        assert_eq!(env.data.len(), 1);
    }

    #[test]
    fn envelope_error_parses() {
        let json = r#"{"code": 1, "message": "API key invalid"}"#;
        let env: ZoneEnvelope = serde_json::from_str(json).unwrap();
        assert!(!env.is_ok());
        assert_eq!(env.message, "API key invalid");
        assert!(env.data.is_empty());
    }

    #[test]
    fn site_entry_parses() {
        let json = r#"{
            "ip": "1.2.3.4",
            "url": "https://example.com",
            "title": "Example",
            "status_code": 200,
            "group": "Example Corp",
            "operator": "ChinaTelecom",
            "cms": "WordPress"
        }"#;
        let e: SiteEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(e.group.as_deref(), Some("Example Corp"));
        assert_eq!(e.cms.as_deref(), Some("WordPress"));
    }

    #[test]
    fn apk_entry_with_nested_msg() {
        let json = r#"{
            "title": "MyApp",
            "source": "Android",
            "group": "Corp",
            "msg": {"wechat_id": "wx123", "iconUrl": "https://x.com/icon.png", "code": "abc"}
        }"#;
        let e: ApkEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.title.as_deref(), Some("MyApp"));
        let msg = e.msg.unwrap();
        assert_eq!(msg.wechat_id.as_deref(), Some("wx123"));
    }

    #[test]
    fn entries_default_when_fields_missing() {
        let json = r#"{}"#;
        let e: SiteEntry = serde_json::from_str(json).unwrap();
        assert!(e.ip.is_none());
        assert!(e.group.is_none());
    }
}
