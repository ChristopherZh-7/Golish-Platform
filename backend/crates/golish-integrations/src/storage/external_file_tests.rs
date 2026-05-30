
use super::*;
use crate::schema::{Field, FieldType, IntegrationGroup, IntegrationSchema};
use tempfile::TempDir;

fn enscan_tyc_schema(path: &Path) -> IntegrationSchema {
    IntegrationSchema {
        category: "enterprise-intel".into(),
        display_name: "ENScan_GO".into(),
        description: None,
        storage: Storage::ExternalFile {
            external_file: ExternalFileStorage {
                path: path.to_string_lossy().into(),
                format: ExternalFileFormat::Yaml,
                preserve_unknown_keys: true,
                backup_on_write: true,
            },
        },
        help_url: None,
        groups: vec![IntegrationGroup {
            id: "tyc".into(),
            name: "TYC".into(),
            description: None,
            icon: None,
            help_url: None,
            test: None,
            capture: None,
            fields: vec![
                Field {
                    key: "cookies.tyc".into(),
                    label: "Cookie".into(),
                    field_type: FieldType::SecretTextarea,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                },
                Field {
                    key: "tyc.tycid".into(),
                    label: "tycid".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                },
                Field {
                    key: "tyc.auth_token".into(),
                    label: "auth_token".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                },
            ],
        }],
    }
}

fn aqc_schema(path: &Path, preserve: bool) -> IntegrationSchema {
    IntegrationSchema {
        category: "enterprise-intel".into(),
        display_name: "ENScan_GO".into(),
        description: None,
        storage: Storage::ExternalFile {
            external_file: ExternalFileStorage {
                path: path.to_string_lossy().into(),
                format: ExternalFileFormat::Yaml,
                preserve_unknown_keys: preserve,
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
    }
}

#[tokio::test]
async fn write_creates_yaml_when_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "BAIDUID=1;BDUSS=2".into());
    backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("cookies:"));
    assert!(text.contains("aqc:"));
    assert!(text.contains("BAIDUID=1;BDUSS=2"));
}

#[tokio::test]
async fn write_preserves_unknown_keys() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        r#"
# user's hand-written config
log_level: debug
proxy:
  enabled: true
  url: http://127.0.0.1:8080
cookies:
  aqc: old_cookie
"#,
    )
    .unwrap();

    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "new_cookie".into());
    backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap();

    let parsed: YamlValue = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // Schema key updated:
    assert_eq!(
        parsed
            .get("cookies")
            .and_then(|v| v.get("aqc"))
            .and_then(|v| v.as_str()),
        Some("new_cookie")
    );
    // Unrelated user keys preserved:
    assert_eq!(
        parsed.get("log_level").and_then(|v| v.as_str()),
        Some("debug")
    );
    assert_eq!(
        parsed
            .get("proxy")
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        parsed
            .get("proxy")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str()),
        Some("http://127.0.0.1:8080")
    );
}

#[tokio::test]
async fn write_three_field_group_makes_nested_yaml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = enscan_tyc_schema(&path);
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.tyc".into(), "TYC_COOKIE".into());
    fields.insert("tyc.tycid".into(), "TYCID123".into());
    fields.insert("tyc.auth_token".into(), "AUTH456".into());
    backend
        .write("enscan-go", "tyc", &schema, fields)
        .await
        .unwrap();

    let parsed: YamlValue = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        parsed
            .get("cookies")
            .and_then(|v| v.get("tyc"))
            .and_then(|v| v.as_str()),
        Some("TYC_COOKIE")
    );
    assert_eq!(
        parsed
            .get("tyc")
            .and_then(|v| v.get("tycid"))
            .and_then(|v| v.as_str()),
        Some("TYCID123")
    );
    assert_eq!(
        parsed
            .get("tyc")
            .and_then(|v| v.get("auth_token"))
            .and_then(|v| v.as_str()),
        Some("AUTH456")
    );
}

#[tokio::test]
async fn read_returns_secret_set_without_value() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "cookies:\n  aqc: BAIDUID=1\n").unwrap();
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let result = backend.read("enscan-go", "aqc", &schema).await.unwrap();
    let v = result.get("cookies.aqc").expect("field present");
    assert!(v.has_value);
    assert_eq!(v.value, None, "secret field MUST NOT surface plaintext");
}

#[tokio::test]
async fn read_returns_empty_for_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let result = backend.read("enscan-go", "aqc", &schema).await.unwrap();
    let v = result.get("cookies.aqc").expect("field declared");
    assert!(!v.has_value);
}

#[tokio::test]
async fn read_cleartext_surfaces_actual_value() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "cookies:\n  aqc: BAIDUID=1\n").unwrap();
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let result = backend
        .read_cleartext("enscan-go", "aqc", &schema)
        .await
        .unwrap();
    assert_eq!(
        result.get("cookies.aqc").map(String::as_str),
        Some("BAIDUID=1")
    );
}

#[tokio::test]
async fn clear_removes_only_declared_fields() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        r#"
log_level: debug
cookies:
  aqc: keep_user_other  # this will be wiped (it's our schema key)
  unrelated: untouched
"#,
    )
    .unwrap();
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    backend.clear("enscan-go", "aqc", &schema).await.unwrap();

    let parsed: YamlValue = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        parsed.get("log_level").and_then(|v| v.as_str()),
        Some("debug"),
        "non-schema keys must survive clear()"
    );
    assert!(
        parsed.get("cookies").and_then(|v| v.get("aqc")).is_none(),
        "schema key must be removed"
    );
    assert_eq!(
        parsed
            .get("cookies")
            .and_then(|v| v.get("unrelated"))
            .and_then(|v| v.as_str()),
        Some("untouched"),
        "sibling user keys in the same map must survive"
    );
}

