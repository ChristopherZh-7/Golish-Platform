use super::*;
use golish_integrations::schema::{
    Field, FieldType, IntegrationGroup, IntegrationSchema, Storage, TestKind, VaultStorage,
};
use std::collections::HashMap;

fn fake_schema_one_secret() -> IntegrationSchema {
    IntegrationSchema {
        category: "asm".into(),
        display_name: "demo".into(),
        description: None,
        help_url: None,
        storage: Storage::Vault {
            vault: VaultStorage::default(),
        },
        groups: vec![IntegrationGroup {
            id: "default".into(),
            name: "API Key".into(),
            description: None,
            icon: None,
            help_url: None,
            test: Some(TestKind::Builtin),
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
    }
}

fn pick_value_test_group() -> IntegrationGroup {
    IntegrationGroup {
        id: "default".into(),
        name: "x".into(),
        description: None,
        icon: None,
        help_url: None,
        test: None,
        capture: None,
        fields: vec![
            Field {
                key: "label_only".into(),
                label: "Label".into(),
                field_type: FieldType::Text,
                placeholder: None,
                required: false,
                rows: None,
                options: vec![],
                pattern: None,
            },
            Field {
                key: "api_key".into(),
                label: "API Key".into(),
                field_type: FieldType::SecretText,
                placeholder: None,
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            },
        ],
    }
}

#[test]
fn pick_credential_prefers_secret_field_over_first_field() {
    let group = pick_value_test_group();
    let mut cleartext = HashMap::new();
    cleartext.insert("label_only".into(), "ignored-label".into());
    cleartext.insert("api_key".into(), "real-secret".into());
    let v = pick_credential_value(&group, &cleartext);
    assert_eq!(v.as_deref(), Some("real-secret"));
}

#[test]
fn pick_credential_returns_none_when_secret_missing_from_cleartext() {
    let group = pick_value_test_group();
    let cleartext = HashMap::new();
    let v = pick_credential_value(&group, &cleartext);
    assert!(v.is_none(), "no cleartext entry → None, not empty string");
}

#[test]
fn connection_status_ok_with_quota_appended_to_message() {
    let s = ConnectionStatus::Ok {
        message: "0.zone validated".into(),
        quota_remaining: Some(42),
        quota_total: Some(100),
    };
    let h = connection_status_to_health(s);
    assert_eq!(h.status, golish_integrations::HealthStatus::Healthy);
    assert!(h.message.contains("0.zone validated"));
    assert!(h.message.contains("42/100"));
}

#[test]
fn connection_status_auth_failed_maps_to_invalid() {
    let s = ConnectionStatus::AuthFailed {
        message: "bad key".into(),
    };
    let h = connection_status_to_health(s);
    assert_eq!(h.status, golish_integrations::HealthStatus::Invalid);
    assert_eq!(h.message, "bad key");
}

#[test]
fn connection_status_quota_exhausted_maps_to_rate_limited() {
    let s = ConnectionStatus::QuotaExhausted {
        message: "quota out".into(),
    };
    let h = connection_status_to_health(s);
    assert_eq!(h.status, golish_integrations::HealthStatus::RateLimited);
    assert_eq!(h.message, "quota out");
}

#[test]
fn connection_status_network_error_maps_to_unknown() {
    let s = ConnectionStatus::NetworkError {
        message: "dns failure".into(),
    };
    let h = connection_status_to_health(s);
    assert_eq!(h.status, golish_integrations::HealthStatus::Unknown);
    assert_eq!(h.message, "dns failure");
}

#[tokio::test]
async fn intel_dispatcher_unknown_tool_returns_unknown_health() {
    let dispatcher = IntelBuiltinDispatcher {
        providers: Arc::new(HashMap::new()),
    };
    let schema = fake_schema_one_secret();
    let cleartext = HashMap::new();
    let h = dispatcher
        .dispatch("missing-id", "default", &schema, &cleartext)
        .await
        .unwrap();
    assert_eq!(h.status, golish_integrations::HealthStatus::Unknown);
    assert!(h.message.contains("missing-id"));
}

#[tokio::test]
async fn intel_dispatcher_bad_group_returns_validation_error() {
    // Register a real provider so the dispatcher gets past the
    // "missing-id" early return and reaches the group lookup.
    let p: Arc<dyn IntelProvider> = Arc::new(ZoneProvider::default());
    let mut providers = HashMap::new();
    providers.insert("0.zone".to_string(), p);
    let dispatcher = IntelBuiltinDispatcher {
        providers: Arc::new(providers),
    };
    let schema = fake_schema_one_secret();
    let cleartext = HashMap::new();
    let err = dispatcher
        .dispatch("0.zone", "nonexistent", &schema, &cleartext)
        .await
        .unwrap_err();
    match err {
        IntegrationError::Validation(m) => assert!(m.contains("nonexistent")),
        other => panic!("expected Validation, got {other:?}"),
    }
}
