//! Map 0.zone wire-format responses to uniform [`ProviderRecord`]s.
//!
//! Each `query_type` has its own mapper that knows which fields to extract
//! and which normalized keys to put them under. Normalized keys match what
//! `output_store::store_organization_update` expects (see baseline doc §2).

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::{
    ApkEntry, CodeEntry, DomainEntry, EmailEntry, MemberEntry, OrgEntry, SiteEntry,
};

const PROVIDER: &str = "0.zone";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

fn insert_nonempty(fields: &mut HashMap<String, String>, key: &str, val: Option<String>) {
    if let Some(v) = val {
        let v = v.trim();
        if !v.is_empty() {
            fields.insert(key.to_string(), v.to_string());
        }
    }
}

fn stringify_json_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(stringify_json_value).collect();
            (!parts.is_empty()).then(|| parts.join(","))
        }
        serde_json::Value::Object(_) => Some(value.to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
    }
}

fn org_name_from<'a>(group: Option<&'a String>, company: Option<&'a String>) -> Option<&'a String> {
    group
        .filter(|v| !v.trim().is_empty())
        .or_else(|| company.filter(|v| !v.trim().is_empty()))
}

fn domain_from_email(email: &str) -> Option<String> {
    email
        .split_once('@')
        .map(|(_, domain)| domain.trim().to_string())
        .filter(|domain| !domain.is_empty())
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
        fields.insert("status_code".into(), sc.into_string());
    }
    if let Some(port) = entry.port {
        fields.insert("port".into(), port.into_string());
    }
    insert_if_present(
        &mut fields,
        "organization_name",
        org_name_from(entry.group.as_ref(), entry.company.as_ref()),
    );
    insert_if_present(&mut fields, "operator", entry.operator.as_ref());
    insert_if_present(&mut fields, "cms", entry.cms.as_ref());
    insert_if_present(&mut fields, "os", entry.os.as_ref());
    insert_if_present(&mut fields, "protocol", entry.protocol.as_ref());
    insert_if_present(&mut fields, "service", entry.service.as_ref());
    insert_if_present(&mut fields, "asn", entry.asn.as_ref());
    insert_if_present(&mut fields, "asn_org", entry.asn_org.as_ref());
    insert_if_present(&mut fields, "cert", entry.ssl_certificate.as_ref());
    insert_if_present(&mut fields, "beian", entry.beian.as_ref());
    insert_if_present(&mut fields, "cname", entry.cname.as_ref());
    insert_if_present(&mut fields, "country", entry.country.as_ref());
    insert_if_present(&mut fields, "province", entry.province.as_ref());
    insert_if_present(&mut fields, "city", entry.city.as_ref());
    if let Some(is_cdn) = entry.is_cdn {
        let v = is_cdn.into_string();
        if v == "1" || v.eq_ignore_ascii_case("true") {
            fields.insert("cdn".into(), "cdn".into());
        }
        fields.insert("is_cdn".into(), v);
    }
    let webserver_name = entry
        .server_name
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .or(entry.component.as_ref().filter(|v| !v.trim().is_empty()));
    let webserver_version = entry
        .server_version
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .or(entry.versions.as_ref().filter(|v| !v.trim().is_empty()));
    match (webserver_name, webserver_version) {
        (Some(name), Some(version)) => {
            fields.insert("webserver".into(), format!("{name}/{version}"));
        }
        (Some(name), None) => {
            fields.insert("webserver".into(), name.clone());
        }
        _ => {}
    }
    let mut technologies = Vec::new();
    if let Some(cms) = entry.cms.filter(|v| !v.trim().is_empty() && v != "unknown") {
        technologies.push(cms);
    }
    if let Some(component) = entry.component.filter(|v| !v.trim().is_empty()) {
        technologies.push(component);
    }
    if !technologies.is_empty() {
        if let Ok(json) = serde_json::to_string(&technologies) {
            fields.insert("technologies".into(), json);
        }
    }
    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

/// `domain` mapping: subdomain → `domain` field, prefer `url` over `host`.
pub fn map_domain(entry: DomainEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    let domain = entry.domain.or(entry.url).or(entry.host);
    insert_if_present(&mut fields, "domain", domain.as_ref());
    insert_if_present(&mut fields, "title", entry.title.as_ref());
    insert_if_present(
        &mut fields,
        "organization_name",
        org_name_from(entry.group.as_ref(), entry.company.as_ref()),
    );
    insert_if_present(&mut fields, "root_domain", entry.root_domain.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Domain, fields, raw)
}

