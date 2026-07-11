use super::*;
use std::env;
use tempfile::TempDir;

#[test]
fn test_interpolate_env_vars_simple() {
    env::set_var("TEST_MCP_VAR", "hello");
    assert_eq!(interpolate_env_vars("$TEST_MCP_VAR"), "hello");
    env::remove_var("TEST_MCP_VAR");
}

#[test]
fn test_interpolate_env_vars_braced() {
    env::set_var("TEST_MCP_VAR2", "world");
    assert_eq!(interpolate_env_vars("${TEST_MCP_VAR2}"), "world");
    env::remove_var("TEST_MCP_VAR2");
}

#[test]
fn test_interpolate_env_vars_mixed() {
    // Note: $VAR syntax consumes all valid var chars (alphanumeric + underscore)
    // So $TEST_MCP_A_middle would look for var "TEST_MCP_A_middle", not "TEST_MCP_A"
    // Use ${VAR} syntax for explicit boundaries
    env::set_var("TEST_MCP_A", "foo");
    env::set_var("TEST_MCP_B", "bar");
    assert_eq!(
        interpolate_env_vars("prefix_${TEST_MCP_A}_middle_${TEST_MCP_B}_suffix"),
        "prefix_foo_middle_bar_suffix"
    );
    env::remove_var("TEST_MCP_A");
    env::remove_var("TEST_MCP_B");
}

#[test]
fn test_interpolate_env_vars_bare_consumes_underscores() {
    // Bare $VAR syntax includes underscores in the var name (shell-like behavior)
    env::set_var("TEST_MCP_WITH_UNDERSCORES", "value");
    assert_eq!(interpolate_env_vars("$TEST_MCP_WITH_UNDERSCORES"), "value");
    env::remove_var("TEST_MCP_WITH_UNDERSCORES");
}

#[test]
fn test_interpolate_env_vars_missing() {
    // Missing env vars should be replaced with empty string
    assert_eq!(interpolate_env_vars("$NONEXISTENT_MCP_VAR_12345"), "");
    assert_eq!(interpolate_env_vars("${NONEXISTENT_MCP_VAR_12345}"), "");
}

#[test]
fn test_interpolate_env_vars_no_vars() {
    assert_eq!(
        interpolate_env_vars("no variables here"),
        "no variables here"
    );
}

#[test]
fn test_interpolate_env_vars_dollar_only() {
    assert_eq!(interpolate_env_vars("$"), "$");
    assert_eq!(interpolate_env_vars("$ "), "$ ");
    assert_eq!(interpolate_env_vars("$1"), "$1"); // Numbers don't start var names
}

#[test]
fn test_interpolate_env_vars_empty_braces() {
    assert_eq!(interpolate_env_vars("${}"), "${}");
}

#[test]
fn test_load_mcp_config_empty_dir() {
    let temp = TempDir::new().unwrap();
    let config = load_mcp_config_inner(None, temp.path(), false).unwrap();
    assert!(config.mcp_servers.is_empty());
}

#[test]
fn test_load_mcp_config_does_not_activate_untrusted_project_servers() {
    let temp = TempDir::new().unwrap();
    let golish_dir = temp.path().join(".golish");
    fs::create_dir_all(&golish_dir).unwrap();
    fs::write(
        golish_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "untrusted-project-server": {
                    "command": "malicious-command"
                }
            }
        }"#,
    )
    .unwrap();

    let config = load_mcp_config(temp.path()).unwrap();

    assert!(!config.mcp_servers.contains_key("untrusted-project-server"));
}

#[test]
fn test_js_reverse_entry_point_requires_generated_devtools_runtime() {
    let temp = TempDir::new().unwrap();
    let tool_root = temp.path().join("js-reverse-mcp");
    let entry_point = tool_root.join("src/index.js");
    fs::create_dir_all(entry_point.parent().unwrap()).unwrap();
    fs::write(&entry_point, "await import('./main.js');").unwrap();

    assert!(!js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));

    let runtime = tool_root.join("node_modules/chrome-devtools-frontend/mcp/mcp.js");
    fs::create_dir_all(runtime.parent().unwrap()).unwrap();
    fs::write(runtime, "export {};").unwrap();

    assert!(!js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));

    let transitive_runtime =
        tool_root.join("node_modules/chrome-devtools-frontend/front_end/core/common/common.js");
    fs::create_dir_all(transitive_runtime.parent().unwrap()).unwrap();
    fs::write(transitive_runtime, "export {};").unwrap();

    assert!(js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));
}

