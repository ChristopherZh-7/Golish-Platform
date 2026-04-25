use std::collections::HashMap;

use golish_core::ToolName;

use super::defaults::{
    get_known_tool, get_typed_allow_tools, get_typed_deny_tools, get_typed_prompt_tools,
    is_known_tool, ToolPolicyConfig,
};
use super::manager::ToolPolicyManager;
use super::types::{PolicyConstraintResult, ToolConstraints, ToolPolicy};

#[test]
fn test_tool_policy_default() {
    assert_eq!(ToolPolicy::default(), ToolPolicy::Prompt);
}

#[test]
fn test_is_known_tool() {
    assert!(is_known_tool("read_file"));
    assert!(is_known_tool("write_file"));
    assert!(is_known_tool("web_fetch"));
    assert!(!is_known_tool("unknown_tool"));
    assert!(!is_known_tool("debug_agent")); // Dynamic tool
}

#[test]
fn test_get_known_tool() {
    assert_eq!(get_known_tool("read_file"), Some(ToolName::ReadFile));
    assert_eq!(get_known_tool("web_fetch"), Some(ToolName::WebFetch));
    assert_eq!(get_known_tool("unknown"), None);
}

#[test]
fn test_typed_tool_lists() {
    let allow_tools = get_typed_allow_tools();
    assert!(allow_tools.contains(&ToolName::ReadFile));
    assert!(allow_tools.contains(&ToolName::GrepFile));
    assert!(!allow_tools.contains(&ToolName::WriteFile));

    let prompt_tools = get_typed_prompt_tools();
    assert!(prompt_tools.contains(&ToolName::WriteFile));
    assert!(prompt_tools.contains(&ToolName::WebFetch));
    assert!(!prompt_tools.contains(&ToolName::ReadFile));

    let deny_tools = get_typed_deny_tools();
    assert!(deny_tools.contains(&ToolName::DeleteFile));
    assert!(!deny_tools.contains(&ToolName::ReadFile));
}

#[test]
fn test_tool_policy_display() {
    assert_eq!(format!("{}", ToolPolicy::Allow), "allow");
    assert_eq!(format!("{}", ToolPolicy::Prompt), "prompt");
    assert_eq!(format!("{}", ToolPolicy::Deny), "deny");
}

#[test]
fn test_constraints_url_blocked() {
    let constraints = ToolConstraints {
        blocked_hosts: Some(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
            ".internal".to_string(),
        ]),
        blocked_schemes: Some(vec!["file://".to_string()]),
        ..Default::default()
    };

    // Blocked hosts
    assert!(constraints.is_url_blocked("http://localhost/api").is_some());
    assert!(constraints
        .is_url_blocked("http://127.0.0.1:8080/")
        .is_some());
    assert!(constraints
        .is_url_blocked("https://app.internal/")
        .is_some());

    // Blocked schemes
    assert!(constraints.is_url_blocked("file:///etc/passwd").is_some());

    // Allowed
    assert!(constraints
        .is_url_blocked("https://api.example.com/")
        .is_none());
}

#[test]
fn test_constraints_path_blocked() {
    let constraints = ToolConstraints {
        blocked_patterns: Some(vec!["*.env".to_string(), "**/secrets/*".to_string()]),
        allowed_extensions: Some(vec![".rs".to_string(), ".ts".to_string()]),
        ..Default::default()
    };

    // Blocked patterns
    assert!(constraints.is_path_blocked(".env").is_some());
    assert!(constraints.is_path_blocked("config/.env").is_some());
    assert!(constraints
        .is_path_blocked("config/secrets/key.txt")
        .is_some());

    // Allowed extensions (only .rs and .ts allowed)
    assert!(constraints.is_path_blocked("main.py").is_some()); // .py not allowed
    assert!(constraints.is_path_blocked("main.rs").is_none()); // .rs allowed
    assert!(constraints.is_path_blocked("app.ts").is_none()); // .ts allowed
}

