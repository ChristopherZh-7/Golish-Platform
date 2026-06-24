use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::broadcast;

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

/// Inline wait cap before a still-running command is moved to the background.
const DEFAULT_SOFT_TIMEOUT_MS: u64 = 30_000;
/// Background hard limit: runaway jobs are killed after this so nothing leaks.
const DEFAULT_HARD_TIMEOUT_MS: u64 = 1_800_000; // 30 min
/// Background hard limit for DNS zone-transfer (AXFR) probes. These hang
/// indefinitely against resolvers that silently drop TCP zone transfers, so the
/// 30-minute default would let a single hung probe pin a whole stage's closeout
/// reconciliation barrier. Capped aggressively so they fail fast instead.
const DEFAULT_DNS_HARD_TIMEOUT_MS: u64 = 15_000; // 15s

fn env_ms(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

/// True when `command` is a DNS zone-transfer probe (`dig AXFR`, `host -l`, …).
/// These are prone to hanging forever against resolvers that drop TCP AXFR, so
/// the caller caps their background hard limit (see [`compute_hard_ms`]).
fn is_dns_zone_transfer(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    c.contains("axfr") || c.contains("host -l")
}

/// Background hard limit (ms) for `command` given the caller's `timeout_ms`.
///
/// Normally the global default (30 min, raised to outlast `timeout_ms`), but
/// capped hard for DNS zone-transfer probes so one hung `dig AXFR` cannot keep a
/// stage's reconciliation barrier open for the full default.
fn compute_hard_ms(command: &str, timeout_ms: u64) -> u64 {
    let hard_ms = env_ms("GOLISH_TOOL_HARD_TIMEOUT_MS", DEFAULT_HARD_TIMEOUT_MS).max(timeout_ms);
    if is_dns_zone_transfer(command) {
        hard_ms.min(env_ms(
            "GOLISH_DNS_HARD_TIMEOUT_MS",
            DEFAULT_DNS_HARD_TIMEOUT_MS,
        ))
    } else {
        hard_ms
    }
}

/// Run a shell command outside the visible terminal and return structured
/// output for the AI tool detail panel.
///
/// This deliberately avoids `PtyManager::write`, so AI-originated shell
/// commands do not create `CommandBlock`s in the user's terminal timeline.
///
/// The command is spawned through the [`crate::background_jobs`] manager: if it
/// finishes within the *soft* timeout the full result is returned as before;
/// otherwise it keeps running in the background and a
/// `{ status: "backgrounded", job_id }` handle is returned (success-shaped, so
/// the agentic loop does not treat it as a failure). The AI polls progress via
/// the `check_job` tool.
pub async fn run_shell_command_detail(
    command: &str,
    workspace: &Path,
    timeout_ms: u64,
    // AI-elected background (Cursor-style): when true, hand back the job handle
    // immediately without waiting the soft timeout, so the agent continues async.
    background: bool,
) -> Result<Value> {
    let soft_cap = env_ms("GOLISH_TOOL_SOFT_TIMEOUT_MS", DEFAULT_SOFT_TIMEOUT_MS);
    let soft_ms = timeout_ms.min(soft_cap).max(1);
    // The background hard limit normally outlasts the caller's timeout so the job
    // can continue past the point where it would previously have been killed —
    // except DNS zone-transfer probes, which are capped short (they hang forever
    // against resolvers that drop TCP AXFR). See [`compute_hard_ms`].
    let hard_ms = compute_hard_ms(command, timeout_ms);

    let started_at = std::time::Instant::now();
    // Attribute the job to the session whose agentic loop is currently running
    // (set via `golish_core::with_agent_session`), so the completion broadcast
    // can be routed back to that session. Capture the current tool context too
    // so live stdout/stderr chunks can update the existing tool-call detail UI.
    // `None` when not attributable.
    let session_id = golish_core::current_agent_session();
    let tool_context = golish_core::current_agent_tool_context();
    let job_id = crate::background_jobs::manager().spawn_for_session_and_tool(
        command,
        workspace,
        Duration::from_millis(hard_ms),
        session_id.clone(),
        tool_context,
    );

    tracing::info!(
        "[run_pty_cmd] backgrounded spawn: command={}, soft_ms={}, hard_ms={}, job_id={}, session={:?}",
        command,
        soft_ms,
        hard_ms,
        job_id,
        session_id
    );

    // AI-elected background (Cursor-style): skip the soft-timeout wait and hand back
    // the handle now so the agent proceeds asynchronously. Otherwise wait up to the
    // soft timeout for an inline result (poll the manager).
    if !background {
        let soft_deadline = started_at + Duration::from_millis(soft_ms);
        loop {
            if let Some(snap) = crate::background_jobs::manager().snapshot(&job_id) {
                if snap.finished {
                    crate::background_jobs::manager().remove(&job_id);
                    return Ok(json!({
                        "stdout": truncate_output(snap.stdout),
                        "stderr": truncate_output(snap.stderr),
                        "command": command,
                        "exit_code": snap.exit_code.unwrap_or(-1),
                        "duration_ms": snap.duration_ms,
                    }));
                }
            }
            if std::time::Instant::now() >= soft_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // Still running → hand back a background handle. Deliberately no `error` and
    // no non-zero `exit_code`, so the agentic loop treats this as a successful
    // tool result and the model reads `status: "backgrounded"`.
    let (partial_stdout, partial_stderr) = crate::background_jobs::manager()
        .snapshot(&job_id)
        .map(|s| (truncate_output(s.stdout), truncate_output(s.stderr)))
        .unwrap_or_default();
    let hint = if background {
        format!(
            "Started in the background as requested (job {job_id}); continue with other work. \
             Its result is auto-delivered when it finishes — do NOT poll it in a loop or re-run \
             the same command. Reconcile any background jobs before you conclude the task. If it \
             still hasn't finished much later, `check_job` it ONCE: if it's stuck with no new \
             output (e.g. a hung DNS AXFR), `kill_job` it and move on instead of waiting."
        )
    } else {
        format!(
            "Command exceeded the {}s soft timeout and is STILL RUNNING in the background (job {}). \
             It was NOT killed. Its result is auto-delivered when it finishes — do NOT re-run the \
             same command. Poll `check_job` at most once if you need interim output; if it shows \
             the job stuck with no new output, `kill_job` it and proceed (do not wait out the \
             30-minute hard timeout).",
            soft_ms / 1000,
            job_id
        )
    };
    Ok(json!({
        "status": "backgrounded",
        "job_id": job_id,
        "command": command,
        "partial_stdout": partial_stdout,
        "partial_stderr": partial_stderr,
        "soft_timeout_ms": soft_ms,
        "hint": hint,
    }))
}

#[cfg(unix)]
pub(crate) fn shell_command(command: &str) -> tokio::process::Command {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-lc").arg(command);
    cmd
}

#[cfg(windows)]
pub(crate) fn shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd.exe");
    cmd.arg("/C").arg(command);
    cmd
}

fn truncate_output(output: String) -> String {
    let max_output_len = 50_000;
    if output.len() <= max_output_len {
        return output;
    }

    // Byte-bounded, char-boundary-safe head truncation via the canonical
    // `golish_core::utils::truncate_str`, plus this caller's size note.
    format!(
        "{}...\n[Output truncated, {} bytes total]",
        golish_core::utils::truncate_str(&output, max_output_len),
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
                },
                "background": {
                    "type": "boolean",
                    "description": "Run in the background (Cursor-style): return immediately with a job handle and keep the command running so you can proceed with other work. Use for long commands whose output you don't need right now; the result is auto-delivered when it finishes."
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
        let background = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        run_shell_command_detail(command, workspace, timeout_ms, background).await
    }
}

/// Tool that lets the AI poll a background job started when a shell/pentest
/// command exceeded its soft timeout (see [`run_shell_command_detail`]).
pub struct CheckJobTool;

#[async_trait::async_trait]
impl Tool for CheckJobTool {
    fn name(&self) -> &'static str {
        "check_job"
    }

    fn description(&self) -> &'static str {
        "Poll a background job (created when a shell/pentest command exceeded its soft timeout and was \
         moved to the background instead of being killed). Returns its status \
         (running/done/failed/killed), the command's exit code once finished, and the latest \
         stdout/stderr. Pass the job_id from a tool result whose status was \"backgrounded\". \
         If this shows a job still running with no new output for a long time, it is likely stuck \
         (e.g. a hung DNS AXFR / zone-transfer) — cancel it with kill_job and move on."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job_id returned in a tool result with status \"backgrounded\"."
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let job_id = args
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: job_id"))?;

        match crate::background_jobs::manager().snapshot(job_id) {
            Some(snap) => Ok(json!({
                "job_id": job_id,
                "status": snap.status.as_str(),
                "running": !snap.finished,
                // The *command's* exit code is reported as `job_exit_code` (NOT a
                // top-level `exit_code`) so a failed background command does not
                // make this `check_job` call itself read as a tool failure.
                "job_exit_code": snap.exit_code,
                "command": snap.command,
                "stdout": truncate_output(snap.stdout),
                "stderr": truncate_output(snap.stderr),
                "duration_ms": snap.duration_ms,
            })),
            None => Ok(json!({
                "error": format!("No background job with id '{job_id}' (it may have finished and been cleared)."),
            })),
        }
    }
}

