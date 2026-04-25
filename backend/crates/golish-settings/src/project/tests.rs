//! Project settings tests.

use super::*;
use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_project_settings_defaults() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    let settings = manager.get().await;
    assert!(settings.ai.provider.is_none());
    assert!(settings.ai.model.is_none());
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_project_settings_save_load() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Set some values
    manager
        .set_model(
            AiProvider::Anthropic,
            "claude-sonnet-4-20250514".to_string(),
        )
        .await
        .unwrap();
    manager
        .set_agent_mode("auto-approve".to_string())
        .await
        .unwrap();

    // Reload and verify
    manager.reload().await.unwrap();
    let settings = manager.get().await;

    assert_eq!(settings.ai.provider, Some(AiProvider::Anthropic));
    assert_eq!(
        settings.ai.model,
        Some("claude-sonnet-4-20250514".to_string())
    );
    assert_eq!(settings.ai.agent_mode, Some("auto-approve".to_string()));
}

#[tokio::test]
async fn test_project_settings_clear() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Set and save
    manager
        .set_agent_mode("planning".to_string())
        .await
        .unwrap();
    assert!(manager.config_path().exists());

    // Clear
    manager.clear().await.unwrap();
    assert!(!manager.config_path().exists());

    let settings = manager.get().await;
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_toml_file_format() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Set values and save
    manager
        .set_model(AiProvider::Openai, "gpt-4".to_string())
        .await
        .unwrap();
    manager
        .set_agent_mode("planning".to_string())
        .await
        .unwrap();

    // Read the raw TOML file
    let toml_content = tokio::fs::read_to_string(manager.config_path())
        .await
        .unwrap();

    // Verify TOML format
    assert!(toml_content.contains("[ai]"));
    assert!(toml_content.contains("provider = \"openai\""));
    assert!(toml_content.contains("model = \"gpt-4\""));
    assert!(toml_content.contains("agent_mode = \"planning\""));

    // Verify it parses as valid TOML
    let parsed: ProjectSettings = toml::from_str(&toml_content).unwrap();
    assert_eq!(parsed.ai.provider, Some(AiProvider::Openai));
    assert_eq!(parsed.ai.model, Some("gpt-4".to_string()));
    assert_eq!(parsed.ai.agent_mode, Some("planning".to_string()));
}

#[tokio::test]
async fn test_load_existing_file() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".golish");
    tokio::fs::create_dir_all(&config_dir).await.unwrap();

    // Create a project.toml manually
    let toml_content = r#"
[ai]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
agent_mode = "auto-approve"
"#;
    let config_path = config_dir.join("project.toml");
    tokio::fs::write(&config_path, toml_content).await.unwrap();

    // Load with manager
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;
    let settings = manager.get().await;

    assert_eq!(settings.ai.provider, Some(AiProvider::Anthropic));
    assert_eq!(
        settings.ai.model,
        Some("claude-sonnet-4-20250514".to_string())
    );
    assert_eq!(settings.ai.agent_mode, Some("auto-approve".to_string()));
}

#[tokio::test]
async fn test_partial_settings_only_provider() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Set only provider and model
    manager
        .set_model(AiProvider::Gemini, "gemini-2.0-flash-exp".to_string())
        .await
        .unwrap();

    let settings = manager.get().await;
    assert_eq!(settings.ai.provider, Some(AiProvider::Gemini));
    assert_eq!(settings.ai.model, Some("gemini-2.0-flash-exp".to_string()));
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_partial_settings_only_agent_mode() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Set only agent mode
    manager.set_agent_mode("default".to_string()).await.unwrap();

    let settings = manager.get().await;
    assert!(settings.ai.provider.is_none());
    assert!(settings.ai.model.is_none());
    assert_eq!(settings.ai.agent_mode, Some("default".to_string()));
}

#[tokio::test]
async fn test_partial_settings_mixed() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".golish");
    tokio::fs::create_dir_all(&config_dir).await.unwrap();

    // Create partial settings (only model and agent_mode, no provider)
    let toml_content = r#"