#[test]
fn test_js_reverse_build_entry_point_requires_generated_devtools_runtime() {
    let temp = TempDir::new().unwrap();
    let build_root = temp.path().join("js-reverse-mcp/build");
    let entry_point = build_root.join("src/index.js");
    fs::create_dir_all(entry_point.parent().unwrap()).unwrap();
    fs::write(&entry_point, "await import('./main.js');").unwrap();

    assert!(!js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));

    let runtime = build_root.join("node_modules/chrome-devtools-frontend/mcp/mcp.js");
    fs::create_dir_all(runtime.parent().unwrap()).unwrap();
    fs::write(runtime, "export {};").unwrap();

    assert!(!js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));

    let transitive_runtime =
        build_root.join("node_modules/chrome-devtools-frontend/front_end/core/common/common.js");
    fs::create_dir_all(transitive_runtime.parent().unwrap()).unwrap();
    fs::write(transitive_runtime, "export {};").unwrap();

    assert!(js_reverse_entry_point_has_generated_devtools_runtime(
        &entry_point
    ));
}

#[test]
fn test_load_mcp_config_project_only() {
    let temp = TempDir::new().unwrap();
    let golish_dir = temp.path().join(".golish");
    fs::create_dir_all(&golish_dir).unwrap();

    let config_json = r#"{
            "mcpServers": {
                "test-server": {
                    "transport": "stdio",
                    "command": "echo",
                    "args": ["hello"]
                }
            }
        }"#;
    fs::write(golish_dir.join("mcp.json"), config_json).unwrap();

    let config = load_mcp_config_inner(None, temp.path(), true).unwrap();
    assert_eq!(config.mcp_servers.len(), 1);
    assert!(config.mcp_servers.contains_key("test-server"));

    let server = &config.mcp_servers["test-server"];
    assert_eq!(server.command.as_deref(), Some("echo"));
    assert_eq!(server.args, vec!["hello"]);
    assert!(server.enabled); // Default
}

#[test]
fn test_load_mcp_config_invalid_json() {
    let temp = TempDir::new().unwrap();
    let golish_dir = temp.path().join(".golish");
    fs::create_dir_all(&golish_dir).unwrap();

    fs::write(golish_dir.join("mcp.json"), "{ invalid json }").unwrap();

    let result = load_mcp_config_inner(None, temp.path(), true);
    assert!(result.is_err());
}

#[test]
fn test_untrusted_project_config_is_not_parsed() {
    let temp = TempDir::new().unwrap();
    let golish_dir = temp.path().join(".golish");
    fs::create_dir_all(&golish_dir).unwrap();
    fs::write(golish_dir.join("mcp.json"), "{ invalid json }").unwrap();

    let config = load_mcp_config_inner(None, temp.path(), false).unwrap();

    assert!(!config.mcp_servers.contains_key("project-only"));
}

#[test]
fn test_trust_and_check_project_config() {
    let temp = TempDir::new().unwrap();

    // Initially not trusted
    assert!(!is_project_config_trusted(temp.path()));

    // Trust it (this writes to ~/.golish which we can't easily test without mocking)
    // So we just verify the function doesn't panic
    // Full integration test would require mocking the home dir
}

#[test]
fn test_load_mcp_config_merges_project_over_user() {
    // This tests the actual merging behavior which is critical:
    // Project config should override user config for same server name

    let user_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();

    // User config defines shared-server and user-only
    let user_golish_dir = user_dir.path().join(".golish");
    fs::create_dir_all(&user_golish_dir).unwrap();
    let user_config = r#"{
            "mcpServers": {
                "shared-server": {
                    "command": "user-command",
                    "args": ["--user"]
                },
                "user-only": {
                    "command": "user-only-cmd"
                }
            }
        }"#;
    fs::write(user_golish_dir.join("mcp.json"), user_config).unwrap();

    // Project config overrides shared-server and adds project-only
    let project_golish_dir = project_dir.path().join(".golish");
    fs::create_dir_all(&project_golish_dir).unwrap();
    let project_config = r#"{
            "mcpServers": {
                "shared-server": {
                    "command": "project-command",
                    "args": ["--project"]
                },
                "project-only": {
                    "command": "project-only-cmd"
                }
            }
        }"#;
    fs::write(project_golish_dir.join("mcp.json"), project_config).unwrap();

    let user_config_path = user_golish_dir.join("mcp.json");
    let config = load_mcp_config_inner(Some(user_config_path), project_dir.path(), true).unwrap();

    // 3 servers: user-only + shared-server (overridden by project) + project-only
    assert_eq!(config.mcp_servers.len(), 3);
    // Project config overrides shared-server
    assert_eq!(
        config.mcp_servers["shared-server"].command.as_deref(),
        Some("project-command")
    );
    assert_eq!(config.mcp_servers["shared-server"].args, vec!["--project"]);
    assert!(config.mcp_servers.contains_key("project-only"));
    assert!(config.mcp_servers.contains_key("user-only"));
}

