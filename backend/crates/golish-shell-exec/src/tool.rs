//! [`RunPtyCmdTool`] — synchronous, blocking shell-command tool.
//!
//! This is the [`golish_core::Tool`] implementation registered with the
//! agent's tool registry. For real-time output prefer
//! [`crate::streaming::execute_streaming`].

use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::debug;

use golish_core::utils::{get_optional_str, get_optional_u64, get_required_str};
use golish_core::Tool;

use crate::common::{resolve_cwd, truncate_output, DEFAULT_TIMEOUT_SECS, MAX_OUTPUT_SIZE};
use crate::process_group::{configure_process_group, kill_process_group};
use crate::shell::get_shell_config;

/// Tool for executing shell commands.
///
/// Shell resolution order:
/// 1. `shell_override` field (from `settings.toml` `terminal.shell`).
/// 2. `$SHELL` environment variable.
/// 3. Fall back to `/bin/sh`.
#[derive(Default)]
pub struct RunPtyCmdTool {
    /// Optional shell override from settings. When set, this takes
    /// priority over the `$SHELL` environment variable.
    pub(crate) shell_override: Option<String>,
}

impl RunPtyCmdTool {
    /// Create a new [`RunPtyCmdTool`] with default shell resolution.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new [`RunPtyCmdTool`] with a shell override from settings.
    ///
    /// The shell override takes priority over the `$SHELL` environment
    /// variable. Pass `None` to use the default shell resolution order.
    pub fn with_shell(shell: Option<String>) -> Self {
        Self {
            shell_override: shell,
        }
    }
}

#[async_trait::async_trait]
impl Tool for RunPtyCmdTool {
    fn name(&self) -> &'static str {
        "run_pty_cmd"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command and return the output. Commands run in a shell environment with access to common tools."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory (relative to workspace)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let command_str = match get_required_str(&args, "command") {
            Ok(c) => c,
            Err(e) => return Ok(e),
        };

        let cwd = get_optional_str(&args, "cwd");
        let timeout_secs = get_optional_u64(&args, "timeout").unwrap_or(DEFAULT_TIMEOUT_SECS);

        let working_dir = resolve_cwd(cwd, workspace);

        if !working_dir.exists() {
            return Ok(json!({
                "error": format!("Working directory not found: {}", working_dir.display()),
                "exit_code": 1
            }));
        }

        // Determine shell and command to use.
        let (shell, wrapped_command) = if cfg!(target_os = "windows") {
            ("cmd".to_string(), command_str.to_string())
        } else {
            let (shell_path, shell_type, home) = get_shell_config(self.shell_override.as_deref());
            shell_type.build_command(&shell_path, command_str, &home)
        };

        let shell_arg = if cfg!(target_os = "windows") { "/c" } else { "-c" };

        debug!(
            shell = %shell,
            original_command = %command_str,
            wrapped_command = %wrapped_command,
            "Executing shell command"
        );

        let mut cmd = Command::new(&shell);
        cmd.arg(shell_arg)
            .arg(&wrapped_command)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        configure_process_group(&mut cmd);

        cmd.env("TERM", "xterm-256color");
        cmd.env("CLICOLOR", "1");
        cmd.env("CLICOLOR_FORCE", "1");

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(json!({
                    "error": format!("Failed to spawn command: {}", e),
                    "exit_code": 1
                }));
            }
        };

        let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout_duration, async {
            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();

            if let Some(mut stdout) = child.stdout.take() {
                let _ = stdout.read_to_end(&mut stdout_buf).await;
            }
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_end(&mut stderr_buf).await;
            }

            let status = child.wait().await;
            (stdout_buf, stderr_buf, status)
        })
        .await;

        match result {
            Ok((stdout_buf, stderr_buf, status)) => {
                let stdout = truncate_output(&stdout_buf, MAX_OUTPUT_SIZE);
                let stderr = truncate_output(&stderr_buf, MAX_OUTPUT_SIZE);
                let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

                let mut response = json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": exit_code,
                    "command": command_str
                });

                if let Some(c) = cwd {
                    response["cwd"] = json!(c);
                }

                if exit_code != 0 {
                    response["error"] = json!(format!(
                        "Command exited with code {}: {}",
                        exit_code,
                        if stderr.is_empty() { &stdout } else { &stderr }
                    ));
                }

                Ok(response)
            }
            Err(_) => {
                kill_process_group(&mut child).await;

                Ok(json!({
                    "error": format!("Command timed out after {} seconds", timeout_secs),
                    "exit_code": 124, // Standard timeout exit code
                    "command": command_str,
                    "timeout": true
                }))
            }
        }
    }
}
