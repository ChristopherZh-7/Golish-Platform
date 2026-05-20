//! Map 0.zone wire-format responses to uniform [`ProviderRecord`]s.
//!
//! Each `query_type` has its own mapper that knows which fields to extract
//! and which normalized keys to put them under. Normalized keys match what
//! `output_store::store_organization_update` expects (see baseline doc §2).

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::{
    ApkEntry, CodeEntry, DomainEntry, EmailEntry, MemberEntry, SensitiveEntry, SiteEntry,
};

const PROVIDER: &str = "0.zone";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

/// `site` mapping:
/// - `ip` / `url` / `title` / `status_code` → target-style fields
/// - `group` → `organization_name` (downstream consumer can match/create org)
/// - `operator` → fingerprint (operator/network)
/// - `cms` → fingerprint (cms/tech)
pub fn map_site(entry: SiteEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "ip", entry.ip.as_ref());
    insert_if_present(&mut fields, "url", entry.url.as_ref());
    insert_if_present(&mut fields, "title", entry.title.as_ref());
    if let Some(sc) = entry.status_code {
        fields.insert("status_code".into(), sc.to_string());
    }
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    insert_if_present(&mut fields, "operator", entry.operator.as_ref());
    insert_if_present(&mut fields, "cms", entry.cms.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

/// `domain` mapping: subdomain → `domain` field, prefer `url` over `host`.
pub fn map_domain(entry: DomainEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    let domain = entry.url.or(entry.host);
    insert_if_present(&mut fields, "domain", domain.as_ref());
    insert_if_present(&mut fields, "title", entry.title.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Domain, fields, raw)
}

/// `email` mapping.
pub fn map_email(entry: EmailEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "email", entry.email.as_ref());
    insert_if_present(&mut fields, "email_type", entry.email_type.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Email, fields, raw)
}

/// `apk` mapping: title/source + WeChat msg if present.
pub fn map_apk(entry: ApkEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "app_name", entry.title.as_ref());
    insert_if_present(&mut fields, "app_source", entry.source.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    if let Some(msg) = entry.msg {
        insert_if_present(&mut fields, "wechat_id", msg.wechat_id.as_ref());
        insert_if_present(&mut fields, "app_icon_url", msg.icon_url.as_ref());
        insert_if_present(&mut fields, "app_code", msg.code.as_ref());
    }
    ProviderRecord::new(PROVIDER, QueryType::Apk, fields, raw)
}

/// `sensitive` mapping: sensitive path / device-type fingerprint.
pub fn map_sensitive(entry: SensitiveEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "url", entry.url.as_ref());
    insert_if_present(&mut fields, "title", entry.title.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    insert_if_present(&mut fields, "device_type", entry.device_type.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Sensitive, fields, raw)
}

/// `code` mapping: code leak URL + source (github/gitee/...).
pub fn map_code(entry: CodeEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "code_url", entry.code_url.as_ref());
    insert_if_present(&mut fields, "code_name", entry.name.as_ref());
    insert_if_present(&mut fields, "code_keyword", entry.keyword.as_ref());
    insert_if_present(&mut fields, "code_source", entry.source.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    // If source is github, also surface as github_org for organizations.github_orgs.
    if entry.source.as_deref() == Some("github") {
        if let Some(url) = entry.code_url {
            // Best-effort: extract org from github.com/<org>/...
            if let Some(after) = url.split("github.com/").nth(1) {
                if let Some(org) = after.split('/').next() {
                    if !org.is_empty() {
                        fields.insert("github_org".to_string(), org.to_string());
                    }
                }
            }
        }
    }
    ProviderRecord::new(PROVIDER, QueryType::Code, fields, raw)
}

