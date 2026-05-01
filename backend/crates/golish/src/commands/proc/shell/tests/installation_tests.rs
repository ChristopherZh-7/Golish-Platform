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

mod prop_tests;