#[tokio::test]
async fn write_validates_required_fields() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    // empty value for required field — should reject
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "   ".into());
    let err = backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap_err();
    assert!(matches!(err, IntegrationError::Validation(_)));
    // file must NOT be created
    assert!(!path.exists(), "must not write when validation fails");
}

#[tokio::test]
async fn write_rejects_unknown_field_keys() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "ok".into());
    fields.insert("not.in.schema".into(), "bad".into());
    let err = backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap_err();
    assert!(matches!(err, IntegrationError::Validation(_)));
}

#[tokio::test]
async fn backup_keeps_max_three_rolling() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    for i in 0..5 {
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), format!("cookie_{i}"));
        backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap();
        // bump mtime so backups don't get lumped into the same sort bucket
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    // After 5 writes: 1 main file + at most 3 backups
    let bak_count = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.contains(".bak.")
        })
        .count();
    assert!(
        bak_count <= MAX_BACKUPS,
        "expected at most {MAX_BACKUPS} backups, got {bak_count}"
    );
}

#[tokio::test]
async fn atomic_write_no_leftover_tmp() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "v".into());
    backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap();
    let leftover = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
    assert!(!leftover, "tmp file must be renamed away, not left behind");
}

#[tokio::test]
async fn json_format_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    let mut schema = aqc_schema(&path, true);
    if let Storage::ExternalFile { external_file } = &mut schema.storage {
        external_file.format = ExternalFileFormat::Json;
    }
    let backend = ExternalFileBackend::new();
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "JSON_COOKIE".into());
    backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        parsed.pointer("/cookies/aqc").and_then(|v| v.as_str()),
        Some("JSON_COOKIE")
    );

    // read back works
    let result = backend
        .read_cleartext("enscan-go", "aqc", &schema)
        .await
        .unwrap();
    assert_eq!(
        result.get("cookies.aqc").map(String::as_str),
        Some("JSON_COOKIE")
    );
}

#[tokio::test]
async fn corrupt_yaml_returns_external_file_corrupt_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "not: valid: yaml: [unclosed").unwrap();
    let schema = aqc_schema(&path, true);
    let backend = ExternalFileBackend::new();
    let err = backend.read("enscan-go", "aqc", &schema).await.unwrap_err();
    match err {
        IntegrationError::ExternalFileCorrupt { reason, .. } => {
            assert!(reason.contains("YAML"));
        }
        other => panic!("expected ExternalFileCorrupt, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_non_external_file_storage() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.yaml");
    let mut schema = aqc_schema(&path, true);
    schema.storage = Storage::vault_default();
    let backend = ExternalFileBackend::new();
    let err = backend.read("enscan-go", "aqc", &schema).await.unwrap_err();
    assert!(matches!(err, IntegrationError::Validation(_)));
}

#[tokio::test]
async fn tools_dir_template_expands_to_real_path() {
    // Schema declares `{{tools_dir}}/ENScan_GO/config.yaml`. The
    // backend was built with `with_tools_dir(tmp/tools)`. Writing
    // must drop the file under that real path.
    let dir = TempDir::new().unwrap();
    let tools_dir = dir.path().join("tools");
    std::fs::create_dir_all(&tools_dir).unwrap();
    let templated = "{{tools_dir}}/ENScan_GO/config.yaml";
    let mut schema = aqc_schema(Path::new(templated), true);
    if let Storage::ExternalFile { external_file } = &mut schema.storage {
        external_file.path = templated.into();
    }
    let backend = ExternalFileBackend::new().with_tools_dir(tools_dir.clone());
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "expanded_ok".into());
    backend
        .write("enscan-go", "aqc", &schema, fields)
        .await
        .unwrap();

    let actual_path = tools_dir.join("ENScan_GO").join("config.yaml");
    assert!(
        actual_path.exists(),
        "expanded path {} should exist",
        actual_path.display()
    );
    let parsed: YamlValue =
        serde_yaml::from_str(&std::fs::read_to_string(&actual_path).unwrap()).unwrap();
    assert_eq!(
        parsed
            .get("cookies")
            .and_then(|v| v.get("aqc"))
            .and_then(|v| v.as_str()),
        Some("expanded_ok")
    );
}

#[tokio::test]
async fn tools_dir_template_without_hint_stays_literal() {
    // If the backend has no `tools_dir` hint, the path is left
    // with the literal `{{tools_dir}}` token. write() then fails
    // because the `{` directory can't be created (or, if it
    // can, the path is obviously wrong — both are louder failure
    // modes than silently writing to a default location).
    let dir = TempDir::new().unwrap();
    let templated = format!(
        "{}/{{{{tools_dir}}}}/ENScan_GO/config.yaml",
        dir.path().display()
    );
    let mut schema = aqc_schema(Path::new(&templated), true);
    if let Storage::ExternalFile { external_file } = &mut schema.storage {
        external_file.path = templated.clone();
    }
    let backend = ExternalFileBackend::new(); // no with_tools_dir
    let mut fields = HashMap::new();
    fields.insert("cookies.aqc".into(), "x".into());
    let res = backend.write("enscan-go", "aqc", &schema, fields).await;
    // The literal directory `{{tools_dir}}/ENScan_GO/` is unlikely
    // to be createable on most platforms (braces fine but path
    // semantically nonsense). Either way, the resulting file path
    // must contain the unexpanded token.
    if let Ok(()) = res {
        assert!(
            std::fs::read_dir(dir.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("{{")),
            "without tools_dir hint, template stays literal"
        );
    }
}
