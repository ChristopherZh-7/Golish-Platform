use super::*;
use crate::schema::{
    ExternalFileFormat, ExternalFileStorage, Field, FieldType, IntegrationGroup, IntegrationSchema,
    Storage,
};
use tempfile::TempDir;

fn sample_integration_json(tool_id: &str) -> serde_json::Value {
    serde_json::json!({
        "tool": {
            "id": tool_id,
            "integration": {
                "category": "enterprise-intel",
                "display_name": format!("{tool_id} demo"),
                "storage": {
                    "type": "external_file",
                    "external_file": {
                        "path": "~/.config/demo/config.yaml",
                        "format": "yaml",
                        "preserve_unknown_keys": true,
                        "backup_on_write": true
                    }
                },
                "groups": [{
                    "id": "default",
                    "name": "Default",
                    "fields": [{
                        "key": "api_key",
                        "label": "API Key",
                        "type": "secret_text",
                        "required": true
                    }]
                }]
            }
        }
    })
}

fn in_code_provider(tool_id: &str) -> ResolvedIntegration {
    ResolvedIntegration {
        tool_id: tool_id.to_string(),
        schema: IntegrationSchema {
            category: "asm".into(),
            display_name: format!("{tool_id} in-code"),
            description: None,
            storage: Storage::vault_default(),
            help_url: None,
            groups: vec![IntegrationGroup {
                id: "default".into(),
                name: "API Key".into(),
                description: None,
                icon: None,
                help_url: None,
                test: None,
                capture: None,
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
            }],
        },
    }
}

#[tokio::test]
async fn collects_from_toolsconfig_only() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("enscan-go.json"),
        serde_json::to_string(&sample_integration_json("enscan-go")).unwrap(),
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
    let list = resolver.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].tool_id, "enscan-go");
}

#[tokio::test]
async fn collects_from_in_code_only() {
    let resolver: DefaultSchemaResolver =
        DefaultSchemaResolver::new(None::<&Path>, vec![in_code_provider("0.zone")]);
    let list = resolver.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].tool_id, "0.zone");
}

#[tokio::test]
async fn in_code_overrides_toolsconfig_for_same_id() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("0.zone.json"),
        serde_json::to_string(&sample_integration_json("0.zone")).unwrap(),
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![in_code_provider("0.zone")]);
    let list = resolver.list().await.unwrap();
    assert_eq!(list.len(), 1);
    // in-code wins → schema display_name should match the in-code version
    assert_eq!(list[0].schema.display_name, "0.zone in-code");
}

#[tokio::test]
async fn stable_ordering_by_id() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("zzz.json"),
        serde_json::to_string(&sample_integration_json("zzz")).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("aaa.json"),
        serde_json::to_string(&sample_integration_json("aaa")).unwrap(),
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
    let list = resolver.list().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].tool_id, "aaa");
    assert_eq!(list[1].tool_id, "zzz");
}

#[tokio::test]
async fn skips_tools_without_integration_field() {
    let dir = TempDir::new().unwrap();
    // Tool without `integration` field — should be silently skipped.
    std::fs::write(
        dir.path().join("plain.json"),
        r#"{"tool":{"id":"plain","name":"Plain"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("with_int.json"),
        serde_json::to_string(&sample_integration_json("with-int")).unwrap(),
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
    let list = resolver.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].tool_id, "with-int");
}

#[tokio::test]
async fn get_missing_returns_schema_not_found() {
    let resolver: DefaultSchemaResolver = DefaultSchemaResolver::new(None::<&Path>, vec![]);
    let err = resolver.get("not-here").await.unwrap_err();
    assert!(matches!(err, IntegrationError::SchemaNotFound(_)));
}

