use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::time::timeout;

use golish_core::Tool;
use golish_pty::PtyManager;

#[derive(Debug, Clone)]
pub struct PtyOutputEvent {
    pub session_id: String,
    pub data: String,
}

/// Shared broadcast channel that taps into PTY output events.
///
/// Fed by a Tauri event listener for `terminal_output` events,
/// allowing tools to subscribe and capture PTY output.
pub struct PtyOutputTap {
    sender: broadcast::Sender<PtyOutputEvent>,
}

impl Default for PtyOutputTap {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyOutputTap {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1024);
        Self { sender }
    }

    /// Feed an output event into the tap (called from Tauri event listener).
    pub fn feed(&self, session_id: String, data: String) {
        let _ = self.sender.send(PtyOutputEvent { session_id, data });
    }

    /// Subscribe to output events.
    pub fn subscribe(&self) -> broadcast::Receiver<PtyOutputEvent> {
        self.sender.subscribe()
    }
}

const DEFAULT_TIMEOUT_MS: u64 = 10000;
const MAX_TIMEOUT_MS: u64 = 120000;

/// Run a shell command outside the visible terminal and return structured
/// output for the AI tool detail panel.
///
/// This deliberately avoids `PtyManager::write`, so AI-originated shell
/// commands do not create `CommandBlock`s in the user's terminal timeline.
pub(crate) async fn run_shell_command_detail(
    command: &str,
    workspace: &Path,
    timeout_ms: u64,
) -> Result<Value> {
    let started_at = std::time::Instant::now();
    let timeout_duration = Duration::from_millis(timeout_ms);

    let mut cmd = shell_command(command);
    cmd.current_dir(workspace).kill_on_drop(true);

    tracing::info!(
        "[run_pty_cmd] Executing in background for tool detail: command={}, timeout_ms={}",
        command,
        timeout_ms
    );

    let output_result = timeout(timeout_duration, cmd.output()).await;
    let duration_ms = started_at.elapsed().as_millis() as u64;

    let output = match output_result {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Ok(json!({
                "error": format!("Failed to execute command: {e}"),
                "command": command,
                "exit_code": 1,
                "duration_ms": duration_ms,
            }));
        }
        Err(_) => {
            return Ok(json!({
                "error": format!("Command timed out after {}ms", timeout_ms),
                "command": command,
                "stdout": "",
                "stderr": "",
                "exit_code": 124,
                "timed_out": true,
                "duration_ms": duration_ms,
            }));
        }
    };

    Ok(json!({
        "stdout": truncate_output(String::from_utf8_lossy(&output.stdout).into_owned()),
        "stderr": truncate_output(String::from_utf8_lossy(&output.stderr).into_owned()),
        "command": command,
        "exit_code": output.status.code().unwrap_or(-1),
        "duration_ms": duration_ms,
    }))
}

#[cfg(unix)]
fn shell_command(command: &str) -> tokio::process::Command {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-lc").arg(command);
    cmd
}

#[cfg(windows)]
fn shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd.exe");
    cmd.arg("/C").arg(command);
    cmd
}

fn truncate_output(output: String) -> String {
    let max_output_len = 50_000;
    if output.len() <= max_output_len {
        return output;
    }

    let end = output.floor_char_boundary(max_output_len);
    format!(
        "{}...\n[Output truncated, {} bytes total]",
        &output[..end],
        output.len()
    )
}

/// Drop-in replacement for `RunPtyCmdTool` that returns shell output to the
/// AI tool detail panel instead of writing into the user's visible terminal.
pub struct VisibleRunPtyCmdTool;

impl VisibleRunPtyCmdTool {
    pub fn new(
        _pty_manager: Arc<PtyManager>,
        _output_tap: Arc<PtyOutputTap>,
        _active_session: Arc<parking_lot::Mutex<Option<String>>>,
    ) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for VisibleRunPtyCmdTool {
    fn name(&self) -> &'static str {
        "run_pty_cmd"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command and return stdout/stderr/exit_code. \
         Output is shown in the AI tool detail panel, not in the user's terminal timeline."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 10, max: 120)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: command"))?;

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS / 1000);
        let timeout_ms = (timeout_secs * 1000).min(MAX_TIMEOUT_MS);

        run_shell_command_detail(command, workspace, timeout_ms).await
    }
}
