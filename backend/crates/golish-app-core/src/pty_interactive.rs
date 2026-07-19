use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// Brief startup confirmation for AI-elected background jobs. Commands that
/// fail immediately due to bad flags / missing runtime should be returned inline
/// so the model can correct them instead of blindly continuing.
const DEFAULT_BACKGROUND_STARTUP_GRACE_MS: u64 = 800;
/// Background hard limit: runaway jobs are killed after this so nothing leaks.
const DEFAULT_HARD_TIMEOUT_MS: u64 = 1_800_000; // 30 min
/// Background hard limit for DNS zone-transfer (AXFR) probes. These hang
/// indefinitely against resolvers that silently drop TCP zone transfers, so the
/// 30-minute default would let a single hung probe pin a whole stage's closeout
/// reconciliation barrier. Capped aggressively so they fail fast instead.
const DEFAULT_DNS_HARD_TIMEOUT_MS: u64 = 15_000; // 15s
/// Explicit wait tool default. Unlike `submit_stage_deliverable`, this is a
/// visible step: the model/user see "waiting for background jobs" as its own
/// tool card, then receive the finished job tails before re-submitting.
const DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS: u64 = 300_000;
const MAX_WAIT_BACKGROUND_JOBS_TIMEOUT_MS: u64 = 900_000;
const DEFAULT_WAIT_BACKGROUND_JOBS_IDLE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_WAIT_BACKGROUND_JOBS_POLL_MS: u64 = 1_000;
const WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES: usize = 12 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellRunMode {
    AutoBackgroundAfterSoftTimeout,
    BackgroundAfterStartup,
    ForegroundOnly,
}

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

fn finished_job_value(command: &str, snap: crate::background_jobs::JobSnapshot) -> Value {
    let BoundedOutput {
        text: stdout,
        truncated: stdout_truncated,
        original_bytes: stdout_original_bytes,
    } = truncate_output(snap.stdout);
    let BoundedOutput {
        text: stderr,
        truncated: stderr_truncated,
        original_bytes: stderr_original_bytes,
    } = truncate_output(snap.stderr);
    json!({
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "stdout_original_bytes": stdout_original_bytes,
        "stderr_original_bytes": stderr_original_bytes,
        "command": command,
        "exit_code": snap.exit_code.unwrap_or(-1),
        "duration_ms": snap.duration_ms,
    })
}

fn timeout_job_value(
    command: &str,
    snap: crate::background_jobs::JobSnapshot,
    timeout_ms: u64,
) -> Value {
    let BoundedOutput {
        text: stdout,
        truncated: stdout_truncated,
        original_bytes: stdout_original_bytes,
    } = truncate_output(snap.stdout);
    let BoundedOutput {
        text: stderr,
        truncated: stderr_truncated,
        original_bytes: stderr_original_bytes,
    } = truncate_output(snap.stderr);
    json!({
        "status": "timeout",
        "error_kind": "COMMAND_TIMEOUT",
        "error": format!(
            "Command exceeded the {}s timeout and was killed before being moved to the background.",
            timeout_ms.div_ceil(1000)
        ),
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "stdout_original_bytes": stdout_original_bytes,
        "stderr_original_bytes": stderr_original_bytes,
        "command": command,
        "exit_code": snap.exit_code.unwrap_or(124),
        "duration_ms": snap.duration_ms,
    })
}

fn cancelled_job_value(command: &str, snap: crate::background_jobs::JobSnapshot) -> Value {
    let mut value = finished_job_value(command, snap);
    value["status"] = Value::String("cancelled".to_string());
    value["error_kind"] = Value::String("COMMAND_CANCELLED".to_string());
    value["error"] = Value::String(
        "Command was cancelled, killed, reaped, and drained before wrapper landing.".to_string(),
    );
    value
}

