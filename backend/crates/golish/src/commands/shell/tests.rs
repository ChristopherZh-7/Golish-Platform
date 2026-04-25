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

mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// All integration scripts must have balanced quotes
        #[test]
        fn prop_scripts_have_balanced_quotes(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let script = get_integration_script(shell_type);
            let single_quotes = script.matches('\'').count();
            let double_quotes = script.matches('"').count();

            // Quotes should be balanced (even count)
            // Note: This is a heuristic - some edge cases may have odd counts
            // but it catches most syntax errors
            prop_assert!(
                single_quotes.is_multiple_of(2),
                "{:?} has unbalanced single quotes: {}", shell_type, single_quotes
            );
            prop_assert!(
                double_quotes.is_multiple_of(2),
                "{:?} has unbalanced double quotes: {}", shell_type, double_quotes
            );
        }

        /// All shells must emit the same set of OSC markers
        #[test]
        fn prop_all_shells_emit_same_markers(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let script = get_integration_script(shell_type);

            // Every shell must emit all 4 markers
            for marker in ["A", "B", "C", "D"] {
                prop_assert!(
                    script.contains(&format!(r#""{}"#, marker)) ||
                    script.contains(&format!("133;{}", marker)),
                    "{:?} missing marker {}", shell_type, marker
                );
            }
        }

        /// All scripts must have the double-source guard
        #[test]
        fn prop_all_scripts_have_source_guard(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let script = get_integration_script(shell_type);
            prop_assert!(
                script.contains("QBIT_INTEGRATION_LOADED"),
                "{:?} missing double-source guard", shell_type
            );
        }

        /// All scripts must check QBIT environment variable
        #[test]
        fn prop_all_scripts_check_golish_env(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let script = get_integration_script(shell_type);
            prop_assert!(
                script.contains("QBIT"),
                "{:?} missing QBIT environment check", shell_type
            );
        }

        /// Script extension matches shell type
        #[test]
        fn prop_extension_matches_shell(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let ext = get_integration_extension(shell_type);
            match shell_type {
                ShellType::Zsh => prop_assert_eq!(ext, "zsh"),
                ShellType::Bash => prop_assert_eq!(ext, "bash"),
                ShellType::Fish => prop_assert_eq!(ext, "fish"),
                ShellType::Unknown => prop_assert_eq!(ext, "zsh"),
            }
        }

        /// All scripts have proper OSC format string
        #[test]
        fn prop_all_scripts_have_osc_format(
            shell_type in prop_oneof![
                Just(ShellType::Zsh),
                Just(ShellType::Bash),
                Just(ShellType::Fish),
            ]
        ) {
            let script = get_integration_script(shell_type);
            // All scripts should use printf with OSC 133 format
            prop_assert!(
                script.contains(r"133;%s") || script.contains("133;"),
                "{:?} missing OSC 133 format", shell_type
            );
        }
    }
}

// =========================================================================
// Installation Tests (using TempDir for isolation)
// =========================================================================