#[tokio::test]
async fn malformed_integration_schema_errors() {
    let dir = TempDir::new().unwrap();
    // `integration.storage` missing → invalid schema → Validation err
    std::fs::write(
        dir.path().join("bad.json"),
        r#"{"tool":{"id":"bad","integration":{"category":"x","display_name":"x","groups":[]}}}"#,
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
    let err = resolver.list().await.unwrap_err();
    assert!(matches!(err, IntegrationError::Validation(_)));
}

// ────────────────────────────────────────────────────────────────
// validate_capture tests (Phase 1 T1.4)
// ────────────────────────────────────────────────────────────────

fn group_with_capture(target_field: &str, login_url: &str) -> IntegrationGroup {
    use crate::schema::{CaptureRecipe, CaptureRule};
    IntegrationGroup {
        id: "default".into(),
        name: "Test".into(),
        description: None,
        icon: None,
        help_url: None,
        test: None,
        capture: Some(CaptureRecipe {
            login_url: login_url.into(),
            success_url_pattern: None,
            visit_url: None,
            instructions: None,
            timeout_secs: 60,
            rules: vec![CaptureRule::Cookie {
                domain: ".example.com".into(),
                name: "BDUSS".into(),
                target_field: target_field.into(),
                required: true,
            }],
        }),
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
    }
}

#[test]
fn validate_capture_accepts_valid_recipe() {
    let g = group_with_capture("cookies.aqc", "https://aiqicha.baidu.com");
    assert!(validate_capture(&g).is_ok());
}

#[test]
fn validate_capture_rejects_unknown_target_field() {
    let g = group_with_capture("missing_field", "https://aiqicha.baidu.com");
    let err = validate_capture(&g).unwrap_err();
    match err {
        IntegrationError::CaptureInvalidTargetField { rule_index, field } => {
            assert_eq!(rule_index, 0);
            assert_eq!(field, "missing_field");
        }
        other => panic!("expected CaptureInvalidTargetField, got {other:?}"),
    }
}

#[test]
fn validate_capture_rejects_javascript_url() {
    let g = group_with_capture("cookies.aqc", "javascript:alert(1)");
    let err = validate_capture(&g).unwrap_err();
    assert!(matches!(err, IntegrationError::CaptureInvalidUrl(_)));
}

#[test]
fn validate_capture_rejects_file_url() {
    let g = group_with_capture("cookies.aqc", "file:///etc/passwd");
    let err = validate_capture(&g).unwrap_err();
    let s = err.to_string();
    assert!(s.contains("CAPTURE_INVALID_URL"));
    assert!(s.contains("file"));
}

#[test]
fn validate_capture_skips_when_capture_is_none() {
    let mut g = group_with_capture("cookies.aqc", "https://aiqicha.baidu.com");
    g.capture = None;
    assert!(validate_capture(&g).is_ok());
}

/// Fixture sanity: load the real `resources/toolsconfig/enscan-go.json`
/// from the repo root via [`DefaultSchemaResolver::get`] and verify
/// the `aqc` group's capture recipe survives all of (a) JSON parse,
/// (b) IntegrationSchema deserialization, (c) validate_capture
/// cross-checks. This is the closest end-to-end smoke we can run
/// from within the integrations crate — catches Phase 5 T5.1
/// regressions if anyone edits the JSON and breaks the recipe
/// shape.
#[tokio::test]
async fn fixture_enscan_aqc_capture_recipe_loads() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("resources")
        .join("toolsconfig");
    if !toolsconfig_dir.exists() {
        // Skip outside a normal git checkout.
        eprintln!(
            "fixture skipped: toolsconfig dir not found at {}",
            toolsconfig_dir.display()
        );
        return;
    }
    let resolver = DefaultSchemaResolver::new(Some(toolsconfig_dir), vec![]);
    let resolved = resolver
        .get("enscan-go")
        .await
        .expect("enscan-go integration should load");
    let aqc = resolved
        .schema
        .groups
        .iter()
        .find(|g| g.id == "aqc")
        .expect("aqc group should exist");
    let recipe = aqc
        .capture
        .as_ref()
        .expect("aqc group should declare a capture recipe");
    assert!(
        recipe.login_url.starts_with("https://aiqicha.baidu.com"),
        "login_url should target aiqicha.baidu.com, got {}",
        recipe.login_url
    );
    let success_url_pattern = recipe
        .success_url_pattern
        .as_ref()
        .expect("AQC capture should trigger on login success URLs");
    let success_url_re =
        regex::Regex::new(success_url_pattern).expect("AQC success_url_pattern should compile");
    assert!(
        success_url_re.is_match("https://qiye.baidu.com/usercenter/personalcenter?fr=c1009"),
        "AQC success_url_pattern should match Baidu Enterprise post-login redirect, got {}",
        success_url_pattern
    );
    assert!(
        recipe.timeout_secs >= 30 && recipe.timeout_secs <= 900,
        "timeout_secs should be within engine clamp window, got {}",
        recipe.timeout_secs
    );
    assert!(!recipe.rules.is_empty(), "recipe should declare ≥1 rule");
    // After 2026-05-21 schema fix: AQC must harvest the **full**
    // baidu.com Cookie header (CookieJoined with names=[]) because
    // BDUSS alone trips aiqicha.baidu.com's safety wall. Target
    // field renamed to `cookies.aiqicha` to match the literal key
    // ENScan v2.0.5 reads from its yaml.
    let cookie_joined_all = recipe.rules.iter().any(|r| match r {
        crate::schema::CaptureRule::CookieJoined {
            domain,
            names,
            target_field,
            ..
        } => domain == ".baidu.com" && names.is_empty() && target_field == "cookies.aiqicha",
        _ => false,
    });
    assert!(
        cookie_joined_all,
        "AQC capture should join every baidu.com cookie into cookies.aiqicha"
    );
}

