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
            ShellType::PowerShell | ShellType::Cmd | ShellType::Unknown => {
                prop_assert_eq!(ext, "zsh")
            }
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