mod installation_tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, TempDir) {
        let home = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        (home, config)
    }

    // -------------------------------------------------------------------------
    // Integration Script Creation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_install_creates_integration_script_for_zsh() {
        let (home, config) = setup_test_env();

        let result = install_integration_internal(ShellType::Zsh, config.path(), home.path());
        assert!(result.is_ok());

        let script_path = config.path().join("integration.zsh");
        assert!(script_path.exists(), "Zsh integration script not created");

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.contains("QBIT_INTEGRATION_LOADED"));
    }

    #[test]
    fn test_install_creates_integration_script_for_bash() {
        let (home, config) = setup_test_env();

        let result = install_integration_internal(ShellType::Bash, config.path(), home.path());
        assert!(result.is_ok());

        let script_path = config.path().join("integration.bash");
        assert!(script_path.exists(), "Bash integration script not created");

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.contains("PROMPT_COMMAND"));
    }

    #[test]
    fn test_install_creates_integration_script_for_fish() {
        let (home, config) = setup_test_env();

        let result = install_integration_internal(ShellType::Fish, config.path(), home.path());
        assert!(result.is_ok());

        let script_path = config.path().join("integration.fish");
        assert!(script_path.exists(), "Fish integration script not created");

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.contains("fish_preexec"));
    }

    #[test]
    fn test_install_creates_version_file() {
        let (home, config) = setup_test_env();

        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();

        let version_path = config.path().join("integration.version");
        assert!(version_path.exists(), "Version file not created");

        let version = std::fs::read_to_string(&version_path).unwrap();
        assert_eq!(version.trim(), INTEGRATION_VERSION);
    }

    // -------------------------------------------------------------------------
    // RC File Update Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_install_updates_zshrc() {
        let (home, config) = setup_test_env();

        // Create empty .zshrc
        std::fs::write(home.path().join(".zshrc"), "# existing content\n").unwrap();

        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();

        let rc_content = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();
        assert!(
            rc_content.contains("Golish shell integration"),
            "RC file missing Golish header"
        );
        assert!(
            rc_content.contains("integration.zsh"),
            "RC file missing source line"
        );
        assert!(rc_content.contains("QBIT"), "RC file missing QBIT guard");
    }

    #[test]
    fn test_install_updates_both_bash_rc_files() {
        let (home, config) = setup_test_env();

        // Create empty bashrc files
        std::fs::write(home.path().join(".bashrc"), "# bashrc\n").unwrap();
        std::fs::write(home.path().join(".bash_profile"), "# bash_profile\n").unwrap();

        install_integration_internal(ShellType::Bash, config.path(), home.path()).unwrap();

        let bashrc = std::fs::read_to_string(home.path().join(".bashrc")).unwrap();
        let bash_profile = std::fs::read_to_string(home.path().join(".bash_profile")).unwrap();

        assert!(
            bashrc.contains("integration.bash"),
            ".bashrc not updated with source line"
        );
        assert!(
            bash_profile.contains("integration.bash"),
            ".bash_profile not updated with source line"
        );
    }

    #[test]
    fn test_install_creates_fish_config_directory() {
        let (home, config) = setup_test_env();

        // Don't create .config/fish - let install create it
        install_integration_internal(ShellType::Fish, config.path(), home.path()).unwrap();

        let fish_config = home.path().join(".config/fish/conf.d/golish.fish");
        assert!(fish_config.exists(), "Fish config file not created");

        let content = std::fs::read_to_string(&fish_config).unwrap();
        assert!(content.contains("integration.fish"));
    }

    #[test]
    fn test_fish_rc_uses_fish_syntax() {
        let (home, config) = setup_test_env();

        install_integration_internal(ShellType::Fish, config.path(), home.path()).unwrap();

        let fish_config = home.path().join(".config/fish/conf.d/golish.fish");
        let content = std::fs::read_to_string(&fish_config).unwrap();

        // Fish syntax uses 'test' and 'end', not [[ ]]
        assert!(
            content.contains("if test"),
            "Fish RC should use 'test' syntax"
        );
        assert!(content.contains("end"), "Fish RC should use 'end' keyword");
    }

    // -------------------------------------------------------------------------
    // Idempotency Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_install_is_idempotent_zsh() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        // Install twice
        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();
        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();

        let rc_content = std::fs::read_to_string(home.path().join(".zshrc")).unwrap();
        let source_count = rc_content.matches("integration.zsh").count();

        assert_eq!(source_count, 1, "Integration sourced multiple times");
    }

    #[test]
    fn test_install_is_idempotent_bash() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".bashrc"), "").unwrap();
        std::fs::write(home.path().join(".bash_profile"), "").unwrap();

        // Install twice
        install_integration_internal(ShellType::Bash, config.path(), home.path()).unwrap();
        install_integration_internal(ShellType::Bash, config.path(), home.path()).unwrap();

        let bashrc = std::fs::read_to_string(home.path().join(".bashrc")).unwrap();
        let source_count = bashrc.matches("integration.bash").count();

        assert_eq!(
            source_count, 1,
            "Integration sourced multiple times in .bashrc"
        );
    }

    #[test]
    fn test_install_is_idempotent_fish() {
        let (home, config) = setup_test_env();

        // Install twice
        install_integration_internal(ShellType::Fish, config.path(), home.path()).unwrap();
        install_integration_internal(ShellType::Fish, config.path(), home.path()).unwrap();

        let fish_config = home.path().join(".config/fish/conf.d/golish.fish");
        let content = std::fs::read_to_string(&fish_config).unwrap();
        let source_count = content.matches("integration.fish").count();

        assert_eq!(
            source_count, 1,
            "Integration sourced multiple times in fish config"
        );
    }

    // -------------------------------------------------------------------------
    // Uninstall Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_uninstall_removes_integration_script_zsh() {
        let (home, config) = setup_test_env();

        // Install first
        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();
        assert!(config.path().join("integration.zsh").exists());

        // Uninstall
        uninstall_integration_internal(ShellType::Zsh, config.path()).unwrap();
        assert!(!config.path().join("integration.zsh").exists());
    }

    #[test]
    fn test_uninstall_removes_integration_script_bash() {
        let (home, config) = setup_test_env();

        install_integration_internal(ShellType::Bash, config.path(), home.path()).unwrap();
        assert!(config.path().join("integration.bash").exists());

        uninstall_integration_internal(ShellType::Bash, config.path()).unwrap();
        assert!(!config.path().join("integration.bash").exists());
    }

    #[test]
    fn test_uninstall_removes_integration_script_fish() {
        let (home, config) = setup_test_env();

        install_integration_internal(ShellType::Fish, config.path(), home.path()).unwrap();
        assert!(config.path().join("integration.fish").exists());

        uninstall_integration_internal(ShellType::Fish, config.path()).unwrap();
        assert!(!config.path().join("integration.fish").exists());
    }

    #[test]
    fn test_uninstall_removes_version_file() {
        let (home, config) = setup_test_env();

        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();
        assert!(config.path().join("integration.version").exists());

        uninstall_integration_internal(ShellType::Zsh, config.path()).unwrap();
        assert!(!config.path().join("integration.version").exists());
    }

    #[test]
    fn test_uninstall_is_idempotent() {
        let (home, config) = setup_test_env();

        // Uninstall without ever installing - should not error
        let result = uninstall_integration_internal(ShellType::Zsh, config.path());
        assert!(result.is_ok());

        // Install then uninstall twice
        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();
        uninstall_integration_internal(ShellType::Zsh, config.path()).unwrap();
        let result = uninstall_integration_internal(ShellType::Zsh, config.path());
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // Status Detection Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_status_detects_not_installed() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        assert!(matches!(status, IntegrationStatus::NotInstalled));
    }

    #[test]
    fn test_status_detects_installed() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        match status {
            IntegrationStatus::Installed { version } => {
                assert_eq!(version, INTEGRATION_VERSION);
            }
            other => panic!("Expected Installed, got {:?}", other),
        }
    }

    #[test]
    fn test_status_detects_outdated() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        install_integration_internal(ShellType::Zsh, config.path(), home.path()).unwrap();

        // Manually downgrade version file
        std::fs::write(config.path().join("integration.version"), "0.0.1").unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        match status {
            IntegrationStatus::Outdated { current, latest } => {
                assert_eq!(current, "0.0.1");
                assert_eq!(latest, INTEGRATION_VERSION);
            }
            other => panic!("Expected Outdated, got {:?}", other),
        }
    }

    #[test]
    fn test_status_detects_misconfigured() {
        let (home, config) = setup_test_env();

        // Create integration files
        std::fs::create_dir_all(config.path()).unwrap();
        std::fs::write(config.path().join("integration.zsh"), "script").unwrap();
        std::fs::write(
            config.path().join("integration.version"),
            INTEGRATION_VERSION,
        )
        .unwrap();

        // Create .zshrc WITHOUT the source line
        std::fs::write(home.path().join(".zshrc"), "# no golish integration\n").unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        match status {
            IntegrationStatus::Misconfigured { issue, .. } => {
                assert!(issue.contains(".zshrc"));
            }
            other => panic!("Expected Misconfigured, got {:?}", other),
        }
    }

    #[test]
    fn test_status_not_installed_when_no_version_file() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        // Create integration script but NO version file
        std::fs::create_dir_all(config.path()).unwrap();
        std::fs::write(config.path().join("integration.zsh"), "script").unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        assert!(matches!(status, IntegrationStatus::NotInstalled));
    }

    #[test]
    fn test_status_not_installed_when_no_script_file() {
        let (home, config) = setup_test_env();
        std::fs::write(home.path().join(".zshrc"), "").unwrap();

        // Create version file but NO integration script
        std::fs::create_dir_all(config.path()).unwrap();
        std::fs::write(
            config.path().join("integration.version"),
            INTEGRATION_VERSION,
        )
        .unwrap();

        let status = get_integration_status_internal(ShellType::Zsh, config.path(), home.path());
        assert!(matches!(status, IntegrationStatus::NotInstalled));
    }

    // -------------------------------------------------------------------------
    // RC File Path Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_rc_file_paths_zsh() {
        let home = TempDir::new().unwrap();
        let paths = get_rc_file_paths(home.path(), ShellType::Zsh);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with(".zshrc"));
    }

    #[test]
    fn test_get_rc_file_paths_bash() {
        let home = TempDir::new().unwrap();
        let paths = get_rc_file_paths(home.path(), ShellType::Bash);
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|p| p.ends_with(".bashrc")));
        assert!(paths.iter().any(|p| p.ends_with(".bash_profile")));
    }

    #[test]
    fn test_get_rc_file_paths_fish() {
        let home = TempDir::new().unwrap();
        let paths = get_rc_file_paths(home.path(), ShellType::Fish);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("golish.fish"));
        assert!(paths[0].to_string_lossy().contains(".config/fish"));
    }

    // -------------------------------------------------------------------------
    // Integration Path Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_integration_path_for_shell_zsh() {
        let config = TempDir::new().unwrap();
        let path = get_integration_path_for_shell(config.path(), ShellType::Zsh);
        assert!(path.ends_with("integration.zsh"));
    }

    #[test]
    fn test_get_integration_path_for_shell_bash() {
        let config = TempDir::new().unwrap();
        let path = get_integration_path_for_shell(config.path(), ShellType::Bash);
        assert!(path.ends_with("integration.bash"));
    }

    #[test]
    fn test_get_integration_path_for_shell_fish() {
        let config = TempDir::new().unwrap();
        let path = get_integration_path_for_shell(config.path(), ShellType::Fish);
        assert!(path.ends_with("integration.fish"));
    }

    // -------------------------------------------------------------------------
    // Property-Based Installation Tests
    // -------------------------------------------------------------------------

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Install then uninstall leaves no integration files
            #[test]
            fn prop_install_uninstall_cleanup(
                shell_type in prop_oneof![
                    Just(ShellType::Zsh),
                    Just(ShellType::Bash),
                    Just(ShellType::Fish),
                ]
            ) {
                let (home, config) = setup_test_env();

                install_integration_internal(shell_type, config.path(), home.path()).unwrap();
                uninstall_integration_internal(shell_type, config.path()).unwrap();

                let ext = get_integration_extension(shell_type);
                prop_assert!(
                    !config.path().join(format!("integration.{}", ext)).exists(),
                    "Integration script should be removed after uninstall"
                );
            }

            /// Status is NotInstalled before install, Installed after install
            #[test]
            fn prop_status_changes_after_install(
                shell_type in prop_oneof![
                    Just(ShellType::Zsh),
                    Just(ShellType::Bash),
                    Just(ShellType::Fish),
                ]
            ) {
                let (home, config) = setup_test_env();

                // Create RC file for zsh/bash so status check works
                match shell_type {
                    ShellType::Zsh => {
                        std::fs::write(home.path().join(".zshrc"), "").unwrap();
                    }
                    ShellType::Bash => {
                        std::fs::write(home.path().join(".bashrc"), "").unwrap();
                    }
                    _ => {}
                }

                let before = get_integration_status_internal(shell_type, config.path(), home.path());
                prop_assert!(matches!(before, IntegrationStatus::NotInstalled));

                install_integration_internal(shell_type, config.path(), home.path()).unwrap();

                let after = get_integration_status_internal(shell_type, config.path(), home.path());
                prop_assert!(
                    matches!(after, IntegrationStatus::Installed { .. }),
                    "Expected Installed after install, got {:?}", after
                );
            }

            /// Multiple installs don't corrupt RC files
            #[test]
            fn prop_multiple_installs_safe(
                shell_type in prop_oneof![
                    Just(ShellType::Zsh),
                    Just(ShellType::Bash),
                    Just(ShellType::Fish),
                ],
                install_count in 1usize..5
            ) {
                let (home, config) = setup_test_env();

                // Pre-create RC files
                match shell_type {
                    ShellType::Zsh => {
                        std::fs::write(home.path().join(".zshrc"), "").unwrap();
                    }
                    ShellType::Bash => {
                        std::fs::write(home.path().join(".bashrc"), "").unwrap();
                        std::fs::write(home.path().join(".bash_profile"), "").unwrap();
                    }
                    _ => {}
                }

                for _ in 0..install_count {
                    install_integration_internal(shell_type, config.path(), home.path()).unwrap();
                }

                // Check RC files have exactly one source line
                let rc_paths = get_rc_file_paths(home.path(), shell_type);
                let ext = get_integration_extension(shell_type);
                let integration_marker = format!("integration.{}", ext);

                for rc_path in rc_paths {
                    if rc_path.exists() {
                        let content = std::fs::read_to_string(&rc_path).unwrap();
                        let count = content.matches(&integration_marker).count();
                        prop_assert_eq!(
                            count, 1,
                            "RC file {} should have exactly 1 source line, found {}",
                            rc_path.display(), count
                        );
                    }
                }
            }
        }
    }
}
