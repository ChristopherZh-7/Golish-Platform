use crate::error::{GolishError, Result};
use crate::pty::ShellType;
use std::fs;
use std::io::Write;
#[cfg(test)]
use std::path::PathBuf;

mod scripts;
#[cfg(test)]
mod tests;
mod types;

pub use scripts::get_integration_script;
pub use types::IntegrationStatus;

#[cfg(test)]
pub(crate) use scripts::get_integration_extension;
pub(crate) use scripts::INTEGRATION_VERSION;
pub(crate) use types::{get_config_dir, get_integration_path, get_version_path, get_zshrc_path};

#[cfg(test)]
/// Get integration script path for a specific shell type within a config directory
fn get_integration_path_for_shell(config_dir: &std::path::Path, shell_type: ShellType) -> PathBuf {
    let filename = format!("integration.{}", get_integration_extension(shell_type));
    config_dir.join(filename)
}

#[cfg(test)]
/// Get RC file paths for a shell type within a home directory
/// Returns multiple paths for shells that need multiple RC files (e.g., bash)
fn get_rc_file_paths(home_dir: &std::path::Path, shell_type: ShellType) -> Vec<PathBuf> {
    match shell_type {
        ShellType::Zsh => vec![home_dir.join(".zshrc")],
        ShellType::Bash => vec![home_dir.join(".bashrc"), home_dir.join(".bash_profile")],
        ShellType::Fish => vec![home_dir.join(".config/fish/conf.d/golish.fish")],
        ShellType::Unknown => vec![home_dir.join(".zshrc")], // Default to zsh
    }
}

#[cfg(test)]
/// Install shell integration for a specific shell type
/// This is the testable version that accepts path parameters
fn install_integration_internal(
    shell_type: ShellType,
    config_dir: &std::path::Path,
    home_dir: &std::path::Path,
) -> Result<()> {
    // Create config directory
    fs::create_dir_all(config_dir).map_err(GolishError::Io)?;

    // Write integration script
    let script_path = get_integration_path_for_shell(config_dir, shell_type);
    fs::write(&script_path, get_integration_script(shell_type)).map_err(GolishError::Io)?;

    // Write version marker
    let version_path = config_dir.join("integration.version");
    fs::write(&version_path, INTEGRATION_VERSION).map_err(GolishError::Io)?;

    // Update RC files
    let rc_paths = get_rc_file_paths(home_dir, shell_type);
    for rc_path in rc_paths {
        update_rc_file_internal(&rc_path, &script_path, shell_type)?;
    }

    Ok(())
}

#[cfg(test)]
/// Update a single RC file to source the integration script
fn update_rc_file_internal(
    rc_path: &std::path::Path,
    integration_path: &std::path::Path,
    shell_type: ShellType,
) -> Result<()> {
    // Create parent directories if needed (for fish config)
    if let Some(parent) = rc_path.parent() {
        fs::create_dir_all(parent).map_err(GolishError::Io)?;
    }

    let source_line = match shell_type {
        ShellType::Fish => format!(
            r#"
# Golish shell integration
if test "$QBIT" = "1"
    source "{}"
end
"#,
            integration_path.display()
        ),
        _ => format!(
            r#"
# Golish shell integration
[[ -n "$QBIT" ]] && source "{}"
"#,
            integration_path.display()
        ),
    };

    if rc_path.exists() {
        let content = fs::read_to_string(rc_path).map_err(GolishError::Io)?;
        let integration_path_str = integration_path.display().to_string();

        // Check if already configured correctly
        if content.contains(&integration_path_str) {
            return Ok(());
        }

        // Check if there's an old golish integration line that needs updating
        if content.contains("golish/integration.") || content.contains("golish\\integration.") {
            // Remove old integration lines and add new one
            let mut new_lines: Vec<&str> = Vec::new();
            let mut skip_next = false;

            for line in content.lines() {
                if line.trim() == "# Golish shell integration" {
                    skip_next = true;
                    continue;
                }

                if skip_next
                    && (line.contains("golish/integration.")
                        || line.contains("golish\\integration."))
                {
                    skip_next = false;
                    continue;
                }

                // Fish has different structure - skip the 'end' too
                if skip_next && shell_type == ShellType::Fish && line.trim() == "end" {
                    skip_next = false;
                    continue;
                }

                skip_next = false;
                new_lines.push(line);
            }

            let mut new_content = new_lines.join("\n");
            if !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push_str(&source_line);

            fs::write(rc_path, new_content).map_err(GolishError::Io)?;
            return Ok(());
        }
    }

    // No existing integration, append new one
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_path)
        .map_err(GolishError::Io)?;

    writeln!(file, "{}", source_line).map_err(GolishError::Io)?;

    Ok(())
}

