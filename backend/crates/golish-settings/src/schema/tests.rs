use super::*;

#[test]
fn test_default_settings() {
    let settings = GolishSettings::default();
    assert_eq!(settings.version, 1);
    assert_eq!(settings.ai.default_provider, AiProvider::VertexAi);
    assert_eq!(settings.ai.default_model, "claude-opus-4-5@20251101");
    assert_eq!(settings.ui.theme, Theme::Dark);
    assert_eq!(settings.advanced.log_level, LogLevel::Info);
    assert_eq!(settings.terminal.font_size, 14);
    assert!(settings.agent.session_persistence);
}

#[test]
fn test_parse_minimal_toml() {
    let toml = r#"
        version = 1
        [ai]
        default_provider = "openrouter"
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert_eq!(settings.ai.default_provider, AiProvider::Openrouter);
    // Defaults should fill in missing fields
    assert_eq!(settings.terminal.font_size, 14);
}

#[test]
fn test_serialize_settings() {
    let settings = GolishSettings::default();
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    assert!(toml_str.contains("version = 1"));
    assert!(toml_str.contains("[ai]"));
}

#[test]
fn test_context_settings_defaults() {
    let context = ContextSettings::default();
    assert!(context.enabled);
    assert!((context.compaction_threshold - 0.80).abs() < f64::EPSILON);
    assert_eq!(context.protected_turns, 2);
    assert_eq!(context.cooldown_seconds, 60);
}

#[test]
fn test_context_settings_deserialize_from_toml() {
    let toml = r#"
        [context]
        enabled = false
        compaction_threshold = 0.75
        protected_turns = 3
        cooldown_seconds = 120
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert!(!settings.context.enabled);
    assert!((settings.context.compaction_threshold - 0.75).abs() < f64::EPSILON);
    assert_eq!(settings.context.protected_turns, 3);
    assert_eq!(settings.context.cooldown_seconds, 120);
}

#[test]
fn test_context_settings_missing_section_uses_defaults() {
    // Test backward compatibility: missing [context] section should use defaults
    let toml = r#"
        version = 1
        [ai]
        default_provider = "anthropic"
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    // Context settings should have defaults
    assert!(settings.context.enabled);
    assert!((settings.context.compaction_threshold - 0.80).abs() < f64::EPSILON);
    assert_eq!(settings.context.protected_turns, 2);
    assert_eq!(settings.context.cooldown_seconds, 60);
}

#[test]
fn test_context_settings_partial_section_fills_defaults() {
    // Test that partial [context] section fills in missing fields with defaults
    let toml = r#"
        [context]
        enabled = false
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert!(!settings.context.enabled);
    // Other fields should have defaults
    assert!((settings.context.compaction_threshold - 0.80).abs() < f64::EPSILON);
    assert_eq!(settings.context.protected_turns, 2);
    assert_eq!(settings.context.cooldown_seconds, 60);
}

#[test]
fn test_summarizer_model_setting() {
    let toml = r#"
        [ai]
        summarizer_model = "claude-haiku-4-5@20251001"
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert_eq!(
        settings.ai.summarizer_model,
        Some("claude-haiku-4-5@20251001".to_string())
    );
}

#[test]
fn test_summarizer_model_defaults_to_none() {
    let toml = r#"
        [ai]
        default_provider = "anthropic"
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert!(settings.ai.summarizer_model.is_none());
}

