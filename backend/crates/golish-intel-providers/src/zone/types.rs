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

/// String-or-integer value from 0.zone. Some fields (`status_code`, `port`)
/// are documented as numbers but observed as strings in production responses.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StringOrNumber {
    String(String),
    I64(i64),
    U64(u64),
}

impl StringOrNumber {
    pub fn into_string(self) -> String {
        match self {
            Self::String(s) => s,
            Self::I64(n) => n.to_string(),
            Self::U64(n) => n.to_string(),
        }
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
    pub status_code: Option<StringOrNumber>,
    #[serde(default)]
    pub port: Option<StringOrNumber>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub cms: Option<String>,
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub versions: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub asn: Option<String>,
    #[serde(default)]
    pub asn_org: Option<String>,
    #[serde(default)]
    pub ssl_certificate: Option<String>,
    #[serde(default)]
    pub is_cdn: Option<StringOrNumber>,
    #[serde(default)]
    pub beian: Option<String>,
    #[serde(default)]
    pub cname: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub province: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
}

/// `query_type=domain` — 子域名.
///
/// 0.zone's domain response is similar to site but without the operator/cms
/// detail. We accept both shapes (`url` or `host`) defensively.
#[derive(Debug, Deserialize, Default)]
pub struct DomainEntry {
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub root_domain: Option<String>,
}

/// `query_type=email` — 邮箱.
#[derive(Debug, Deserialize, Default)]
pub struct EmailEntry {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub mail_domain: Option<String>,
    #[serde(default)]
    pub email_type: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub leakage_num: Option<StringOrNumber>,
    #[serde(default)]
    pub leakage_time: Option<String>,
}

/// `query_type=apk` — 移动端应用.
#[derive(Debug, Deserialize, Default)]
pub struct ApkEntry {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub app_type: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub icp: Option<String>,
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
    #[serde(default)]
    pub app_url: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub introduction: Option<String>,
}

/// `query_type=code` — 代码/文档泄漏.
#[derive(Debug, Deserialize, Default)]
pub struct CodeEntry {
    #[serde(default)]
    pub code_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub keyword: Option<serde_json::Value>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub owner: Option<serde_json::Value>,
    #[serde(default)]
    pub repository: Option<serde_json::Value>,
    #[serde(default)]
    pub file_extension: Option<String>,
    #[serde(default)]
    pub code_detail: Option<String>,
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

/// `query_type=org` — 企业画像.
#[derive(Debug, Deserialize, Default)]
pub struct OrgEntry {
    #[serde(default)]
    pub name_cn: Option<String>,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_home: Option<String>,
    #[serde(default)]
    pub org_parent: Option<String>,
    #[serde(default)]
    pub parent_company: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub msg: Option<OrgMsg>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OrgMsg {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub industry: Option<String>,
    #[serde(default)]
    pub contact_number: Option<String>,
    #[serde(default)]
    pub legal_person: Option<String>,
    #[serde(default)]
    pub reg_address: Option<String>,
    #[serde(default)]
    pub capital: Option<String>,
    #[serde(default)]
    pub business: Option<String>,
    #[serde(default)]
    pub website: Vec<String>,
    #[serde(default)]
    pub relation_company: Vec<serde_json::Value>,
    #[serde(default)]
    pub name_before: Vec<String>,
    #[serde(default)]
    pub email: Vec<String>,
    #[serde(default)]
    pub ip: Vec<String>,
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