/// `email` mapping.
pub fn map_email(entry: EmailEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "email_address", entry.email.as_ref());
    insert_if_present(
        &mut fields,
        "email",
        entry.mail_domain.as_ref().or(entry.email.as_ref()),
    );
    insert_if_present(&mut fields, "email_domain", entry.mail_domain.as_ref());
    insert_if_present(&mut fields, "email_type", entry.email_type.as_ref());
    insert_if_present(
        &mut fields,
        "organization_name",
        org_name_from(entry.group.as_ref(), entry.company.as_ref()),
    );
    if let Some(n) = entry.leakage_num {
        fields.insert("leakage_num".into(), n.into_string());
    }
    insert_if_present(&mut fields, "leakage_time", entry.leakage_time.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Email, fields, raw)
}

/// `apk` mapping: title/source + WeChat msg if present.
pub fn map_apk(entry: ApkEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "app_name", entry.title.as_ref());
    insert_if_present(
        &mut fields,
        "app_source",
        entry.source.as_ref().or(entry.app_type.as_ref()),
    );
    insert_if_present(
        &mut fields,
        "organization_name",
        org_name_from(entry.group.as_ref(), entry.company.as_ref()),
    );
    insert_if_present(&mut fields, "beian", entry.icp.as_ref());
    if let Some(msg) = entry.msg {
        insert_if_present(&mut fields, "wechat_id", msg.wechat_id.as_ref());
        insert_if_present(&mut fields, "app_icon_url", msg.icon_url.as_ref());
        insert_if_present(&mut fields, "app_code", msg.code.as_ref());
        insert_if_present(&mut fields, "app_url", msg.app_url.as_ref());
        insert_if_present(&mut fields, "app_id", msg.app_id.as_ref());
        insert_if_present(&mut fields, "app_intro", msg.introduction.as_ref());
    }
    ProviderRecord::new(PROVIDER, QueryType::Apk, fields, raw)
}