#[test]
fn test_constraints_mode_allowed() {
    let constraints = ToolConstraints {
        allowed_modes: Some(vec!["read".to_string(), "list".to_string()]),
        ..Default::default()
    };

    assert!(constraints.is_mode_allowed("read"));
    assert!(constraints.is_mode_allowed("list"));
    assert!(!constraints.is_mode_allowed("write"));
    assert!(!constraints.is_mode_allowed("delete"));

    // No mode restriction
    let empty_constraints = ToolConstraints::default();
    assert!(empty_constraints.is_mode_allowed("anything"));
}

#[test]
fn test_constraints_limits() {
    let constraints = ToolConstraints {
        max_items: Some(100),
        max_bytes: Some(65536),
        ..Default::default()
    };

    assert!(!constraints.exceeds_max_items(50));
    assert!(!constraints.exceeds_max_items(100));
    assert!(constraints.exceeds_max_items(101));

    assert!(!constraints.exceeds_max_bytes(32768));
    assert!(!constraints.exceeds_max_bytes(65536));
    assert!(constraints.exceeds_max_bytes(65537));
}

#[test]
fn test_default_config() {
    let config = ToolPolicyConfig::default();

    // Check default policies
    assert_eq!(config.policies.get("read_file"), Some(&ToolPolicy::Allow));
    assert_eq!(config.policies.get("write_file"), Some(&ToolPolicy::Prompt));
    assert_eq!(config.policies.get("delete_file"), Some(&ToolPolicy::Deny));

    // Check default policy for unknown tools
    assert_eq!(config.default_policy, ToolPolicy::Prompt);

    // Check constraints exist for web_fetch
    assert!(config.constraints.contains_key("web_fetch"));
}

#[tokio::test]
async fn test_policy_manager_get_set() {
    let temp_dir = tempfile::tempdir().unwrap();
    let manager = ToolPolicyManager::new(temp_dir.path()).await;

    // Default policy for read_file should be Allow
    assert_eq!(manager.get_policy("read_file").await, ToolPolicy::Allow);

    // Default policy for unknown tool should be Prompt
    assert_eq!(manager.get_policy("unknown_tool").await, ToolPolicy::Prompt);

    // Set a policy
    manager
        .set_policy("custom_tool", ToolPolicy::Deny)
        .await
        .unwrap();
    assert_eq!(manager.get_policy("custom_tool").await, ToolPolicy::Deny);
}

#[tokio::test]
async fn test_policy_manager_preapproval() {
    let temp_dir = tempfile::tempdir().unwrap();
    let manager = ToolPolicyManager::new(temp_dir.path()).await;

    // Initially, write_file requires prompt
    assert!(manager.requires_prompt("write_file").await);
    assert!(!manager.should_execute("write_file").await);

    // Pre-approve
    manager.preapprove("write_file").await;

    // Now it should execute without prompt
    assert!(!manager.requires_prompt("write_file").await);
    assert!(manager.should_execute("write_file").await);

    // Take pre-approval (one-time use)
    assert!(manager.take_preapproved("write_file").await);
    assert!(!manager.take_preapproved("write_file").await);

    // Back to requiring prompt
    assert!(manager.requires_prompt("write_file").await);
}

#[tokio::test]
async fn test_policy_manager_full_auto() {
    let temp_dir = tempfile::tempdir().unwrap();
    let manager = ToolPolicyManager::new(temp_dir.path()).await;

    // Initially not in full-auto mode
    assert!(!manager.is_full_auto_enabled().await);
    assert!(!manager.is_allowed_in_full_auto("write_file").await);

    // Enable full-auto with specific tools
    manager
        .enable_full_auto(vec!["read_file".to_string(), "write_file".to_string()])
        .await;

    assert!(manager.is_full_auto_enabled().await);
    assert!(manager.is_allowed_in_full_auto("read_file").await);
    assert!(manager.is_allowed_in_full_auto("write_file").await);
    assert!(!manager.is_allowed_in_full_auto("delete_file").await);

    // Policy should return Allow for full-auto tools
    assert_eq!(manager.get_policy("write_file").await, ToolPolicy::Allow);

    // Disable full-auto
    manager.disable_full_auto().await;
    assert!(!manager.is_full_auto_enabled().await);

    // Back to normal policy
    assert_eq!(manager.get_policy("write_file").await, ToolPolicy::Prompt);
}