/// Tool that lets the AI cancel a background job that is stuck or no longer
/// needed (e.g. a DNS AXFR / zone-transfer probe that hangs against a resolver
/// dropping TCP zone transfers). Closes the Cursor-style loop: `check_job` shows
/// no progress → `kill_job` it → continue or re-run differently, instead of
/// waiting out the hard-timeout watchdog.
pub struct KillJobTool;

#[async_trait::async_trait]
impl Tool for KillJobTool {
    fn name(&self) -> &'static str {
        "kill_job"
    }

    fn description(&self) -> &'static str {
        "Cancel a background job that is stuck or no longer needed (created when a shell/pentest \
         command was moved to the background). Use this AFTER check_job shows a job has been \
         running with no new output for a long time (e.g. a hung DNS AXFR / zone-transfer probe): \
         cancel it, then continue or re-run differently rather than waiting out the hard timeout. \
         Pass the job_id from a tool result whose status was \"backgrounded\"."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job_id of the background job to cancel (from a tool result with status \"backgrounded\")."
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let job_id = args
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: job_id"))?;

        let killed = crate::background_jobs::manager().kill(job_id);
        if killed {
            Ok(json!({
                "job_id": job_id,
                "killed": true,
                "message": format!(
                    "Requested cancellation of background job '{job_id}'. It will transition to \
                     'killed' shortly — continue without waiting (do NOT re-run the same command \
                     blindly; reconsider why it hung first)."
                ),
            }))
        } else {
            Ok(json!({
                "job_id": job_id,
                "killed": false,
                "message": format!(
                    "No background job with id '{job_id}' (it may have already finished and been cleared)."
                ),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ws() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn detects_dns_zone_transfer_probes() {
        assert!(is_dns_zone_transfer("dig AXFR +short pingan.com"));
        assert!(is_dns_zone_transfer("dig axfr example.com"));
        assert!(is_dns_zone_transfer(
            "dig @ns1.example.com example.com AXFR"
        ));
        assert!(is_dns_zone_transfer("host -l example.com ns1.example.com"));
    }

    #[test]
    fn non_zone_transfer_commands_are_not_flagged() {
        assert!(!is_dns_zone_transfer("dig A example.com"));
        assert!(!is_dns_zone_transfer("subfinder -d example.com"));
        assert!(!is_dns_zone_transfer("httpx -u https://example.com"));
        assert!(!is_dns_zone_transfer("echo hello"));
    }

    #[test]
    fn zone_transfer_hard_limit_is_capped_short() {
        // A hung AXFR must not be allowed to pin a stage for the 30-min default.
        let axfr = compute_hard_ms("dig AXFR +short pingan.com", 10_000);
        assert!(
            axfr <= DEFAULT_DNS_HARD_TIMEOUT_MS,
            "zone-transfer hard limit should be capped to <= {DEFAULT_DNS_HARD_TIMEOUT_MS}ms, got {axfr}"
        );
        // A normal command keeps the long default (and must outlast the caller timeout).
        let normal = compute_hard_ms("dig A example.com", 10_000);
        assert!(
            normal >= DEFAULT_HARD_TIMEOUT_MS,
            "normal command should keep the long hard limit, got {normal}"
        );
        assert!(
            axfr < normal,
            "zone-transfer hard limit ({axfr}) must be shorter than a normal one ({normal})"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_job_tool_cancels_a_running_job() {
        // Spawn a long-runner through the process-global manager the tool uses.
        let job_id =
            crate::background_jobs::manager().spawn("sleep 30", &ws(), Duration::from_secs(60));
        let res = KillJobTool
            .execute(json!({ "job_id": job_id }), &ws())
            .await
            .expect("kill_job execute ok");
        assert_eq!(
            res.get("killed"),
            Some(&json!(true)),
            "kill_job should report killed:true for a live job, got {res:?}"
        );
    }

    #[tokio::test]
    async fn kill_job_tool_reports_unknown_id() {
        let res = KillJobTool
            .execute(json!({ "job_id": "job_doesnotexist" }), &ws())
            .await
            .expect("kill_job execute ok");
        assert_eq!(res.get("killed"), Some(&json!(false)));
    }

    #[tokio::test]
    async fn kill_job_tool_requires_job_id() {
        let err = KillJobTool.execute(json!({}), &ws()).await;
        assert!(err.is_err(), "missing job_id must be an error");
    }
}