/// `code` mapping: code leak URL + source (github/gitee/...).
pub fn map_code(entry: CodeEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    let code_url = entry.code_url.or(entry.url);
    insert_if_present(&mut fields, "code_url", code_url.as_ref());
    insert_if_present(&mut fields, "code_name", entry.name.as_ref());
    insert_nonempty(
        &mut fields,
        "code_keyword",
        entry.keyword.as_ref().and_then(stringify_json_value),
    );
    insert_if_present(&mut fields, "code_source", entry.source.as_ref());
    insert_if_present(
        &mut fields,
        "organization_name",
        org_name_from(entry.group.as_ref(), entry.company.as_ref()),
    );
    insert_if_present(&mut fields, "code_path", entry.path.as_ref());
    insert_if_present(&mut fields, "code_extension", entry.file_extension.as_ref());
    insert_if_present(&mut fields, "code_detail", entry.code_detail.as_ref());
    insert_nonempty(
        &mut fields,
        "code_owner",
        entry.owner.as_ref().and_then(stringify_json_value),
    );
    insert_nonempty(
        &mut fields,
        "code_repository",
        entry.repository.as_ref().and_then(stringify_json_value),
    );
    // If source is github, also surface as github_org for organizations.github_orgs.
    if matches!(entry.source.as_deref(), Some("github") | Some("github.com")) {
        if let Some(url) = code_url {
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

/// `org` mapping: enterprise profile → organization fields.
pub fn map_org(entry: OrgEntry, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    let org_name = entry
        .name_cn
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .or(entry.company.as_ref().filter(|v| !v.trim().is_empty()))
        .or(entry.name_home.as_ref().filter(|v| !v.trim().is_empty()))
        .or(entry.group.as_ref().filter(|v| !v.trim().is_empty()));
    insert_if_present(&mut fields, "organization_name", org_name);
    insert_if_present(&mut fields, "alias", entry.name_en.as_ref());
    insert_if_present(&mut fields, "alias", entry.name_home.as_ref());
    insert_if_present(&mut fields, "subsidiary", entry.org_parent.as_ref());
    insert_if_present(&mut fields, "subsidiary", entry.parent_company.as_ref());
    insert_if_present(&mut fields, "org_source", entry.source.as_ref());
    if let Some(msg) = entry.msg {
        insert_if_present(&mut fields, "credit_code", msg.code.as_ref());
        insert_if_present(&mut fields, "industry", msg.industry.as_ref());
        insert_if_present(&mut fields, "contact_phone", msg.contact_number.as_ref());
        insert_if_present(&mut fields, "legal_person", msg.legal_person.as_ref());
        insert_if_present(&mut fields, "reg_address", msg.reg_address.as_ref());
        insert_if_present(&mut fields, "registered_capital", msg.capital.as_ref());
        insert_if_present(&mut fields, "business_scope", msg.business.as_ref());
        if let Some(website) = msg.website.into_iter().find(|v| !v.trim().is_empty()) {
            fields.insert("domain".into(), website);
        }
        if let Some(email) = msg.email.into_iter().find(|v| !v.trim().is_empty()) {
            fields.insert("email_address".into(), email.clone());
            fields.insert("email".into(), domain_from_email(&email).unwrap_or(email));
        }
        if let Some(ip) = msg.ip.into_iter().find(|v| !v.trim().is_empty()) {
            fields.insert("ip".into(), ip);
        }
        if let Some(alias) = msg.name_before.into_iter().find(|v| !v.trim().is_empty()) {
            fields.insert("alias".into(), alias);
        }
        if let Some(subsidiary) = msg
            .relation_company
            .iter()
            .filter_map(stringify_json_value)
            .find(|v| !v.trim().is_empty())
        {
            fields.insert("subsidiary".into(), subsidiary);
        }
    }
    ProviderRecord::new(PROVIDER, QueryType::Org, fields, raw)
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
            status_code: Some(super::super::types::StringOrNumber::I64(200)),
            port: None,
            group: Some("Acme Corp".into()),
            company: None,
            operator: Some("ChinaTelecom".into()),
            cms: Some("WordPress".into()),
            component: None,
            versions: None,
            server_name: None,
            server_version: None,
            os: None,
            protocol: None,
            service: None,
            asn: None,
            asn_org: None,
            ssl_certificate: None,
            is_cdn: None,
            beian: None,
            cname: None,
            country: None,
            province: None,
            city: None,
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
            domain: None,
            url: Some("sub.example.com".into()),
            host: Some("other.example.com".into()),
            title: None,
            group: None,
            company: None,
            root_domain: None,
        };
        let rec = map_domain(entry, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "sub.example.com");
    }

    #[test]
    fn domain_mapper_accepts_real_api_domain_key() {
        let entry: DomainEntry = serde_json::from_value(serde_json::json!({
            "domain": "ipc.lc.duer.baidu.com",
            "url": "ipc.lc.duer.baidu.com",
            "root_domain": "baidu.com",
            "company": "Beijing Baidu Netcom Science & Technology Co.,Ltd"
        }))
        .unwrap();
        let rec = map_domain(entry, raw_object());
        assert_eq!(
            rec.fields.get("domain").map(String::as_str),
            Some("ipc.lc.duer.baidu.com")
        );
        assert_eq!(
            rec.fields.get("organization_name").map(String::as_str),
            Some("Beijing Baidu Netcom Science & Technology Co.,Ltd")
        );
    }

    #[test]
    fn email_mapper_basic() {
        let entry = EmailEntry {
            email: Some("a@b.com".into()),
            mail_domain: Some("b.com".into()),
            email_type: Some("work".into()),
            group: Some("Corp".into()),
            company: None,
            leakage_num: None,
            leakage_time: None,
        };
        let rec = map_email(entry, raw_object());
        assert_eq!(rec.fields.get("email").unwrap(), "b.com");
        assert_eq!(rec.fields.get("email_address").unwrap(), "a@b.com");
        assert_eq!(rec.fields.get("email_type").unwrap(), "work");
    }

    #[test]
    fn email_mapper_uses_mail_domain_for_email_domains() {
        let entry: EmailEntry = serde_json::from_value(serde_json::json!({
            "email": "alice@example.com",
            "email_type": "企业邮箱",
            "mail_domain": "example.com",
            "company": "Example Corp"
        }))
        .unwrap();
        let rec = map_email(entry, raw_object());
        assert_eq!(
            rec.fields.get("email_domain").map(String::as_str),
            Some("example.com")
        );
        assert_eq!(
            rec.fields.get("email_address").map(String::as_str),
            Some("alice@example.com")
        );
        assert_eq!(
            rec.fields.get("organization_name").map(String::as_str),
            Some("Example Corp")
        );
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
    fn site_mapper_accepts_real_string_status_and_extended_fields() {
        let entry: SiteEntry = serde_json::from_value(serde_json::json!({
            "ip": "39.98.59.137",
            "port": "9001",
            "url": "http://39.98.59.137:9001",
            "title": "Example Title",
            "status_code": "200",
            "component": "nginx",
            "versions": "1.20.1",
            "server_name": "Nginx",
            "server_version": "1.20.1",
            "os": "Linux",
            "cms": "WordPress",
            "operator": "阿里云",
            "asn": "AS37963",
            "ssl_certificate": "CN=example.com",
            "company": "Example Corp"
        }))
        .unwrap();
        let rec = map_site(entry, raw_object());
        assert_eq!(
            rec.fields.get("status_code").map(String::as_str),
            Some("200")
        );
        assert_eq!(rec.fields.get("port").map(String::as_str), Some("9001"));
        assert_eq!(
            rec.fields.get("webserver").map(String::as_str),
            Some("Nginx/1.20.1")
        );
        assert_eq!(rec.fields.get("os").map(String::as_str), Some("Linux"));
        assert_eq!(rec.fields.get("asn").map(String::as_str), Some("AS37963"));
        assert_eq!(
            rec.fields.get("cert").map(String::as_str),
            Some("CN=example.com")
        );
        assert_eq!(
            rec.fields.get("organization_name").map(String::as_str),
            Some("Example Corp")
        );
        assert!(rec.fields.get("technologies").is_some());
    }

    #[test]
    fn code_mapper_extracts_github_org_from_source() {
        let entry = CodeEntry {
            code_url: Some("https://github.com/acmecorp/internal-tool".into()),
            url: None,
            name: Some("internal-tool".into()),
            keyword: Some(serde_json::json!("apikey")),
            source: Some("github".into()),
            group: Some("Acme Corp".into()),
            company: None,
            path: None,
            owner: None,
            repository: None,
            file_extension: None,
            code_detail: None,
        };
        let rec = map_code(entry, raw_object());
        assert_eq!(rec.fields.get("github_org").unwrap(), "acmecorp");
        assert_eq!(rec.fields.get("code_source").unwrap(), "github");
    }

    #[test]
    fn code_mapper_accepts_real_github_source_and_url_key() {
        let entry: CodeEntry = serde_json::from_value(serde_json::json!({
            "url": "https://github.com/acmecorp/internal-tool/blob/main/config.yml",
            "name": "config.yml",
            "keyword": ["apikey"],
            "source": "github.com"
        }))
        .unwrap();
        let rec = map_code(entry, raw_object());
        assert_eq!(
            rec.fields.get("github_org").map(String::as_str),
            Some("acmecorp")
        );
        assert_eq!(
            rec.fields.get("code_url").map(String::as_str),
            Some("https://github.com/acmecorp/internal-tool/blob/main/config.yml")
        );
    }

    #[test]
    fn code_mapper_skips_github_org_when_source_not_github() {
        let entry = CodeEntry {
            code_url: Some("https://gitee.com/x/y".into()),
            url: None,
            name: None,
            keyword: None,
            source: Some("gitee".into()),
            group: None,
            company: None,
            path: None,
            owner: None,
            repository: None,
            file_extension: None,
            code_detail: None,
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
    fn org_mapper_extracts_enterprise_profile_fields() {
        let entry: OrgEntry = serde_json::from_value(serde_json::json!({
            "name_cn": "北京百度网讯科技有限公司",
            "name_en": "Baidu Online Network Technology Beijing Co Ltd",
            "source": "tianyancha.com",
            "msg": {
                "code": "91110000802100433B",
                "industry": "互联网",
                "contact_number": "010-59928888",
                "legal_person": "梁志祥",
                "reg_address": "北京市海淀区",
                "website": ["www.baidu.com"],
                "email": ["contact@baidu.com"],
                "name_before": ["百度在线网络技术（北京）有限公司"]
            }
        }))
        .unwrap();
        let rec = map_org(entry, raw_object());
        assert_eq!(
            rec.fields.get("organization_name").map(String::as_str),
            Some("北京百度网讯科技有限公司")
        );
        assert_eq!(
            rec.fields.get("credit_code").map(String::as_str),
            Some("91110000802100433B")
        );
        assert_eq!(
            rec.fields.get("industry").map(String::as_str),
            Some("互联网")
        );
        assert_eq!(
            rec.fields.get("domain").map(String::as_str),
            Some("www.baidu.com")
        );
        assert_eq!(
            rec.fields.get("email").map(String::as_str),
            Some("baidu.com")
        );
    }

    #[test]
    fn empty_entry_yields_empty_fields() {
        let entry = SiteEntry::default();
        let rec = map_site(entry, raw_object());
        assert!(rec.fields.is_empty());
    }
}