/// `member` mapping: employee name + source.
pub fn map_member(entry: MemberEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "contact_name", entry.name.as_ref());
    insert_if_present(&mut fields, "contact_source", entry.source.as_ref());
    insert_if_present(&mut fields, "organization_name", entry.group.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Member, fields, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_object() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn site_mapper_extracts_all_fields() {
        let entry = SiteEntry {
            ip: Some("1.2.3.4".into()),
            url: Some("https://x.com".into()),
            title: Some("Hello".into()),
            status_code: Some(200),
            group: Some("Acme Corp".into()),
            operator: Some("ChinaTelecom".into()),
            cms: Some("WordPress".into()),
        };
        let rec = map_site(entry, raw_object());
        assert_eq!(rec.fields.get("ip").map(|s| s.as_str()), Some("1.2.3.4"));
        assert_eq!(rec.fields.get("title").map(|s| s.as_str()), Some("Hello"));
        assert_eq!(
            rec.fields.get("status_code").map(|s| s.as_str()),
            Some("200")
        );
        assert_eq!(
            rec.fields.get("organization_name").map(|s| s.as_str()),
            Some("Acme Corp")
        );
        assert_eq!(rec.fields.get("cms").map(|s| s.as_str()), Some("WordPress"));
        assert_eq!(rec.provider, "0.zone");
        assert_eq!(rec.query_type, QueryType::Site);
    }

    #[test]
    fn domain_mapper_prefers_url_over_host() {
        let entry = DomainEntry {
            url: Some("sub.example.com".into()),
            host: Some("other.example.com".into()),
            title: None,
            group: None,
        };
        let rec = map_domain(entry, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "sub.example.com");
    }

    #[test]
    fn email_mapper_basic() {
        let entry = EmailEntry {
            email: Some("a@b.com".into()),
            email_type: Some("work".into()),
            group: Some("Corp".into()),
        };
        let rec = map_email(entry, raw_object());
        assert_eq!(rec.fields.get("email").unwrap(), "a@b.com");
        assert_eq!(rec.fields.get("email_type").unwrap(), "work");
    }

    #[test]
    fn apk_mapper_flattens_msg() {
        let entry: ApkEntry = serde_json::from_value(serde_json::json!({
            "title": "MyApp",
            "source": "Android",
            "group": "Corp",
            "msg": {"wechat_id": "wx", "iconUrl": "https://x", "code": "c"}
        }))
        .unwrap();
        let rec = map_apk(entry, raw_object());
        assert_eq!(rec.fields.get("app_name").unwrap(), "MyApp");
        assert_eq!(rec.fields.get("wechat_id").unwrap(), "wx");
        assert_eq!(rec.fields.get("app_code").unwrap(), "c");
    }

    #[test]
    fn sensitive_mapper_includes_device_type() {
        let entry = SensitiveEntry {
            url: Some("https://x.com/admin".into()),
            title: Some("Admin Panel".into()),
            group: Some("Corp".into()),
            device_type: Some("router".into()),
        };
        let rec = map_sensitive(entry, raw_object());
        assert_eq!(rec.fields.get("device_type").unwrap(), "router");
    }

    #[test]
    fn code_mapper_extracts_github_org_from_source() {
        let entry = CodeEntry {
            code_url: Some("https://github.com/acmecorp/internal-tool".into()),
            name: Some("internal-tool".into()),
            keyword: Some("apikey".into()),
            source: Some("github".into()),
            group: Some("Acme Corp".into()),
        };
        let rec = map_code(entry, raw_object());
        assert_eq!(rec.fields.get("github_org").unwrap(), "acmecorp");
        assert_eq!(rec.fields.get("code_source").unwrap(), "github");
    }

    #[test]
    fn code_mapper_skips_github_org_when_source_not_github() {
        let entry = CodeEntry {
            code_url: Some("https://gitee.com/x/y".into()),
            name: None,
            keyword: None,
            source: Some("gitee".into()),
            group: None,
        };
        let rec = map_code(entry, raw_object());
        assert!(!rec.fields.contains_key("github_org"));
    }

    #[test]
    fn member_mapper_basic() {
        let entry = MemberEntry {
            name: Some("张三".into()),
            group: Some("Acme".into()),
            source: Some("linkedin".into()),
        };
        let rec = map_member(entry, raw_object());
        assert_eq!(rec.fields.get("contact_name").unwrap(), "张三");
        assert_eq!(rec.fields.get("contact_source").unwrap(), "linkedin");
    }

    #[test]
    fn empty_entry_yields_empty_fields() {
        let entry = SiteEntry::default();
        let rec = map_site(entry, raw_object());
        assert!(rec.fields.is_empty());
    }
}