/// Run a shell command outside the visible terminal and return structured
/// output for the AI tool detail panel.
///
/// This deliberately avoids `PtyManager::write`, so AI-originated shell
/// commands do not create `CommandBlock`s in the user's terminal timeline.
///
/// The command is spawned through the [`crate::background_jobs`] manager: if it
/// finishes within the *soft* timeout (or, for AI-elected background runs, the
/// startup confirmation window) the full result is returned as before; otherwise
/// it keeps running in the background and a
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
    let mode = if background {
        ShellRunMode::BackgroundAfterStartup
    } else {
        ShellRunMode::AutoBackgroundAfterSoftTimeout
    };
    run_shell_command_detail_with_mode(command, workspace, timeout_ms, mode).await
}

/// Run a shell command in the current tool call only.
///
/// Unlike [`run_shell_command_detail`], timeout does not create a background
/// handle. The process is killed and the retained stdout/stderr are returned so
/// callers can decide whether to retry with narrower flags.
pub async fn run_shell_command_detail_foreground_only(
    command: &str,
    workspace: &Path,
    timeout_ms: u64,
) -> Result<Value> {
    run_shell_command_detail_with_mode(command, workspace, timeout_ms, ShellRunMode::ForegroundOnly)
        .await
}

async fn run_shell_command_detail_with_mode(
    command: &str,
    workspace: &Path,
    timeout_ms: u64,
    mode: ShellRunMode,
) -> Result<Value> {
    let soft_cap = env_ms("GOLISH_TOOL_SOFT_TIMEOUT_MS", DEFAULT_SOFT_TIMEOUT_MS);
    let soft_ms = timeout_ms.min(soft_cap).max(1);
    // The background hard limit normally outlasts the caller's timeout so the job
    // can continue past the point where it would previously have been killed —
    // except DNS zone-transfer probes, which are capped short (they hang forever
    // against resolvers that drop TCP AXFR). See [`compute_hard_ms`].
    let hard_ms = match mode {
        ShellRunMode::ForegroundOnly => timeout_ms.saturating_add(1_000).max(1),
        ShellRunMode::AutoBackgroundAfterSoftTimeout | ShellRunMode::BackgroundAfterStartup => {
            compute_hard_ms(command, timeout_ms)
        }
    };

    let started_at = std::time::Instant::now();
    // Attribute the job to the session whose agentic loop is currently running
    // (set via `golish_core::with_agent_session`), so the completion broadcast
    // can be routed back to that session. Capture the current tool context too
    // so live stdout/stderr chunks can update the existing tool-call detail UI.
    // `None` when not attributable.
    let tool_cancellation = golish_core::current_agent_tool_cancellation();
    let (session_id, tool_context) = match mode {
        // Foreground-only jobs are consumed by this tool call, so they should
        // not hold the stage-close background barrier open. Keep the tool
        // context so stdout/stderr and cancellation remain attached to the
        // exact durable wrapper call.
        ShellRunMode::ForegroundOnly => (None, golish_core::current_agent_tool_context()),
        ShellRunMode::AutoBackgroundAfterSoftTimeout | ShellRunMode::BackgroundAfterStartup => (
            golish_core::current_agent_session(),
            golish_core::current_agent_tool_context(),
        ),
    };
    let job_id = crate::background_jobs::manager()
        .try_spawn_for_session_and_tool(
            command,
            workspace,
            Duration::from_millis(hard_ms),
            session_id.clone(),
            tool_context,
        )
        .map_err(|error| anyhow::anyhow!("{}: {}", error.code(), error))?;

    tracing::info!(
        "[run_pty_cmd] spawn: command={}, mode={:?}, soft_ms={}, hard_ms={}, job_id={}, session={:?}",
        command,
        mode,
        soft_ms,
        hard_ms,
        job_id,
        session_id
    );

    // Cursor-style backgrounding still needs a tiny startup confirmation window:
    // if the child exits immediately with a usage/runtime error, return that
    // inline so the model can correct its command. Once the window expires and
    // the child is still running, hand back the job handle and let it continue.
    let inline_wait_ms = match mode {
        ShellRunMode::BackgroundAfterStartup => env_ms(
            "GOLISH_TOOL_BACKGROUND_STARTUP_GRACE_MS",
            DEFAULT_BACKGROUND_STARTUP_GRACE_MS,
        )
        .min(soft_ms),
        ShellRunMode::AutoBackgroundAfterSoftTimeout => soft_ms,
        ShellRunMode::ForegroundOnly => timeout_ms.max(1),
    };

    if inline_wait_ms > 0 {
        let inline_deadline = started_at + Duration::from_millis(inline_wait_ms);
        loop {
            if tool_cancellation
                .as_ref()
                .is_some_and(golish_core::AgentToolCancellation::is_cancelled)
            {
                let _ = crate::background_jobs::manager().kill(&job_id);
                let snap = crate::background_jobs::manager()
                    .wait_terminal(&job_id)
                    .await
                    .unwrap_or_else(|| crate::background_jobs::JobSnapshot {
                        command: command.to_string(),
                        status: crate::background_jobs::JobStatus::Killed,
                        exit_code: Some(124),
                        stdout: String::new(),
                        stderr: String::new(),
                        duration_ms: started_at.elapsed().as_millis() as u64,
                        finished: true,
                    });
                crate::background_jobs::manager().remove(&job_id);
                return Ok(cancelled_job_value(command, snap));
            }
            if let Some(snap) = crate::background_jobs::manager().snapshot(&job_id) {
                if snap.finished {
                    crate::background_jobs::manager().remove(&job_id);
                    return Ok(finished_job_value(command, snap));
                }
            }
            if std::time::Instant::now() >= inline_deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    if mode == ShellRunMode::ForegroundOnly {
        if let Some(snap) = crate::background_jobs::manager().snapshot(&job_id) {
            if snap.finished {
                crate::background_jobs::manager().remove(&job_id);
                return Ok(finished_job_value(command, snap));
            }
        }
        let _ = crate::background_jobs::manager().kill(&job_id);
        let snap = crate::background_jobs::manager()
            .wait_terminal(&job_id)
            .await
            .unwrap_or_else(|| crate::background_jobs::JobSnapshot {
                command: command.to_string(),
                status: crate::background_jobs::JobStatus::Killed,
                exit_code: Some(124),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: started_at.elapsed().as_millis() as u64,
                finished: true,
            });
        crate::background_jobs::manager().remove(&job_id);
        return Ok(timeout_job_value(command, snap, timeout_ms));
    }

    // Still running → hand back a background handle. Deliberately no `error` and
    // no non-zero `exit_code`, so the agentic loop treats this as a successful
    // tool result and the model reads `status: "backgrounded"`.
    let (partial_stdout, partial_stderr) = crate::background_jobs::manager()
        .snapshot(&job_id)
        .map(|s| (truncate_output(s.stdout), truncate_output(s.stderr)))
        .unwrap_or_default();
    let hint = if mode == ShellRunMode::BackgroundAfterStartup {
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
        "partial_stdout": partial_stdout.text,
        "partial_stderr": partial_stderr.text,
        "partial_stdout_truncated": partial_stdout.truncated,
        "partial_stderr_truncated": partial_stderr.truncated,
        "partial_stdout_original_bytes": partial_stdout.original_bytes,
        "partial_stderr_original_bytes": partial_stderr.original_bytes,
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

const MAX_TOOL_OUTPUT_BYTES: usize = 50_000;

#[derive(Debug, Default, PartialEq, Eq)]
struct BoundedOutput {
    text: String,
    truncated: bool,
    original_bytes: usize,
}

fn truncate_output(output: String) -> BoundedOutput {
    let original_bytes = output.len();
    if original_bytes <= MAX_TOOL_OUTPUT_BYTES {
        return BoundedOutput {
            text: output,
            truncated: false,
            original_bytes,
        };
    }

    // Byte-bounded, char-boundary-safe head truncation via the canonical
    // `golish_core::utils::truncate_str`, plus this caller's size note.
    BoundedOutput {
        text: format!(
            "{}...\n[Output truncated, {} bytes total]",
            golish_core::utils::truncate_str(&output, MAX_TOOL_OUTPUT_BYTES),
            original_bytes
        ),
        truncated: true,
        original_bytes,
    }
}

fn tail_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }
    let mut cut = output.len() - max_bytes;
    while cut < output.len() && !output.is_char_boundary(cut) {
        cut += 1;
    }
    format!("...[+{} earlier bytes]\n{}", cut, &output[cut..])
}

fn job_snapshot_value(job_id: &str, snap: crate::background_jobs::JobSnapshot) -> Value {
    json!({
        "job_id": job_id,
        "status": snap.status.as_str(),
        "running": !snap.finished,
        "job_exit_code": snap.exit_code,
        "command": snap.command,
        "stdout_tail": tail_output(&snap.stdout, WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES),
        "stderr_tail": tail_output(&snap.stderr, WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES),
        "duration_ms": snap.duration_ms,
    })
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
                    "description": "Run in the background (Cursor-style): after a brief startup check, return a job handle and keep the command running so you can proceed with other work. Immediate flag/runtime errors are returned inline so you can fix them. Use for long commands whose output you don't need right now; the result is auto-delivered when it finishes."
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
            Some(snap) => {
                let BoundedOutput {
                    text: stdout,
                    truncated: stdout_truncated,
                    original_bytes: stdout_original_bytes,
                } = truncate_output(snap.stdout);
                let BoundedOutput {
                    text: stderr,
                    truncated: stderr_truncated,
                    original_bytes: stderr_original_bytes,
                } = truncate_output(snap.stderr);
                Ok(json!({
                "job_id": job_id,
                "status": snap.status.as_str(),
                "running": !snap.finished,
                // The *command's* exit code is reported as `job_exit_code` (NOT a
                // top-level `exit_code`) so a failed background command does not
                // make this `check_job` call itself read as a tool failure.
                "job_exit_code": snap.exit_code,
                "command": snap.command,
                "stdout": stdout,
                "stderr": stderr,
                "stdout_truncated": stdout_truncated,
                "stderr_truncated": stderr_truncated,
                "stdout_original_bytes": stdout_original_bytes,
                "stderr_original_bytes": stderr_original_bytes,
                "duration_ms": snap.duration_ms,
                }))
            }
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

/// Tool that turns "wait for background scans" into an explicit, visible agent
/// step. It returns when all tracked jobs settle, when any tracked job completes,
/// or when the remaining jobs go idle/timeout, so the model can inspect landed
/// output incrementally instead of blocking an entire stage behind the slowest
/// batch.
pub struct WaitForBackgroundJobsTool;

#[async_trait::async_trait]
impl Tool for WaitForBackgroundJobsTool {
    fn name(&self) -> &'static str {
        "wait_for_background_jobs"
    }

    fn description(&self) -> &'static str {
        "Wait for background jobs started by this AI session. Return as soon as all tracked \
         jobs finish, any tracked job finishes while others are still running, or the remaining \
         jobs go idle/timeout. The result includes completed stdout/stderr tails plus still-running \
         jobs, so inspect landed output before deciding whether to wait again, narrow, kill, or submit."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum time to wait, in seconds. Default 300, max 900. The tool returns earlier when any tracked job completes, or when remaining jobs go idle."
                },
                "idle_timeout_secs": {
                    "type": "integer",
                    "description": "Return early if running jobs produce no new stdout/stderr for this many seconds. Default 60. If output is moving, the wait continues up to timeout_secs."
                },
                "poll_interval_ms": {
                    "type": "integer",
                    "description": "Polling interval while waiting. Default 1000ms."
                }
            }
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let Some(session_id) = golish_core::current_agent_session() else {
            return Ok(json!({
                "status": "no_session",
                "note": "No current AI session is attached, so background jobs cannot be attributed."
            }));
        };

        let timeout_ms = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|s| s.saturating_mul(1_000))
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS)
            .min(MAX_WAIT_BACKGROUND_JOBS_TIMEOUT_MS);
        let poll_ms = args
            .get("poll_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_POLL_MS)
            .clamp(100, 10_000);
        let idle_timeout_ms = args
            .get("idle_timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|s| s.saturating_mul(1_000))
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_IDLE_TIMEOUT_MS)
            .clamp(1_000, timeout_ms.max(1_000));

        let manager = crate::background_jobs::manager();
        let started_at = Instant::now();
        let deadline = started_at + Duration::from_millis(timeout_ms);
        let mut last_progress_at = started_at;
        let mut wait_reason = "timeout";
        let mut tracked_ids = BTreeSet::new();
        let mut running = manager.running_for_session(&session_id);
        for job in &running {
            tracked_ids.insert(job.job_id.clone());
        }
        let mut last_sizes = background_output_sizes(manager, &running);

        while !running.is_empty() && Instant::now() < deadline {
            let now = Instant::now();
            let remaining = deadline.duration_since(now).as_millis() as u64;
            let nap = poll_ms.min(remaining).max(1);
            tokio::time::sleep(Duration::from_millis(nap)).await;
            running = manager.running_for_session(&session_id);
            for job in &running {
                tracked_ids.insert(job.job_id.clone());
            }
            let any_tracked_finished = tracked_ids.iter().any(|job_id| {
                manager
                    .snapshot(job_id)
                    .map(|snap| snap.finished)
                    .unwrap_or(false)
            });
            if any_tracked_finished {
                wait_reason = "job_completed";
                break;
            }
            let sizes = background_output_sizes(manager, &running);
            if sizes != last_sizes {
                last_progress_at = Instant::now();
                last_sizes = sizes;
            } else if !running.is_empty()
                && Instant::now().duration_since(last_progress_at)
                    >= Duration::from_millis(idle_timeout_ms)
            {
                wait_reason = "idle_timeout";
                break;
            }
        }

        let mut completed_jobs = Vec::new();
        let mut running_jobs = Vec::new();
        for job_id in tracked_ids {
            if let Some(snap) = manager.snapshot(&job_id) {
                if snap.finished {
                    completed_jobs.push(job_snapshot_value(&job_id, snap));
                } else {
                    running_jobs.push(job_snapshot_value(&job_id, snap));
                }
            }
        }

        let waited_ms = started_at.elapsed().as_millis() as u64;
        if completed_jobs.is_empty() && running_jobs.is_empty() {
            return Ok(json!({
                "status": "no_running_jobs",
                "waited_ms": waited_ms,
                "completed_background_jobs": [],
                "running_background_jobs": [],
                "note": "No running background jobs were found for this session."
            }));
        }
        if running_jobs.is_empty() {
            return Ok(json!({
                "status": "settled",
                "waited_ms": waited_ms,
                "completed_background_jobs": completed_jobs,
                "running_background_jobs": [],
                "note": "All tracked background jobs have finished. Inspect the stdout_tail/stderr_tail above, then call submit_stage_deliverable."
            }));
        }
        Ok(json!({
            "status": "still_running",
            "waited_ms": waited_ms,
            "wait_reason": wait_reason,
            "completed_background_jobs": completed_jobs,
            "running_background_jobs": running_jobs,
            "recommended_action": if wait_reason == "idle_timeout" {
                "check_job_once_then_kill_or_narrow_if_stuck"
            } else if wait_reason == "job_completed" {
                "inspect_completed_output_then_wait_again_or_check_remaining"
            } else {
                "check_job_once_then_wait_again_if_progressing"
            },
            "note": "Some background jobs are still running. First inspect any completed job output above and let its evidence land. Then use check_job once on the remaining jobs if needed. If stdout/stderr is still moving and the batch is appropriate, wait again; if it has gone idle or the batch is too broad, kill_job it and close the affected cells with a concrete blocked/error/not_applicable note or a narrower batch."
        }))
    }
}

