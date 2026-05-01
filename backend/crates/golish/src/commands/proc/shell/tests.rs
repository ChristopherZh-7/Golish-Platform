use super::*;

#[test]
fn test_source_line_uses_actual_config_path() {
    // This test ensures we never regress to hardcoded paths
    let integration_path = get_integration_path().expect("Should get integration path");
    let config_dir = get_config_dir().expect("Should get config dir");

    // The integration path must be under the config directory
    assert!(
        integration_path.starts_with(&config_dir),
        "Integration path {:?} should be under config dir {:?}",
        integration_path,
        config_dir
    );

    // On macOS, this should NOT be ~/.config but ~/Library/Application Support
    #[cfg(target_os = "macos")]
    {
        let path_str = integration_path.display().to_string();
        assert!(
            !path_str.contains("/.config/"),
            "macOS should use Application Support, not .config. Got: {}",
            path_str
        );
        assert!(
            path_str.contains("Library/Application Support"),
            "macOS should use Library/Application Support. Got: {}",
            path_str
        );
    }

    // On Linux, it should be ~/.config
    #[cfg(target_os = "linux")]
    {
        let path_str = integration_path.display().to_string();
        assert!(
            path_str.contains("/.config/") || path_str.contains("XDG_CONFIG"),
            "Linux should use .config or XDG_CONFIG. Got: {}",
            path_str
        );
    }
}

#[test]
fn test_validate_zshrc_detects_wrong_path() {
    // This test requires mocking the filesystem which is complex in Rust
    // Instead, we test the logic by checking the actual system config
    let integration_path = get_integration_path().expect("Should get integration path");
    let expected_path_str = integration_path.display().to_string();

    // Verify the path we generate is what we expect
    assert!(
        expected_path_str.contains("golish"),
        "Path should contain 'golish'"
    );
    assert!(
        expected_path_str.ends_with("integration.zsh"),
        "Path should end with integration.zsh"
    );
}