#[tokio::test]
async fn fixture_enscan_tyc_capture_uses_search_probe_url() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("resources")
        .join("toolsconfig");
    if !toolsconfig_dir.exists() {
        eprintln!(
            "fixture skipped: toolsconfig dir not found at {}",
            toolsconfig_dir.display()
        );
        return;
    }
    let resolver = DefaultSchemaResolver::new(Some(toolsconfig_dir), vec![]);
    let resolved = resolver
        .get("enscan-go")
        .await
        .expect("enscan-go integration should load");
    let tyc = resolved
        .schema
        .groups
        .iter()
        .find(|g| g.id == "tyc")
        .expect("tyc group should exist");
    let recipe = tyc
        .capture
        .as_ref()
        .expect("tyc group should declare a capture recipe");

    assert!(
            recipe.login_url.starts_with("https://www.tianyancha.com/search?"),
            "TYC capture should open a search probe URL so a logged-in session emits auth headers, got {}",
            recipe.login_url
        );
    for expected_key in ["cookies.tyc", "tyc.tycid", "tyc.auth_token"] {
        assert!(
            tyc.fields.iter().any(|field| field.key == expected_key),
            "TYC group should declare ENScan config key {expected_key}"
        );
        assert!(
            recipe
                .rules
                .iter()
                .any(|rule| rule.target_field() == expected_key),
            "TYC capture should write into ENScan config key {expected_key}"
        );
    }
    assert!(
        recipe.rules.iter().any(|r| matches!(
            r,
            crate::schema::CaptureRule::Cookie { domain, name, target_field, .. }
                if domain == ".tianyancha.com"
                    && name == "TYCID"
                    && target_field == "tyc.tycid"
        )),
        "TYC capture should read tycid from the TYCID cookie"
    );
    assert!(
        recipe.rules.iter().any(|r| matches!(
            r,
            crate::schema::CaptureRule::Cookie { domain, name, target_field, .. }
                if domain == ".tianyancha.com"
                    && name == "auth_token"
                    && target_field == "tyc.auth_token"
        )),
        "TYC capture should read auth_token from the auth_token cookie"
    );
}

#[tokio::test]
async fn fixture_enscan_kc_and_rb_require_login_state_proof() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join("..")
        .join("resources")
        .join("toolsconfig");
    if !toolsconfig_dir.exists() {
        eprintln!(
            "fixture skipped: toolsconfig dir not found at {}",
            toolsconfig_dir.display()
        );
        return;
    }
    let resolver = DefaultSchemaResolver::new(Some(toolsconfig_dir), vec![]);
    let resolved = resolver
        .get("enscan-go")
        .await
        .expect("enscan-go integration should load");

    let assert_min_count = |group_id: &str, expected_min_count: usize| {
        let group = resolved
            .schema
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .unwrap_or_else(|| panic!("{group_id} group should exist"));
        let recipe = group
            .capture
            .as_ref()
            .unwrap_or_else(|| panic!("{group_id} group should declare capture"));
        assert!(
                recipe.rules.iter().any(|r| matches!(
                    r,
                    crate::schema::CaptureRule::CookieJoined { min_count, .. }
                        if *min_count == expected_min_count
                )),
                "{group_id} should require at least {expected_min_count} cookies before capture succeeds"
            );
    };

    assert_min_count("kc", 2);
    assert_min_count("rb", 3);

    let kc = resolved
        .schema
        .groups
        .iter()
        .find(|g| g.id == "kc")
        .expect("kc group should exist");
    let kc_recipe = kc
        .capture
        .as_ref()
        .expect("kc group should declare capture");
    assert!(
            kc_recipe.rules.iter().any(|r| matches!(
                r,
                crate::schema::CaptureRule::CookieJoined { required_names, .. }
                    if required_names == &vec!["USERINFO".to_string(), "aso_ucenter".to_string()]
            )),
            "Qimai capture should require login-only cookies, not just anonymous synct/syncd/qm_check/PHPSESSID"
        );
}

// ────────────────────────────────────────────────────────────────
// Pre-existing tests below
// ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn schema_can_be_round_tripped_through_resolver() {
    // Sanity: a fully populated schema serialized to JSON should
    // load back identically (proves wire format stability).
    let original = IntegrationSchema {
        category: "enterprise-intel".into(),
        display_name: "ENScan".into(),
        description: Some("Demo".into()),
        storage: Storage::ExternalFile {
            external_file: ExternalFileStorage {
                path: "~/.config/enscan/config.yaml".into(),
                format: ExternalFileFormat::Yaml,
                preserve_unknown_keys: true,
                backup_on_write: true,
            },
        },
        help_url: None,
        groups: vec![IntegrationGroup {
            id: "aqc".into(),
            name: "AQC".into(),
            description: None,
            icon: None,
            help_url: None,
            test: None,
            capture: None,
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
        }],
    };
    let json = serde_json::json!({
        "tool": { "id": "enscan-go", "integration": original.clone() }
    });
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("enscan.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();
    let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
    let resolved = resolver.get("enscan-go").await.unwrap();
    assert_eq!(resolved.schema, original);
}