#[cfg(test)]
/// Uninstall shell integration for a specific shell type
fn uninstall_integration_internal(
    shell_type: ShellType,
    config_dir: &std::path::Path,
) -> Result<()> {
    let script_path = get_integration_path_for_shell(config_dir, shell_type);
    let version_path = config_dir.join("integration.version");

    if script_path.exists() {
        fs::remove_file(&script_path).map_err(GolishError::Io)?;
    }
    if version_path.exists() {
        fs::remove_file(&version_path).map_err(GolishError::Io)?;
    }

    Ok(())
}

#[cfg(test)]
/// Get integration status for a specific shell type
fn get_integration_status_internal(
    shell_type: ShellType,
    config_dir: &std::path::Path,
    home_dir: &std::path::Path,
) -> IntegrationStatus {
    let script_path = get_integration_path_for_shell(config_dir, shell_type);
    let version_path = config_dir.join("integration.version");

    // Check if version file exists
    if !version_path.exists() {
        return IntegrationStatus::NotInstalled;
    }

    // Check if integration script exists
    if !script_path.exists() {
        return IntegrationStatus::NotInstalled;
    }

    // Read current version
    let current_version = match fs::read_to_string(&version_path) {
        Ok(v) => v.trim().to_string(),
        Err(_) => return IntegrationStatus::NotInstalled,
    };

    // Check if RC file has correct source line
    let rc_paths = get_rc_file_paths(home_dir, shell_type);
    let script_path_str = script_path.display().to_string();

    let mut any_configured = false;
    for rc_path in &rc_paths {
        if rc_path.exists() {
            if let Ok(content) = fs::read_to_string(rc_path) {
                if content.contains(&script_path_str) {
                    any_configured = true;
                    break;
                }
            }
        }
    }

    if !any_configured && !rc_paths.is_empty() {
        // Check if any RC file exists but doesn't have our integration
        for rc_path in &rc_paths {
            if rc_path.exists() {
                return IntegrationStatus::Misconfigured {
                    expected_path: script_path_str,
                    issue: format!("No Golish integration found in {}", rc_path.display()),
                };
            }
        }
    }

    if current_version == INTEGRATION_VERSION {
        IntegrationStatus::Installed {
            version: current_version,
        }
    } else {
        IntegrationStatus::Outdated {
            current: current_version,
            latest: INTEGRATION_VERSION.to_string(),
        }
    }
}

/// Validates that the .zshrc sources the integration script from the correct path
fn validate_zshrc_integration() -> Result<Option<String>> {
    let zshrc_path = get_zshrc_path()
        .ok_or_else(|| GolishError::Internal("Could not determine home directory".into()))?;

    let integration_path = get_integration_path()
        .ok_or_else(|| GolishError::Internal("Could not determine integration path".into()))?;

    if !zshrc_path.exists() {
        return Ok(Some("No .zshrc file found".to_string()));
    }

    let content = fs::read_to_string(&zshrc_path).map_err(GolishError::Io)?;

    // Check if there's any golish integration line
    if !content.contains("golish/integration.zsh") && !content.contains("golish\\integration.zsh") {
        return Ok(Some("No Golish integration found in .zshrc".to_string()));
    }

    // Check if the correct path is referenced
    let expected_path_str = integration_path.display().to_string();
    if !content.contains(&expected_path_str) {
        // Try to find what path is actually being used
        for line in content.lines() {
            if line.contains("golish/integration.zsh") || line.contains("golish\\integration.zsh") {
                if line.trim().starts_with('#') && !line.contains("# Golish shell integration") {
                    continue; // Skip comments that aren't our marker
                }
                return Ok(Some(format!(
                    "Incorrect path in .zshrc. Expected: {}",
                    expected_path_str
                )));
            }
        }
    }

    Ok(None) // No issues found
}

/// Get the current git branch for a directory
/// Returns None if the directory is not in a git repository
#[tauri::command]
pub async fn get_git_branch(path: String) -> std::result::Result<Option<String>, String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if branch.is_empty() {
                Ok(None)
            } else {
                Ok(Some(branch))
            }
        }
        _ => Ok(None), // Not a git repo or git not available
    }
}