fn background_output_sizes(
    manager: &crate::background_jobs::BackgroundJobManager,
    running: &[crate::background_jobs::RunningJob],
) -> BTreeMap<String, (usize, usize)> {
    running
        .iter()
        .filter_map(|job| {
            manager
                .snapshot(&job.job_id)
                .map(|snap| (job.job_id.clone(), (snap.stdout.len(), snap.stderr.len())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ws() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn output_over_50kb_reports_truncation_and_lost_tail() {
        let output = format!("{}TAIL_SENTINEL", "x".repeat(MAX_TOOL_OUTPUT_BYTES + 1));

        let bounded = truncate_output(output.clone());

        assert!(bounded.truncated);
        assert_eq!(bounded.original_bytes, output.len());
        assert!(!bounded.text.contains("TAIL_SENTINEL"));
        assert!(bounded.text.contains("[Output truncated,"));

        let result = finished_job_value(
            "fake-scan",
            crate::background_jobs::JobSnapshot {
                command: "fake-scan".to_string(),
                status: crate::background_jobs::JobStatus::Done,
                exit_code: Some(0),
                stdout: output.clone(),
                stderr: String::new(),
                duration_ms: 1,
                finished: true,
            },
        );
        assert_eq!(result["stdout_truncated"], true);
        assert_eq!(result["stdout_original_bytes"], json!(output.len()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_large_process_output_is_drained_before_truncation_metadata() {
        const BYTES_PER_STREAM: usize = 128 * 1024;
        let command = format!(
            "python3 -c 'import sys; sys.stdout.write(\"A\"*{BYTES_PER_STREAM}); sys.stderr.write(\"B\"*{BYTES_PER_STREAM})'"
        );

        let result = run_shell_command_detail_foreground_only(&command, &ws(), 10_000)
            .await
            .unwrap();

        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout_truncated"], true);
        assert_eq!(result["stderr_truncated"], true);
        assert_eq!(result["stdout_original_bytes"], BYTES_PER_STREAM);
        assert_eq!(result["stderr_original_bytes"], BYTES_PER_STREAM);
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
    async fn background_command_that_fails_during_startup_returns_inline_error() {
        let res = run_shell_command_detail("printf bad-flag >&2; exit 2", &ws(), 10_000, true)
            .await
            .expect("background command should return a structured result");

        assert_eq!(res.get("exit_code"), Some(&json!(2)));
        assert!(
            res.get("stderr")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .contains("bad-flag"),
            "stderr should be returned inline so the model can correct it: {res:?}"
        );
        assert!(
            res.get("status").and_then(|v| v.as_str()) != Some("backgrounded"),
            "fast failures must not be hidden behind a background handle: {res:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_command_that_survives_startup_returns_job_handle() {
        let res = run_shell_command_detail("sleep 30", &ws(), 10_000, true)
            .await
            .expect("background command should return a structured result");

        assert_eq!(res.get("status"), Some(&json!("backgrounded")));
        let job_id = res
            .get("job_id")
            .and_then(|v| v.as_str())
            .expect("backgrounded result includes job_id");
        assert!(
            crate::background_jobs::manager().kill(job_id),
            "test long-runner should be cancellable"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_only_timeout_kills_instead_of_backgrounding() {
        let res = run_shell_command_detail_foreground_only("sleep 30", &ws(), 250)
            .await
            .expect("foreground-only command should return a structured result");

        assert_eq!(res.get("status"), Some(&json!("timeout")));
        assert_eq!(res.get("exit_code"), Some(&json!(124)));
        assert!(
            res.get("job_id").is_none(),
            "foreground-only timeout must not hand the model a background job: {res:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_only_cancellation_kills_reaps_and_returns_inline() {
        let cancellation = golish_core::AgentToolCancellation::default();
        let cancellation_for_tool = cancellation.clone();
        let execution = tokio::spawn(async move {
            golish_core::with_agent_tool_cancellation(Some(cancellation_for_tool), async {
                run_shell_command_detail_foreground_only("sleep 30", &ws(), 30_000).await
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancellation.cancel();

        let res = tokio::time::timeout(Duration::from_secs(5), execution)
            .await
            .expect("cancelled foreground child must be killed and awaited")
            .expect("foreground task should join")
            .expect("foreground tool should return a structured cancellation");

        assert_eq!(res.get("status"), Some(&json!("cancelled")));
        assert_eq!(res.get("error_kind"), Some(&json!("COMMAND_CANCELLED")));
        assert!(res.get("job_id").is_none());
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

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_for_background_jobs_returns_completed_output() {
        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        crate::background_jobs::manager().spawn_for_session(
            "sleep 0.1; printf wait-done",
            &ws(),
            Duration::from_secs(5),
            Some(session_id.clone()),
        );

        let res = golish_core::with_agent_session(Some(session_id), async {
            WaitForBackgroundJobsTool
                .execute(json!({ "timeout_secs": 2, "poll_interval_ms": 50 }), &ws())
                .await
        })
        .await
        .expect("wait_for_background_jobs execute ok");

        assert_eq!(res.get("status"), Some(&json!("settled")));
        let completed = res["completed_background_jobs"]
            .as_array()
            .expect("completed jobs array");
        assert_eq!(completed.len(), 1, "one job should settle: {res:?}");
        assert!(
            completed[0]["stdout_tail"]
                .as_str()
                .unwrap_or_default()
                .contains("wait-done"),
            "completed stdout tail should be returned: {res:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_for_background_jobs_returns_on_idle_timeout() {
        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        let job_id = crate::background_jobs::manager().spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some(session_id.clone()),
        );

        let start = Instant::now();
        let res = golish_core::with_agent_session(Some(session_id), async {
            WaitForBackgroundJobsTool
                .execute(
                    json!({
                        "timeout_secs": 5,
                        "idle_timeout_secs": 1,
                        "poll_interval_ms": 50
                    }),
                    &ws(),
                )
                .await
        })
        .await
        .expect("wait_for_background_jobs execute ok");

        assert_eq!(res.get("status"), Some(&json!("still_running")));
        assert_eq!(res.get("wait_reason"), Some(&json!("idle_timeout")));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "idle timeout should return before full timeout: {res:?}"
        );
        let _ = crate::background_jobs::manager().kill(&job_id);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_for_background_jobs_returns_when_any_job_completes() {
        let session_id = format!("test-session-{}", uuid::Uuid::new_v4());
        crate::background_jobs::manager().spawn_for_session(
            "sleep 0.1; printf fast-done",
            &ws(),
            Duration::from_secs(5),
            Some(session_id.clone()),
        );
        let slow_job_id = crate::background_jobs::manager().spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some(session_id.clone()),
        );

        let res = golish_core::with_agent_session(Some(session_id), async {
            WaitForBackgroundJobsTool
                .execute(
                    json!({
                        "timeout_secs": 5,
                        "idle_timeout_secs": 4,
                        "poll_interval_ms": 50
                    }),
                    &ws(),
                )
                .await
        })
        .await
        .expect("wait_for_background_jobs execute ok");

        assert_eq!(res.get("status"), Some(&json!("still_running")));
        assert_eq!(res.get("wait_reason"), Some(&json!("job_completed")));
        assert_eq!(
            res["completed_background_jobs"]
                .as_array()
                .expect("completed jobs")
                .len(),
            1
        );
        assert!(
            res["completed_background_jobs"][0]["stdout_tail"]
                .as_str()
                .unwrap_or_default()
                .contains("fast-done"),
            "completed output should be returned: {res:?}"
        );
        assert_eq!(
            res["running_background_jobs"]
                .as_array()
                .expect("running jobs")
                .len(),
            1
        );
        let _ = crate::background_jobs::manager().kill(&slow_job_id);
    }
}