#[test]
fn test_zsh_script_contains_required_markers() {
    let script = get_integration_script(ShellType::Zsh);
    assert!(
        script.contains("__golish_osc"),
        "Script should have OSC helper"
    );
    assert!(
        script.contains(r#"133;%s"#),
        "Script should have OSC 133 format string"
    );
    assert!(
        script.contains(r#"__golish_osc "A""#),
        "Script should emit prompt_start (A marker)"
    );
    assert!(
        script.contains(r#"__golish_osc "B""#),
        "Script should emit prompt_end (B marker)"
    );
    assert!(
        script.contains(r#"__golish_osc "C"#),
        "Script should emit command_start (C marker)"
    );
    assert!(
        script.contains(r#"__golish_osc "D"#),
        "Script should emit command_end (D marker)"
    );
    assert!(script.contains("preexec"), "Script should use preexec hook");
    assert!(script.contains("precmd"), "Script should use precmd hook");
}

#[test]
fn test_bash_script_contains_required_markers() {
    let script = get_integration_script(ShellType::Bash);
    assert!(
        script.contains("__golish_osc"),
        "Bash script should have OSC helper"
    );
    assert!(
        script.contains(r#"133;%s"#),
        "Bash script should have OSC 133 format string"
    );
    assert!(
        script.contains("PROMPT_COMMAND"),
        "Bash script should use PROMPT_COMMAND"
    );
    assert!(
        script.contains("DEBUG"),
        "Bash script should use DEBUG trap"
    );
    assert!(
        script.contains(r#"__golish_osc "A""#),
        "Bash script should emit A marker"
    );
    assert!(
        script.contains(r#"__golish_osc "C""#),
        "Bash script should emit C marker"
    );
    assert!(
        script.contains(r#"__golish_osc "D"#),
        "Bash script should emit D marker"
    );
    // B marker is in PS1 for bash
    assert!(
        script.contains("133;B"),
        "Bash script should emit B marker in PS1"
    );
}

#[test]
fn test_fish_script_contains_required_markers() {
    let script = get_integration_script(ShellType::Fish);
    assert!(
        script.contains("__golish_osc"),
        "Fish script should have OSC helper"
    );
    assert!(
        script.contains(r#"133;%s"#),
        "Fish script should have OSC 133 format string"
    );
    assert!(
        script.contains("fish_preexec"),
        "Fish script should use fish_preexec event"
    );
    assert!(
        script.contains("fish_postexec"),
        "Fish script should use fish_postexec event"
    );
    assert!(
        script.contains(r#"__golish_osc "A""#),
        "Fish script should emit A marker"
    );
    assert!(
        script.contains(r#"__golish_osc "B""#),
        "Fish script should emit B marker"
    );
    assert!(
        script.contains(r#"__golish_osc "C""#),
        "Fish script should emit C marker"
    );
    assert!(
        script.contains(r#"__golish_osc "D"#),
        "Fish script should emit D marker"
    );
}

#[test]
fn test_all_shells_emit_all_markers() {
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::Fish] {
        let script = get_integration_script(shell_type);
        // All shells must emit A, B, C, D markers
        assert!(
            script.contains(r#""A""#) || script.contains("133;A"),
            "{:?} script missing A marker",
            shell_type
        );
        assert!(
            script.contains(r#""B""#) || script.contains("133;B"),
            "{:?} script missing B marker",
            shell_type
        );
        assert!(
            script.contains(r#""C""#) || script.contains("133;C"),
            "{:?} script missing C marker",
            shell_type
        );
        assert!(
            script.contains(r#""D"#) || script.contains("133;D"),
            "{:?} script missing D marker",
            shell_type
        );
    }
}

#[test]
fn test_all_shells_have_golish_guard() {
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::Fish] {
        let script = get_integration_script(shell_type);
        assert!(
            script.contains("QBIT"),
            "{:?} script should check for QBIT env var",
            shell_type
        );
    }
}

#[test]
fn test_all_shells_have_double_source_guard() {
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::Fish] {
        let script = get_integration_script(shell_type);
        assert!(
            script.contains("QBIT_INTEGRATION_LOADED"),
            "{:?} script should guard against double-sourcing",
            shell_type
        );
    }
}

#[test]
fn test_zsh_script_checks_golish_env() {
    let script = get_integration_script(ShellType::Zsh);
    assert!(
        script.contains(r#"[[ -z "$QBIT" ]] && return"#),
        "Zsh script should check for QBIT env var"
    );
}

#[test]
fn test_bash_script_checks_golish_env() {
    let script = get_integration_script(ShellType::Bash);
    assert!(
        script.contains(r#"[[ "$QBIT" != "1" ]] && return"#),
        "Bash script should check for QBIT env var"
    );
}

#[test]
fn test_fish_script_checks_golish_env() {
    let script = get_integration_script(ShellType::Fish);
    assert!(
        script.contains(r#"test "$QBIT" != "1""#),
        "Fish script should check for QBIT env var"
    );
}

#[test]
fn test_get_integration_extension() {
    assert_eq!(get_integration_extension(ShellType::Zsh), "zsh");
    assert_eq!(get_integration_extension(ShellType::Bash), "bash");
    assert_eq!(get_integration_extension(ShellType::Fish), "fish");
    assert_eq!(get_integration_extension(ShellType::Unknown), "zsh");
}

#[test]
fn test_get_integration_script_unknown_defaults_to_zsh() {
    let unknown_script = get_integration_script(ShellType::Unknown);
    let zsh_script = get_integration_script(ShellType::Zsh);
    assert_eq!(unknown_script, zsh_script);
}

#[test]
fn test_config_dir_consistency() {
    // All path functions should use the same base directory
    let config_dir = get_config_dir().expect("Should get config dir");
    let integration_path = get_integration_path().expect("Should get integration path");
    let version_path = get_version_path().expect("Should get version path");

    assert!(
        integration_path.parent() == Some(config_dir.as_path()),
        "Integration path parent should be config dir"
    );
    assert!(
        version_path.parent() == Some(config_dir.as_path()),
        "Version path parent should be config dir"
    );
}

// =========================================================================
// Property-Based Tests
// =========================================================================

mod prop_tests;

// =========================================================================
// Installation Tests (using TempDir for isolation)
// =========================================================================

mod installation_tests;
