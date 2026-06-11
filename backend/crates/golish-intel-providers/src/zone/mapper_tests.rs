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
    assert!(rec.fields.contains_key("technologies"));
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
