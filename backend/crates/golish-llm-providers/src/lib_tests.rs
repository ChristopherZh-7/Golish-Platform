use super::*;
use golish_settings::schema::OpenRouterProviderPreferences;

#[test]
fn test_preferences_to_json_basic() {
    let prefs = OpenRouterProviderPreferences {
        order: Some(vec!["deepinfra".into(), "deepseek".into()]),
        sort: Some("throughput".into()),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    assert_eq!(
        provider.get("order").unwrap(),
        &serde_json::json!(["deepinfra", "deepseek"])
    );
    assert_eq!(
        provider.get("sort").unwrap(),
        &serde_json::json!("throughput")
    );
}

#[test]
fn test_preferences_to_json_filters() {
    let prefs = OpenRouterProviderPreferences {
        only: Some(vec!["deepinfra".into()]),
        ignore: Some(vec!["google vertex".into()]),
        allow_fallbacks: Some(false),
        zdr: Some(true),
        data_collection: Some("deny".into()),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    assert_eq!(
        provider.get("only").unwrap(),
        &serde_json::json!(["deepinfra"])
    );
    assert_eq!(
        provider.get("ignore").unwrap(),
        &serde_json::json!(["google vertex"])
    );
    assert_eq!(
        provider.get("allow_fallbacks").unwrap(),
        &serde_json::json!(false)
    );
    assert_eq!(provider.get("zdr").unwrap(), &serde_json::json!(true));
    assert_eq!(
        provider.get("data_collection").unwrap(),
        &serde_json::json!("deny")
    );
}

#[test]
fn test_preferences_to_json_max_price() {
    let prefs = OpenRouterProviderPreferences {
        max_price_prompt: Some(0.30),
        max_price_completion: Some(0.50),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    let max_price = provider.get("max_price").unwrap().as_object().unwrap();
    assert_eq!(max_price.get("prompt").unwrap(), &serde_json::json!(0.30));
    assert_eq!(
        max_price.get("completion").unwrap(),
        &serde_json::json!(0.50)
    );
}

#[test]
fn test_preferences_to_json_quantizations() {
    let prefs = OpenRouterProviderPreferences {
        quantizations: Some(vec!["fp8".into(), "fp16".into()]),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    assert_eq!(
        provider.get("quantizations").unwrap(),
        &serde_json::json!(["fp8", "fp16"])
    );
}

#[test]
fn test_preferences_to_json_empty() {
    let prefs = OpenRouterProviderPreferences::default();
    let json = openrouter_preferences_to_json(&prefs);
    assert!(json.get("provider").is_some());
}

#[test]
fn test_preferences_to_json_invalid_sort_ignored() {
    let prefs = OpenRouterProviderPreferences {
        sort: Some("invalid_sort".into()),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    assert!(provider.get("sort").is_none());
}

#[test]
fn test_preferences_to_json_invalid_quantization_ignored() {
    let prefs = OpenRouterProviderPreferences {
        quantizations: Some(vec!["invalid_quant".into()]),
        ..Default::default()
    };

    let json = openrouter_preferences_to_json(&prefs);
    let provider = json.get("provider").unwrap().as_object().unwrap();
    assert!(provider.get("quantizations").is_none());
}
