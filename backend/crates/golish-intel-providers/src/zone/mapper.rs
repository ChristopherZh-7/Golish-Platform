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
#[path = "mapper_tests.rs"]
mod tests;
