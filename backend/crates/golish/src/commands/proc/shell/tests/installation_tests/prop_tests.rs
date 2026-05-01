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
