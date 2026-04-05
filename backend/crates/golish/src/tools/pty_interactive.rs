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
const IDLE_THRESHOLD_MS: u64 = 2000;

/// Drop-in replacement for `RunPtyCmdTool` that routes commands through
/// the user's visible terminal instead of spawning a background process.
///
/// Registered with name "run_pty_cmd", this replaces the default background
/// execution so the AI's commands appear in the user's terminal tabs.
pub struct VisibleRunPtyCmdTool {
    pty_manager: Arc<PtyManager>,
    output_tap: Arc<PtyOutputTap>,
    active_session: Arc<parking_lot::Mutex<Option<String>>>,
}

impl VisibleRunPtyCmdTool {
    pub fn new(
        pty_manager: Arc<PtyManager>,
        output_tap: Arc<PtyOutputTap>,
        active_session: Arc<parking_lot::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            pty_manager,
            output_tap,
            active_session,
        }
    }

    fn resolve_session(&self) -> Result<String, Value> {
        if let Some(active) = self.active_session.lock().clone() {
            tracing::info!("[run_pty_cmd] Using active terminal session: {}", active);
            return Ok(active);
        }
        let sessions = self.pty_manager.list_session_ids();
        if sessions.is_empty() {
            return Err(json!({
                "error": "No active terminal sessions. Please open a terminal tab first.",
                "exit_code": 1
            }));
        }
        let first = sessions.into_iter().next().unwrap();
        tracing::info!("[run_pty_cmd] No active session set, falling back to: {}", first);
        Ok(first)
    }
}

#[async_trait::async_trait]
impl Tool for VisibleRunPtyCmdTool {
    fn name(&self) -> &'static str {
        "run_pty_cmd"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command in the user's visible terminal and return the output. \
         Commands run in the active terminal tab so the user can see the execution. \
         Supports interactive commands like SSH, docker, etc."
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

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let session_id = match self.resolve_session() {
            Ok(sid) => sid,
            Err(err_val) => return Ok(err_val),
        };

        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: command"))?;

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS / 1000);
        let timeout_ms = (timeout_secs * 1000).min(MAX_TIMEOUT_MS);

        let input = format!("{}\n", command);

        tracing::info!(
            "[run_pty_cmd] Executing in visible terminal: session={}, command={}, timeout_ms={}",
            session_id, command, timeout_ms
        );

        if let Err(e) = self.pty_manager.get_session(&session_id) {
            return Ok(json!({
                "error": format!("PTY session not found: {}", e),
                "exit_code": 1
            }));
        }

        // Subscribe to output BEFORE writing so we don't miss anything
        let mut rx = self.output_tap.subscribe();

        if let Err(e) = self.pty_manager.write(&session_id, input.as_bytes()) {
            return Ok(json!({
                "error": format!("Failed to write to terminal: {}", e),
                "exit_code": 1
            }));
        }

        // Collect output with timeout and idle detection
        let mut output = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let idle_duration = Duration::from_millis(IDLE_THRESHOLD_MS);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            let wait_time = remaining.min(idle_duration);

            match timeout(wait_time, rx.recv()).await {
                Ok(Ok(event)) if event.session_id == session_id => {
                    output.push_str(&event.data);
                }
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    tracing::warn!("PTY output tap lagged by {} messages", n);
                    continue;
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => {
                    if !output.is_empty() {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                }
            }
        }

        let max_output_len = 50_000;
        let truncated = output.len() > max_output_len;
        let stdout = if truncated {
            let end = output.floor_char_boundary(max_output_len);
            format!("{}...\n[Output truncated, {} bytes total]", &output[..end], output.len())
        } else {
            output
        };

        Ok(json!({
            "stdout": stdout,
            "command": command,
            "exit_code": 0,
        }))
    }
}