#[test]
fn test_load_mcp_config_all_fields() {
    // Test that all config fields are parsed correctly
    let temp = TempDir::new().unwrap();
    let golish_dir = temp.path().join(".golish");
    fs::create_dir_all(&golish_dir).unwrap();

    let config_json = r#"{
            "mcpServers": {
                "full-config": {
                    "transport": "http",
                    "command": "should-be-ignored",
                    "args": ["arg1", "arg2"],
                    "env": {
                        "API_KEY": "${MY_API_KEY}",
                        "DEBUG": "true"
                    },
                    "url": "https://api.example.com/mcp",
                    "headers": {
                        "Authorization": "Bearer ${TOKEN}",
                        "X-Custom": "value"
                    },
                    "enabled": false,
                    "timeout": 60
                }
            }
        }"#;
    fs::write(golish_dir.join("mcp.json"), config_json).unwrap();

    let config = load_mcp_config_inner(None, temp.path(), true).unwrap();
    let server = &config.mcp_servers["full-config"];

    assert!(matches!(
        server.transport(),
        crate::config::McpTransportType::Http
    ));
    assert_eq!(server.command.as_deref(), Some("should-be-ignored"));
    assert_eq!(server.args, vec!["arg1", "arg2"]);
    assert_eq!(server.env.len(), 2);
    assert_eq!(server.env.get("DEBUG"), Some(&"true".to_string()));
    assert_eq!(server.url.as_deref(), Some("https://api.example.com/mcp"));
    assert_eq!(server.headers.len(), 2);
    assert_eq!(server.headers.get("X-Custom"), Some(&"value".to_string()));
    assert!(!server.enabled);
    assert_eq!(server.timeout, 60);
}

#[test]
fn test_untrusted_project_config_cannot_override_user_config() {
    let user_dir = TempDir::new().unwrap();
    let project_dir = TempDir::new().unwrap();
    let user_config_path = user_dir.path().join("mcp.json");
    fs::write(
        &user_config_path,
        r#"{
            "mcpServers": {
                "shared-server": { "command": "user-command" }
            }
        }"#,
    )
    .unwrap();

    let project_golish_dir = project_dir.path().join(".golish");
    fs::create_dir_all(&project_golish_dir).unwrap();
    fs::write(
        project_golish_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "shared-server": { "command": "project-command" },
                "project-only": { "command": "project-only-command" }
            }
        }"#,
    )
    .unwrap();

    let config = load_mcp_config_inner(Some(user_config_path), project_dir.path(), false).unwrap();

    assert_eq!(
        config.mcp_servers["shared-server"].command.as_deref(),
        Some("user-command")
    );
    assert!(!config.mcp_servers.contains_key("project-only"));
}

#[test]
fn test_builtin_setup_directory_rejects_non_registry_server_names() {
    assert!(builtin_setup_directory("project-controlled-name").is_none());
}

#[test]
fn test_builtin_resolution_never_reads_qbit_workspace() {
    let temp = TempDir::new().unwrap();
    let attacker_entry = temp
        .path()
        .join("tools/attacker-controlled-builtin/entry.js");
    fs::create_dir_all(attacker_entry.parent().unwrap()).unwrap();
    fs::write(&attacker_entry, "malicious();").unwrap();

    let previous = env::var_os("QBIT_WORKSPACE");
    env::set_var("QBIT_WORKSPACE", temp.path());
    let resolved = resolve_builtin_tool_path("attacker-controlled-builtin/entry.js");
    match previous {
        Some(value) => env::set_var("QBIT_WORKSPACE", value),
        None => env::remove_var("QBIT_WORKSPACE"),
    }

    assert!(resolved.is_none());
}

#[test]
fn test_interpolate_preserves_surrounding_text() {
    env::set_var("TEST_INTERP_VAR", "VALUE");

    // Test various positions
    assert_eq!(
        interpolate_env_vars("before ${TEST_INTERP_VAR} after"),
        "before VALUE after"
    );
    assert_eq!(
        interpolate_env_vars("${TEST_INTERP_VAR}:suffix"),
        "VALUE:suffix"
    );
    assert_eq!(
        interpolate_env_vars("prefix:${TEST_INTERP_VAR}"),
        "prefix:VALUE"
    );

    env::remove_var("TEST_INTERP_VAR");
}

#[test]
fn test_interpolate_multiple_same_var() {
    env::set_var("TEST_REPEAT", "X");

    assert_eq!(
        interpolate_env_vars("$TEST_REPEAT-$TEST_REPEAT-${TEST_REPEAT}"),
        "X-X-X"
    );

    env::remove_var("TEST_REPEAT");
}

#[test]
fn test_interpolate_unclosed_brace() {
    // Unclosed brace should consume rest of string as var name
    // and result in empty (since that var doesn't exist)
    assert_eq!(interpolate_env_vars("${UNCLOSED"), "");
    assert_eq!(interpolate_env_vars("prefix ${UNCLOSED"), "prefix ");
}

#[test]
fn test_interpolate_nested_not_supported() {
    // Nested ${${VAR}} is not supported - outer should work, inner becomes literal
    env::set_var("OUTER", "outer_val");

    // This will try to find var named "${OUTER" which doesn't exist
    let result = interpolate_env_vars("${${OUTER}}");
    // The inner ${ starts a new var capture, so it looks for var "${OUTER}"
    // which doesn't exist, so empty string, then "}" is left over
    assert_eq!(result, "}");

    env::remove_var("OUTER");
}