[ai]
model = "test-model"
agent_mode = "planning"
"#;
    let config_path = config_dir.join("project.toml");
    tokio::fs::write(&config_path, toml_content).await.unwrap();

    let manager = ProjectSettingsManager::new(temp_dir.path()).await;
    let settings = manager.get().await;

    assert!(settings.ai.provider.is_none());
    assert_eq!(settings.ai.model, Some("test-model".to_string()));
    assert_eq!(settings.ai.agent_mode, Some("planning".to_string()));
}

#[tokio::test]
async fn test_update_ai_settings() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Update all settings at once
    manager
        .update_ai_settings(
            Some(AiProvider::Groq),
            Some("llama-3.3-70b-versatile".to_string()),
            Some("auto-approve".to_string()),
        )
        .await
        .unwrap();

    let settings = manager.get().await;
    assert_eq!(settings.ai.provider, Some(AiProvider::Groq));
    assert_eq!(
        settings.ai.model,
        Some("llama-3.3-70b-versatile".to_string())
    );
    assert_eq!(settings.ai.agent_mode, Some("auto-approve".to_string()));
}

#[tokio::test]
async fn test_update_ai_settings_partial() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // First set all values
    manager
        .update_ai_settings(
            Some(AiProvider::Anthropic),
            Some("claude-3-5-sonnet-20241022".to_string()),
            Some("planning".to_string()),
        )
        .await
        .unwrap();

    // Now update to set some to None
    manager
        .update_ai_settings(Some(AiProvider::Openai), None, Some("default".to_string()))
        .await
        .unwrap();

    let settings = manager.get().await;
    assert_eq!(settings.ai.provider, Some(AiProvider::Openai));
    assert!(settings.ai.model.is_none());
    assert_eq!(settings.ai.agent_mode, Some("default".to_string()));
}

#[tokio::test]
async fn test_persistence_across_instances() {
    let temp_dir = TempDir::new().unwrap();

    // Create first manager and save settings
    {
        let manager1 = ProjectSettingsManager::new(temp_dir.path()).await;
        manager1
            .set_model(AiProvider::Ollama, "qwen2.5-coder:32b".to_string())
            .await
            .unwrap();
        manager1
            .set_agent_mode("planning".to_string())
            .await
            .unwrap();
    }

    // Create new manager and verify it loads saved settings
    let manager2 = ProjectSettingsManager::new(temp_dir.path()).await;
    let settings = manager2.get().await;

    assert_eq!(settings.ai.provider, Some(AiProvider::Ollama));
    assert_eq!(settings.ai.model, Some("qwen2.5-coder:32b".to_string()));
    assert_eq!(settings.ai.agent_mode, Some("planning".to_string()));
}

#[tokio::test]
async fn test_malformed_toml_fallback() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".golish");
    tokio::fs::create_dir_all(&config_dir).await.unwrap();

    // Write malformed TOML
    let malformed_toml = r#"
[ai
provider = "anthropic
model = invalid syntax
"#;
    let config_path = config_dir.join("project.toml");
    tokio::fs::write(&config_path, malformed_toml)
        .await
        .unwrap();

    // Manager should fall back to defaults without panicking
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;
    let settings = manager.get().await;

    assert!(settings.ai.provider.is_none());
    assert!(settings.ai.model.is_none());
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_malformed_toml_with_invalid_enum() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".golish");
    tokio::fs::create_dir_all(&config_dir).await.unwrap();

    // Write TOML with invalid provider enum value
    let invalid_enum_toml = r#"
[ai]
provider = "invalid_provider"
model = "some-model"
"#;
    let config_path = config_dir.join("project.toml");
    tokio::fs::write(&config_path, invalid_enum_toml)
        .await
        .unwrap();

    // Manager should fall back to defaults
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;
    let settings = manager.get().await;

    assert!(settings.ai.provider.is_none());
    assert!(settings.ai.model.is_none());
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_creates_golish_directory() {
    let temp_dir = TempDir::new().unwrap();
    let golish_dir = temp_dir.path().join(".golish");

    // Verify .golish directory doesn't exist yet
    assert!(!golish_dir.exists());

    // Create manager (doesn't create directory yet)
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;
    assert!(!golish_dir.exists());

    // Save settings - this should create the directory
    manager
        .set_agent_mode("planning".to_string())
        .await
        .unwrap();

    // Verify .golish directory was created
    assert!(golish_dir.exists());
    assert!(golish_dir.is_dir());
}

