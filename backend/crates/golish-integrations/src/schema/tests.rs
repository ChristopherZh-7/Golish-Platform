use super::*;

#[test]
fn schema_serde_round_trip_minimal() {
    let s = IntegrationSchema {
        category: "asm-cyberspace".into(),
        display_name: "0.zone".into(),
        description: None,
        storage: Storage::vault_default(),
        groups: vec![IntegrationGroup {
            id: "default".into(),
            name: "API Key".into(),
            description: None,
            icon: None,
            help_url: None,
            fields: vec![Field {
                key: "api_key".into(),
                label: "API Key".into(),
                field_type: FieldType::SecretText,
                placeholder: None,
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            }],
            test: Some(TestKind::Builtin),
            capture: None,
        }],
        help_url: None,
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: IntegrationSchema = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn storage_external_file_yaml() {
    let raw = r#"{
        "type": "external_file",
        "external_file": {
            "path": "~/.config/enscan/config.yaml",
            "format": "yaml",
            "preserve_unknown_keys": true,
            "backup_on_write": true
        }
    }"#;
    let s: Storage = serde_json::from_str(raw).unwrap();
    match s {
        Storage::ExternalFile { external_file: ef } => {
            assert_eq!(ef.path, "~/.config/enscan/config.yaml");
            assert_eq!(ef.format, ExternalFileFormat::Yaml);
            assert!(ef.preserve_unknown_keys);
            assert!(ef.backup_on_write);
        }
        _ => panic!("expected ExternalFile, got {:?}", s),
    }
}

#[test]
fn storage_settings() {
    let raw = r#"{"type":"settings","settings":{"key":"network.github_token"}}"#;
    let s: Storage = serde_json::from_str(raw).unwrap();
    match s {
        Storage::Settings { settings } => {
            assert_eq!(settings.key, "network.github_token");
        }
        _ => panic!("expected Settings"),
    }
}

#[test]
fn storage_vault_default_round_trip() {
    let s = Storage::vault_default();
    let json = serde_json::to_string(&s).unwrap();
    // Vault with empty extra_tags should still serialize cleanly.
    let back: Storage = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn test_kind_exec() {
    let raw = r#"{
        "kind": "exec",
        "cmd": "{{exec}} -n 小米 -type aqc",
        "ok_regex": "company_name",
        "timeout_secs": 10
    }"#;
    let t: TestKind = serde_json::from_str(raw).unwrap();
    match t {
        TestKind::Exec {
            cmd,
            ok_regex,
            fail_regex,
            timeout_secs,
        } => {
            assert!(cmd.contains("{{exec}}"));
            assert_eq!(ok_regex, "company_name");
            assert_eq!(fail_regex, None);
            assert_eq!(timeout_secs, 10);
        }
        _ => panic!("expected Exec"),
    }
}

#[test]
fn test_kind_http() {
    let raw = r#"{
        "kind": "http",
        "method": "GET",
        "url": "https://api.github.com/user",
        "headers": { "Authorization": "Bearer {{value:token}}" }
    }"#;
    let t: TestKind = serde_json::from_str(raw).unwrap();
    match t {
        TestKind::Http {
            method,
            url,
            headers,
            ok_status_range,
            timeout_secs,
        } => {
            assert_eq!(method, "GET");
            assert_eq!(url, "https://api.github.com/user");
            assert!(headers.contains_key("Authorization"));
            assert_eq!(ok_status_range, (200, 299));
            assert_eq!(timeout_secs, 30);
        }
        _ => panic!("expected Http"),
    }
}

#[test]
fn field_type_is_secret() {
    assert!(FieldType::SecretText.is_secret());
    assert!(FieldType::SecretTextarea.is_secret());
    assert!(!FieldType::Text.is_secret());
    assert!(!FieldType::Url.is_secret());
    assert!(!FieldType::Boolean.is_secret());
}

// ────────────────────────────────────────────────────────────────
// CaptureRecipe / CaptureRule tests (Phase 1 T1.1)
// ────────────────────────────────────────────────────────────────

#[test]
fn capture_recipe_round_trip_cookie_rule() {
    let raw = r#"{
        "login_url": "https://aiqicha.baidu.com",
        "success_url_pattern": "aiqicha\\.baidu\\.com/(home|company)",
        "timeout_secs": 300,
        "rules": [
            { "type": "cookie", "domain": ".aiqicha.baidu.com",
              "name": "BDUSS", "target_field": "cookies.aqc" }
        ]
    }"#;
    let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
    assert_eq!(r.login_url, "https://aiqicha.baidu.com");
    assert_eq!(r.timeout_secs, 300);
    assert_eq!(r.rules.len(), 1);
    match &r.rules[0] {
        CaptureRule::Cookie {
            domain,
            name,
            target_field,
            required,
        } => {
            assert_eq!(domain, ".aiqicha.baidu.com");
            assert_eq!(name, "BDUSS");
            assert_eq!(target_field, "cookies.aqc");
            assert!(*required);
        }
        other => panic!("expected Cookie, got {other:?}"),
    }
    let back: CaptureRecipe = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(r, back);
}