#[test]
fn test_caret_settings_defaults() {
    let caret = CaretSettings::default();
    assert_eq!(caret.style, "default");
    assert!((caret.width - 1.0).abs() < f64::EPSILON);
    assert!(caret.color.is_none());
    assert!((caret.blink_speed - 530.0).abs() < f64::EPSILON);
    assert!((caret.opacity - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_caret_settings_in_terminal_defaults() {
    let terminal = TerminalSettings::default();
    assert_eq!(terminal.caret.style, "default");
    assert!((terminal.caret.width - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_caret_settings_deserialize_from_toml() {
    let toml = r##"
        [terminal.caret]
        style = "block"
        width = 2.0
        color = "#ff0000"
        blink_speed = 800.0
        opacity = 0.8
    "##;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert_eq!(settings.terminal.caret.style, "block");
    assert!((settings.terminal.caret.width - 2.0).abs() < f64::EPSILON);
    assert_eq!(settings.terminal.caret.color, Some("#ff0000".to_string()));
    assert!((settings.terminal.caret.blink_speed - 800.0).abs() < f64::EPSILON);
    assert!((settings.terminal.caret.opacity - 0.8).abs() < f64::EPSILON);
}

#[test]
fn test_caret_settings_missing_uses_defaults() {
    let toml = r#"
        [terminal]
        font_size = 16
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert_eq!(settings.terminal.caret.style, "default");
    assert!((settings.terminal.caret.width - 1.0).abs() < f64::EPSILON);
    assert!(settings.terminal.caret.color.is_none());
}

#[test]
fn test_caret_settings_partial_fills_defaults() {
    let toml = r#"
        [terminal.caret]
        style = "block"
    "#;

    let settings: GolishSettings = toml::from_str(toml).unwrap();
    assert_eq!(settings.terminal.caret.style, "block");
    // Other fields should have defaults
    assert!((settings.terminal.caret.width - 1.0).abs() < f64::EPSILON);
    assert!(settings.terminal.caret.color.is_none());
    assert!((settings.terminal.caret.blink_speed - 530.0).abs() < f64::EPSILON);
    assert!((settings.terminal.caret.opacity - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_caret_settings_color_none_not_serialized() {
    let settings = GolishSettings::default();
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // color is None, so it should not appear in output (skip_serializing_if)
    assert!(!toml_str.contains("color"));
}

// =========================================================================
// OpenRouter Provider Preferences Tests
// =========================================================================

#[test]
fn test_openrouter_preferences_default_is_none() {
    let settings = GolishSettings::default();
    assert!(settings.ai.openrouter.provider_preferences.is_none());
}

#[test]
fn test_openrouter_preferences_is_empty() {
    let prefs = OpenRouterProviderPreferences::default();
    assert!(prefs.is_empty());
}

#[test]
fn test_openrouter_preferences_not_empty_with_order() {
    let mut prefs = OpenRouterProviderPreferences::default();
    prefs.order = Some(vec!["deepinfra".to_string()]);
    assert!(!prefs.is_empty());
}

#[test]
fn test_openrouter_preferences_skips_none_fields_in_serialization() {
    let settings = OpenRouterSettings {
        api_key: Some("test-key".to_string()),
        show_in_selector: true,
        provider_preferences: None,
    };
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // provider_preferences is None, so it should not appear in output
    assert!(!toml_str.contains("provider_preferences"));
}

#[test]
fn test_openrouter_preferences_skips_empty_some_in_serialization() {
    let settings = OpenRouterSettings {
        api_key: Some("test-key".to_string()),
        show_in_selector: true,
        provider_preferences: Some(OpenRouterProviderPreferences::default()),
    };
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // provider_preferences is Some but empty (all fields None), so it should not appear
    assert!(!toml_str.contains("provider_preferences"));
}

#[test]
fn test_openrouter_preferences_serializes_when_has_values() {
    let settings = OpenRouterSettings {
        api_key: Some("test-key".to_string()),
        show_in_selector: true,
        provider_preferences: Some(OpenRouterProviderPreferences {
            sort: Some("throughput".to_string()),
            ..Default::default()
        }),
    };
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // provider_preferences has values, so it MUST appear in output
    assert!(
        toml_str.contains("provider_preferences"),
        "Non-empty provider_preferences was dropped! Output:\n{}",
        toml_str
    );
    assert!(toml_str.contains("throughput"));
}

#[test]
fn test_openrouter_preferences_round_trip_toml() {
    let toml_str = r#"
        [provider_preferences]
        order = ["deepinfra", "deepseek"]
        sort = "throughput"
        quantizations = ["fp8"]
        zdr = true
        allow_fallbacks = false
        data_collection = "deny"
        max_price_prompt = 0.30
        max_price_completion = 0.50
    "#;

    let settings: OpenRouterSettings = toml::from_str(toml_str).unwrap();
    let prefs = settings.provider_preferences.unwrap();
    assert_eq!(
        prefs.order,
        Some(vec!["deepinfra".to_string(), "deepseek".to_string()])
    );
    assert_eq!(prefs.sort, Some("throughput".to_string()));
    assert_eq!(prefs.quantizations, Some(vec!["fp8".to_string()]));
    assert_eq!(prefs.zdr, Some(true));
    assert_eq!(prefs.allow_fallbacks, Some(false));
    assert_eq!(prefs.data_collection, Some("deny".to_string()));
    assert!((prefs.max_price_prompt.unwrap() - 0.30).abs() < f64::EPSILON);
    assert!((prefs.max_price_completion.unwrap() - 0.50).abs() < f64::EPSILON);
}

#[test]
fn test_openrouter_preferences_partial_toml() {
    // Only some fields set - others should be None
    let toml_str = r#"
        [provider_preferences]
        order = ["deepinfra"]
    "#;

    let settings: OpenRouterSettings = toml::from_str(toml_str).unwrap();
    let prefs = settings.provider_preferences.unwrap();
    assert_eq!(prefs.order, Some(vec!["deepinfra".to_string()]));
    assert!(prefs.only.is_none());
    assert!(prefs.ignore.is_none());
    assert!(prefs.sort.is_none());
    assert!(prefs.zdr.is_none());
    assert!(prefs.quantizations.is_none());
}

#[test]
fn test_openrouter_settings_with_preferences_in_full_config() {
    let toml_str = r#"
        [ai]
        default_provider = "openrouter"
        default_model = "deepseek/deepseek-v3.2"

        [ai.openrouter]
        api_key = "sk-or-v1-test"

        [ai.openrouter.provider_preferences]
        order = ["deepinfra", "deepseek"]
        sort = "throughput"
        quantizations = ["fp8"]
    "#;

    let settings: GolishSettings = toml::from_str(toml_str).unwrap();
    assert_eq!(settings.ai.default_provider, AiProvider::Openrouter);
    assert_eq!(settings.ai.openrouter.api_key, Some("sk-or-v1-test".to_string()));
    let prefs = settings.ai.openrouter.provider_preferences.unwrap();
    assert_eq!(
        prefs.order,
        Some(vec!["deepinfra".to_string(), "deepseek".to_string()])
    );
    assert_eq!(prefs.sort, Some("throughput".to_string()));
    assert!(!prefs.is_empty());
}

#[test]
fn test_openrouter_preferences_full_round_trip_preserves_prefs() {
    // Simulate: user has provider_preferences set in settings.toml
    let toml_str = r#"
        [ai]
        default_provider = "openrouter"
        default_model = "deepseek/deepseek-v3.2"

        [ai.openrouter]
        api_key = "sk-or-v1-test"

        [ai.openrouter.provider_preferences]
        order = ["deepinfra", "deepseek"]
        sort = "throughput"
    "#;

    // Step 1: Load from TOML (simulating app startup)
    let settings: GolishSettings = toml::from_str(toml_str).unwrap();
    assert!(settings.ai.openrouter.provider_preferences.is_some());

    // Step 2: Serialize back to TOML (simulating save_window_state)
    let serialized = toml::to_string_pretty(&settings).unwrap();

    // Step 3: Verify the provider_preferences section is preserved
    assert!(
        serialized.contains("provider_preferences"),
        "provider_preferences section was lost during round-trip! Serialized:\n{}",
        serialized
    );

    // Step 4: Parse back and verify data integrity
    let reloaded: GolishSettings = toml::from_str(&serialized).unwrap();
    let prefs = reloaded.ai.openrouter.provider_preferences.unwrap();
    assert_eq!(
        prefs.order,
        Some(vec!["deepinfra".to_string(), "deepseek".to_string()])
    );
    assert_eq!(prefs.sort, Some("throughput".to_string()));
}

#[test]
fn test_openrouter_preferences_json_round_trip_preserves_prefs() {
    // Simulate: settings go through JSON (Tauri frontend<->backend)
    let toml_str = r#"
        [ai]
        default_provider = "openrouter"

        [ai.openrouter]
        api_key = "sk-or-v1-test"

        [ai.openrouter.provider_preferences]
        order = ["deepinfra"]
        sort = "throughput"
    "#;

    // Step 1: Load from TOML
    let settings: GolishSettings = toml::from_str(toml_str).unwrap();

    // Step 2: Serialize to JSON (simulating Tauri sending to frontend)
    let json_str = serde_json::to_string(&settings).unwrap();

    // Step 3: Deserialize from JSON (simulating Tauri receiving from frontend)
    let from_json: GolishSettings = serde_json::from_str(&json_str).unwrap();

    // Step 4: Serialize to TOML (simulating settings save)
    let toml_output = toml::to_string_pretty(&from_json).unwrap();

    // Step 5: Verify provider_preferences survived the round trip
    assert!(
        toml_output.contains("provider_preferences"),
        "provider_preferences lost during JSON round-trip! TOML output:\n{}",
        toml_output
    );

    let final_settings: GolishSettings = toml::from_str(&toml_output).unwrap();
    let prefs = final_settings.ai.openrouter.provider_preferences.unwrap();
    assert_eq!(prefs.order, Some(vec!["deepinfra".to_string()]));
    assert_eq!(prefs.sort, Some("throughput".to_string()));
}
