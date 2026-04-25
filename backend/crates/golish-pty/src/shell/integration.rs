//! [`ShellIntegration`] — automatic OSC 133 injection via ZDOTDIR / `--rcfile`.
//!
//! For zsh, uses the ZDOTDIR approach:
//! 1. Creates a wrapper `.zshrc` in a config directory.
//! 2. The wrapper sources the user's real `.zshrc` AND our integration
//!    script.
//! 3. `ZDOTDIR` is set to point to that wrapper directory.
//!
//! For bash, uses the `--rcfile` approach:
//! 1. Creates an integration script in a config directory.
//! 2. Creates a wrapper script that sources integration + user's `.bashrc`.
//! 3. Spawns bash with `--rcfile <wrapper>`.
//!
//! Either way, shell integration works without modifying user config files.

use std::fs;
use std::path::PathBuf;

use super::scripts::{BASH_INTEGRATION_SCRIPT, ZSH_INTEGRATION_SCRIPT, ZSH_WRAPPER_ZSHRC};
use super::ShellType;

/// Manages shell integration files for automatic OSC 133 injection.
pub struct ShellIntegration {
    /// The shell type this integration is for.
    shell_type: ShellType,
    /// Directory containing shell integration files.
    config_dir: PathBuf,
    /// Path to the integration script.
    integration_path: PathBuf,
}

impl ShellIntegration {
    /// Set up shell integration for the given shell type.
    ///
    /// Returns `None` for unsupported shells.
    pub fn setup(shell_type: ShellType) -> Option<Self> {
        match shell_type {
            ShellType::Zsh => Self::setup_zsh(),
            ShellType::Bash => Self::setup_bash(),
            // TODO: Add fish support via conf.d
            _ => None,
        }
    }

    /// Set up zsh integration using ZDOTDIR.
    fn setup_zsh() -> Option<Self> {
        // Use ~/.config/golish/shell as our ZDOTDIR.
        let config_dir = dirs::config_dir()?.join("golish").join("shell");

        if fs::create_dir_all(&config_dir).is_err() {
            tracing::warn!("Failed to create shell integration directory");
            return None;
        }

        let integration_path = config_dir.join("integration.zsh");
        if let Err(e) = fs::write(&integration_path, ZSH_INTEGRATION_SCRIPT) {
            tracing::warn!("Failed to write integration script: {}", e);
            return None;
        }

        let zshrc_path = config_dir.join(".zshrc");
        if let Err(e) = fs::write(&zshrc_path, ZSH_WRAPPER_ZSHRC) {
            tracing::warn!("Failed to write wrapper .zshrc: {}", e);
            return None;
        }

        tracing::debug!(
            zdotdir = %config_dir.display(),
            integration = %integration_path.display(),
            "Zsh integration configured"
        );

        Some(Self {
            shell_type: ShellType::Zsh,
            config_dir,
            integration_path,
        })
    }

    /// Set up bash integration using `--rcfile`.
    ///
    /// We create a wrapper script that:
    /// 1. Sources our integration script.
    /// 2. Sources the user's `~/.bashrc`.
    ///
    /// Then we use `--rcfile wrapper.bash` when spawning bash.
    fn setup_bash() -> Option<Self> {
        // Use ~/.config/golish/shell/bash for bash integration.
        let config_dir = dirs::config_dir()?.join("golish").join("shell").join("bash");

        if fs::create_dir_all(&config_dir).is_err() {
            tracing::warn!("Failed to create bash integration directory");
            return None;
        }

        let integration_path = config_dir.join("integration.bash");
        if let Err(e) = fs::write(&integration_path, BASH_INTEGRATION_SCRIPT) {
            tracing::warn!("Failed to write bash integration script: {}", e);
            return None;
        }

        // Write a wrapper script that sources integration + user's bashrc.
        let wrapper_path = config_dir.join("wrapper.bash");
        let wrapper_content = format!(
            r#"# Golish Bash Wrapper (auto-generated)
# Sources Golish integration before user's bashrc

# Source Golish integration first
if [[ -f "{integration}" ]]; then
    source "{integration}"
fi

# Source user's bashrc
if [[ -f "$HOME/.bashrc" ]]; then
    source "$HOME/.bashrc"
fi
"#,
            integration = integration_path.to_string_lossy()
        );
        if let Err(e) = fs::write(&wrapper_path, wrapper_content) {
            tracing::warn!("Failed to write bash wrapper script: {}", e);
            return None;
        }

        tracing::debug!(
            config_dir = %config_dir.display(),
            integration = %integration_path.display(),
            wrapper = %wrapper_path.display(),
            "Bash integration configured"
        );

        Some(Self {
            shell_type: ShellType::Bash,
            config_dir,
            integration_path,
        })
    }

    /// Get environment variables to set for the shell process.
    ///
    /// Returns a list of `(key, value)` pairs to set in the PTY environment.
    pub fn env_vars(&self) -> Vec<(&'static str, String)> {
        match self.shell_type {
            ShellType::Zsh => {
                let mut vars = vec![
                    ("ZDOTDIR", self.config_dir.to_string_lossy().to_string()),
                    (
                        "QBIT_INTEGRATION_PATH",
                        self.integration_path.to_string_lossy().to_string(),
                    ),
                ];

                // Preserve user's original ZDOTDIR if set, but only when it
                // differs from our wrapper dir. When a nested Golish inherits
                // ZDOTDIR pointing at the wrapper, forwarding it as
                // QBIT_REAL_ZDOTDIR would cause the wrapper .zshrc to source
                // itself, leading to infinite recursion ("job table full").
                if let Ok(original) = std::env::var("ZDOTDIR") {
                    let wrapper_dir = self.config_dir.to_string_lossy();
                    if original != wrapper_dir.as_ref() {
                        vars.push(("QBIT_REAL_ZDOTDIR", original));
                    }
                }

                vars
            }
            ShellType::Bash => {
                vec![(
                    "QBIT_INTEGRATION_PATH",
                    self.integration_path.to_string_lossy().to_string(),
                )]
            }
            _ => vec![],
        }
    }

    /// Get additional arguments to pass to the shell.
    ///
    /// For bash, this returns `["--rcfile", "/path/to/wrapper.bash"]`.
    /// For other shells, returns empty.
    pub fn shell_args(&self) -> Vec<String> {
        match self.shell_type {
            ShellType::Bash => {
                let wrapper_path = self.config_dir.join("wrapper.bash");
                vec![
                    "--rcfile".to_string(),
                    wrapper_path.to_string_lossy().to_string(),
                ]
            }
            _ => vec![],
        }
    }
}