#[tokio::test]
async fn test_all_providers() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    let providers = vec![
        (AiProvider::VertexAi, "vertex_ai"),
        (AiProvider::Openrouter, "openrouter"),
        (AiProvider::Anthropic, "anthropic"),
        (AiProvider::Openai, "openai"),
        (AiProvider::Ollama, "ollama"),
        (AiProvider::Gemini, "gemini"),
        (AiProvider::Groq, "groq"),
        (AiProvider::Xai, "xai"),
        (AiProvider::ZaiSdk, "zai_sdk"),
        (AiProvider::Nvidia, "nvidia"),
    ];

    for (provider, expected_str) in providers {
        // Set the provider
        manager
            .set_model(provider, "test-model".to_string())
            .await
            .unwrap();

        // Verify it's saved
        let settings = manager.get().await;
        assert_eq!(settings.ai.provider, Some(provider));

        // Verify TOML serialization
        let toml_content = tokio::fs::read_to_string(manager.config_path())
            .await
            .unwrap();
        assert!(toml_content.contains(&format!("provider = \"{}\"", expected_str)));

        // Clean for next iteration
        manager.clear().await.unwrap();
    }
}

#[tokio::test]
async fn test_concurrent_reads() {
    let temp_dir = TempDir::new().unwrap();
    let manager = std::sync::Arc::new(ProjectSettingsManager::new(temp_dir.path()).await);

    // First, set initial settings (creates the directory)
    manager.set_agent_mode("initial".to_string()).await.unwrap();

    // Spawn multiple tasks that read concurrently
    let mut handles = vec![];

    for _ in 0..10 {
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let settings = manager_clone.get().await;
            assert!(settings.ai.agent_mode.is_some());
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_sequential_writes() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Test that multiple sequential writes work correctly
    for i in 0..5 {
        manager.set_agent_mode(format!("mode-{}", i)).await.unwrap();

        // Verify each write
        let settings = manager.get().await;
        assert_eq!(settings.ai.agent_mode, Some(format!("mode-{}", i)));
    }

    // Verify file is still valid TOML
    let toml_content = tokio::fs::read_to_string(manager.config_path())
        .await
        .unwrap();
    let parsed: ProjectSettings = toml::from_str(&toml_content).unwrap();
    assert_eq!(parsed.ai.agent_mode, Some("mode-4".to_string()));
}

#[tokio::test]
async fn test_empty_settings_not_saved() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Verify file doesn't exist initially
    assert!(!manager.config_path().exists());

    // Try to update with all None values
    manager.update_ai_settings(None, None, None).await.unwrap();

    // File should still not exist
    assert!(!manager.config_path().exists());

    // Create settings with values
    manager
        .set_agent_mode("planning".to_string())
        .await
        .unwrap();
    assert!(manager.config_path().exists());

    // Set all values to None
    manager.update_ai_settings(None, None, None).await.unwrap();

    // File should still exist (we don't delete it in update_ai_settings)
    // But the in-memory settings should be all None
    let settings = manager.get().await;
    assert!(settings.ai.provider.is_none());
    assert!(settings.ai.model.is_none());
    assert!(settings.ai.agent_mode.is_none());
}

#[tokio::test]
async fn test_update_method() {
    let temp_dir = TempDir::new().unwrap();
    let manager = ProjectSettingsManager::new(temp_dir.path()).await;

    // Create custom settings
    let new_settings = ProjectSettings {
        ai: ProjectAiSettings {
            provider: Some(AiProvider::Xai),
            model: Some("grok-beta".to_string()),
            agent_mode: Some("auto-approve".to_string()),
        },
    };

    // Update using the update method
    manager.update(new_settings).await.unwrap();

    // Verify settings were updated
    let settings = manager.get().await;
    assert_eq!(settings.ai.provider, Some(AiProvider::Xai));
    assert_eq!(settings.ai.model, Some("grok-beta".to_string()));
    assert_eq!(settings.ai.agent_mode, Some("auto-approve".to_string()));
}