#[tokio::test]
async fn test_apply_constraints() {
    let temp_dir = tempfile::tempdir().unwrap();
    let manager = ToolPolicyManager::new(temp_dir.path()).await;

    // Test URL constraint violation
    let args = serde_json::json!({
        "url": "http://localhost:8080/api"
    });
    let result = manager.apply_constraints("web_fetch", &args).await;
    assert!(matches!(result, PolicyConstraintResult::Violated(_)));

    // Test allowed URL
    let args = serde_json::json!({
        "url": "https://api.example.com/"
    });
    let result = manager.apply_constraints("web_fetch", &args).await;
    assert!(matches!(result, PolicyConstraintResult::Allowed));
}

#[test]
fn test_merge_configs() {
    // Global: read_file=Allow (from default), custom_tool=Deny, default=Deny
    let mut global = ToolPolicyConfig::default();
    global
        .policies
        .insert("custom_tool".to_string(), ToolPolicy::Deny);
    global
        .policies
        .insert("global_only_tool".to_string(), ToolPolicy::Allow);
    global.default_policy = ToolPolicy::Deny;

    // Project: custom_tool=Allow (overrides global), project_tool=Prompt, default=Prompt
    let mut project = ToolPolicyConfig {
        version: 1,
        available_tools: vec![],
        policies: HashMap::new(),
        constraints: HashMap::new(),
        default_policy: ToolPolicy::Prompt,
    };
    project
        .policies
        .insert("custom_tool".to_string(), ToolPolicy::Allow);
    project
        .policies
        .insert("project_tool".to_string(), ToolPolicy::Prompt);

    let merged = ToolPolicyManager::merge_configs(&Some(global), &Some(project));

    assert_eq!(merged.policies.get("custom_tool"), Some(&ToolPolicy::Allow));
    assert_eq!(
        merged.policies.get("global_only_tool"),
        Some(&ToolPolicy::Allow)
    );
    assert_eq!(
        merged.policies.get("project_tool"),
        Some(&ToolPolicy::Prompt)
    );
    assert_eq!(merged.default_policy, ToolPolicy::Prompt);
}

#[test]
fn test_merge_configs_global_only() {
    let mut global = ToolPolicyConfig::default();
    global
        .policies
        .insert("my_tool".to_string(), ToolPolicy::Deny);
    global.default_policy = ToolPolicy::Allow;

    let merged = ToolPolicyManager::merge_configs(&Some(global), &None);

    assert_eq!(merged.policies.get("my_tool"), Some(&ToolPolicy::Deny));
    assert_eq!(merged.default_policy, ToolPolicy::Allow);
}

#[test]
fn test_merge_configs_project_only() {
    let mut project = ToolPolicyConfig::default();
    project
        .policies
        .insert("my_tool".to_string(), ToolPolicy::Allow);
    project.default_policy = ToolPolicy::Deny;

    let merged = ToolPolicyManager::merge_configs(&None, &Some(project));

    assert_eq!(merged.policies.get("my_tool"), Some(&ToolPolicy::Allow));
    assert_eq!(merged.default_policy, ToolPolicy::Deny);
}

#[test]
fn test_merge_configs_neither() {
    let merged = ToolPolicyManager::merge_configs(&None, &None);

    // Should use defaults
    assert_eq!(merged.policies.get("read_file"), Some(&ToolPolicy::Allow));
    assert_eq!(merged.default_policy, ToolPolicy::Prompt);
}