#[test]
fn capture_recipe_defaults() {
    // Need r##"..."## because the JSON contains `#api-key` which
    // would otherwise close the r#"..."# delimiter at the `"#`.
    let raw = r##"{
        "login_url": "https://fofa.info",
        "rules": [
            { "type": "page_content", "selector": "#api-key",
              "target_field": "api_key" }
        ]
    }"##;
    let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
    assert_eq!(r.timeout_secs, 300, "default timeout_secs is 300");
    match &r.rules[0] {
        CaptureRule::PageContent {
            wait_ms, required, ..
        } => {
            assert_eq!(*wait_ms, 3000, "default wait_ms is 3000");
            assert!(*required, "default required is true");
        }
        other => panic!("expected PageContent, got {other:?}"),
    }
}

#[test]
fn capture_rule_helper_methods() {
    let cookie = CaptureRule::Cookie {
        domain: ".x.com".into(),
        name: "Y".into(),
        target_field: "f.a".into(),
        required: true,
    };
    assert_eq!(cookie.target_field(), "f.a");
    assert!(cookie.required());
    assert_eq!(cookie.kind_name(), "cookie");

    let cookie_joined = CaptureRule::CookieJoined {
        domain: ".x.com".into(),
        names: vec!["a".into(), "b".into()],
        sep: "; ".into(),
        fmt: "{name}={value}".into(),
        target_field: "f.b".into(),
        required: false,
        required_names: vec![],
        min_count: 0,
    };
    assert_eq!(cookie_joined.target_field(), "f.b");
    assert!(!cookie_joined.required());
    assert_eq!(cookie_joined.kind_name(), "cookie_joined");

    let ls = CaptureRule::LocalStorage {
        key: "k".into(),
        target_field: "f.c".into(),
        required: true,
    };
    assert_eq!(ls.kind_name(), "local_storage");

    let ss = CaptureRule::SessionStorage {
        key: "k".into(),
        target_field: "f.d".into(),
        required: true,
    };
    assert_eq!(ss.kind_name(), "session_storage");

    let pc = CaptureRule::PageContent {
        selector: "#x".into(),
        attribute: Some("data-token".into()),
        wait_ms: 1000,
        target_field: "f.e".into(),
        required: true,
    };
    assert_eq!(pc.kind_name(), "page_content");

    let uq = CaptureRule::UrlQuery {
        name: "code".into(),
        target_field: "f.f".into(),
        required: true,
    };
    assert_eq!(uq.kind_name(), "url_query");
}

#[test]
fn integration_group_capture_defaults_to_none() {
    let raw = r#"{
        "id": "default",
        "name": "API Key",
        "fields": [
            { "key": "api_key", "label": "API Key", "type": "secret_text" }
        ]
    }"#;
    let g: IntegrationGroup = serde_json::from_str(raw).unwrap();
    assert!(g.capture.is_none(), "capture defaults to None when absent");
    assert!(g.test.is_none(), "(sanity) test also still optional");
}

#[test]
fn integration_group_with_capture_round_trip() {
    let g = IntegrationGroup {
        id: "aqc".into(),
        name: "爱企查".into(),
        description: None,
        icon: None,
        help_url: None,
        fields: vec![Field {
            key: "cookies.aqc".into(),
            label: "Cookie".into(),
            field_type: FieldType::SecretTextarea,
            placeholder: None,
            required: true,
            rows: None,
            options: vec![],
            pattern: None,
        }],
        test: None,
        capture: Some(CaptureRecipe {
            login_url: "https://aiqicha.baidu.com".into(),
            success_url_pattern: Some(r"aiqicha\.baidu\.com".into()),
            visit_url: None,
            instructions: None,
            timeout_secs: 300,
            rules: vec![CaptureRule::Cookie {
                domain: ".aiqicha.baidu.com".into(),
                name: "BDUSS".into(),
                target_field: "cookies.aqc".into(),
                required: true,
            }],
        }),
    };
    let json = serde_json::to_string(&g).unwrap();
    let back: IntegrationGroup = serde_json::from_str(&json).unwrap();
    assert_eq!(g, back);
}