#[tauri::command]
pub async fn shell_integration_status() -> Result<IntegrationStatus> {
    let version_path = get_version_path()
        .ok_or_else(|| GolishError::Internal("Could not determine config directory".into()))?;

    let integration_path = get_integration_path()
        .ok_or_else(|| GolishError::Internal("Could not determine integration path".into()))?;

    if !version_path.exists() {
        return Ok(IntegrationStatus::NotInstalled);
    }

    // Check if integration script actually exists
    if !integration_path.exists() {
        return Ok(IntegrationStatus::NotInstalled);
    }

    let current_version = fs::read_to_string(&version_path)
        .map_err(GolishError::Io)?
        .trim()
        .to_string();

    // Validate .zshrc has correct path
    if let Some(issue) = validate_zshrc_integration()? {
        return Ok(IntegrationStatus::Misconfigured {
            expected_path: integration_path.display().to_string(),
            issue,
        });
    }

    if current_version == INTEGRATION_VERSION {
        Ok(IntegrationStatus::Installed {
            version: current_version,
        })
    } else {
        Ok(IntegrationStatus::Outdated {
            current: current_version,
            latest: INTEGRATION_VERSION.to_string(),
        })
    }
}

#[tauri::command]
pub async fn shell_integration_install() -> Result<()> {
    let config_dir = get_config_dir()
        .ok_or_else(|| GolishError::Internal("Could not determine config directory".into()))?;

    // Create config directory
    fs::create_dir_all(&config_dir).map_err(GolishError::Io)?;

    // Write integration script (currently zsh-only, will be extended for multi-shell)
    let script_path = config_dir.join("integration.zsh");
    fs::write(&script_path, get_integration_script(ShellType::Zsh)).map_err(GolishError::Io)?;

    // Write version marker
    let version_path = config_dir.join("integration.version");
    fs::write(&version_path, INTEGRATION_VERSION).map_err(GolishError::Io)?;

    // Update .zshrc
    update_zshrc()?;

    Ok(())
}

#[tauri::command]
pub async fn shell_integration_uninstall() -> Result<()> {
    let config_dir = get_config_dir()
        .ok_or_else(|| GolishError::Internal("Could not determine config directory".into()))?;

    let script_path = config_dir.join("integration.zsh");
    let version_path = config_dir.join("integration.version");

    if script_path.exists() {
        fs::remove_file(&script_path).map_err(GolishError::Io)?;
    }
    if version_path.exists() {
        fs::remove_file(&version_path).map_err(GolishError::Io)?;
    }

    Ok(())
}

fn update_zshrc() -> Result<()> {
    let zshrc_path = get_zshrc_path()
        .ok_or_else(|| GolishError::Internal("Could not determine home directory".into()))?;

    let integration_path = get_integration_path()
        .ok_or_else(|| GolishError::Internal("Could not determine integration path".into()))?;

    let source_line = format!(
        r#"
# Golish shell integration
[[ -n "$QBIT" ]] && source "{}"
"#,
        integration_path.display()
    );

    if zshrc_path.exists() {
        let content = fs::read_to_string(&zshrc_path).map_err(GolishError::Io)?;
        let expected_path_str = integration_path.display().to_string();

        // Check if correctly configured
        if content.contains(&expected_path_str) {
            return Ok(());
        }

        // Check if there's an old/incorrect golish integration line that needs fixing
        if content.contains("golish/integration.zsh") || content.contains("golish\\integration.zsh")
        {
            // Remove old integration lines and add correct one
            let mut new_lines: Vec<&str> = Vec::new();
            let mut skip_next = false;
            let mut found_and_replaced = false;

            for line in content.lines() {
                // Skip the comment line before source command
                if line.trim() == "# Golish shell integration" {
                    skip_next = true;
                    continue;
                }

                // Skip the old source line
                if skip_next
                    && (line.contains("golish/integration.zsh")
                        || line.contains("golish\\integration.zsh"))
                {
                    skip_next = false;
                    // Only add replacement once
                    if !found_and_replaced {
                        // We'll append the new integration at the end
                        found_and_replaced = true;
                    }
                    continue;
                }

                skip_next = false;
                new_lines.push(line);
            }

            // Write updated content
            let mut new_content = new_lines.join("\n");
            if !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push_str(&source_line);

            fs::write(&zshrc_path, new_content).map_err(GolishError::Io)?;
            return Ok(());
        }
    }

    // No existing integration, append new one
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&zshrc_path)
        .map_err(GolishError::Io)?;

    writeln!(file, "{}", source_line).map_err(GolishError::Io)?;

    Ok(())
}
