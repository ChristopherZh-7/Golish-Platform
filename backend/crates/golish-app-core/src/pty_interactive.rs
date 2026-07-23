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

/// Default and bounds for one initial command observation. This is a yield
/// window, not a process timeout: reaching it never changes child lifetime.
const DEFAULT_INITIAL_YIELD_MS: u64 = 10_000;
const MIN_INITIAL_YIELD_MS: u64 = 250;
const MAX_INITIAL_YIELD_MS: u64 = 30_000;
/// Maximum duration of one `check_job` read. Like Codex's empty stdin poll,
/// this bounds only the current read and never the managed process.
const MAX_JOB_READ_YIELD_MS: u64 = 300_000;
/// Brief startup confirmation for AI-elected background jobs. Commands that
/// fail immediately due to bad flags / missing runtime should be returned inline
/// so the model can correct them instead of blindly continuing.
const DEFAULT_BACKGROUND_STARTUP_GRACE_MS: u64 = 800;
/// Explicit recovery wait tool default. Normal closeout uses the manager's
/// event-driven reconciliation barrier; this remains available for manual or
/// exceptional diagnostics.
const DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS: u64 = 300_000;
const MAX_WAIT_BACKGROUND_JOBS_TIMEOUT_MS: u64 = 900_000;
const DEFAULT_WAIT_BACKGROUND_JOBS_IDLE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_WAIT_BACKGROUND_JOBS_POLL_MS: u64 = 1_000;
const WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES: usize = 12 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellRunMode {
    ManagedYield,
    StartupYield,
    SynchronousAuthority,
}

fn env_ms(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
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
/// finishes within the bounded initial yield (or the legacy short startup
/// yield) the full result is returned as before; otherwise the same managed
/// child keeps running and a
/// `{ status: "backgrounded", job_id }` handle is returned (success-shaped, so
/// the agentic loop does not treat it as a failure). The AI polls progress via
/// the `check_job` tool.
pub async fn run_shell_command_detail(
    command: &str,
    workspace: &Path,
    initial_yield_ms: u64,
    // Compatibility only: old callers can request the short startup yield.
    // This does not create a different kind of process or detach/respawn it.
    legacy_background: bool,
) -> Result<Value> {
    let mode = if legacy_background {
        ShellRunMode::StartupYield
    } else {
        ShellRunMode::ManagedYield
    };
    run_shell_command_detail_with_mode(command, workspace, initial_yield_ms, mode, None).await
}

/// Run a command with a server-owned typed completion reconciler. The inline
/// yield is bounded, but process lifetime is not: a still-running child returns
/// its session handle, and its typed reconciler
/// lands business state before completion is delivered to the generic bridge.
pub async fn run_shell_command_detail_managed(
    command: &str,
    workspace: &Path,
    initial_yield_ms: u64,
    reconciler: Arc<dyn crate::background_jobs::BackgroundJobReconciler>,
) -> Result<Value> {
    run_shell_command_detail_with_mode(
        command,
        workspace,
        initial_yield_ms,
        ShellRunMode::ManagedYield,
        Some(reconciler),
    )
    .await
}

/// Run a shell command in the current tool call only.
///
/// Unlike [`run_shell_command_detail`], this policy does not create a background
/// handle. Elapsed time never kills the child; only explicit tool/session
/// cancellation does. Callers use it only when detached completion would lose
/// a required synchronous authority or receipt.
pub async fn run_shell_command_detail_foreground_only(
    command: &str,
    workspace: &Path,
    legacy_wait_ms: u64,
) -> Result<Value> {
    run_shell_command_detail_with_mode(
        command,
        workspace,
        legacy_wait_ms,
        ShellRunMode::SynchronousAuthority,
        None,
    )
    .await
}

async fn run_shell_command_detail_with_mode(
    command: &str,
    workspace: &Path,
    requested_yield_ms: u64,
    mode: ShellRunMode,
    reconciler: Option<Arc<dyn crate::background_jobs::BackgroundJobReconciler>>,
) -> Result<Value> {
    let initial_yield_ms = requested_yield_ms.clamp(MIN_INITIAL_YIELD_MS, MAX_INITIAL_YIELD_MS);
    let started_at = std::time::Instant::now();
    // Attribute the job to the session whose agentic loop is currently running
    // (set via `golish_core::with_agent_session`), so the completion broadcast
    // can be routed back to that session. Capture the current tool context too
    // so live stdout/stderr chunks can update the existing tool-call detail UI.
    // `None` when not attributable.
    let tool_cancellation = golish_core::current_agent_tool_cancellation();
    let (desired_session_id, tool_context) = match mode {
        // Foreground-only jobs are consumed by this tool call, so they should
        // not hold the stage-close background barrier open. Keep the tool
        // context so stdout/stderr and cancellation remain attached to the
        // exact durable wrapper call.
        ShellRunMode::SynchronousAuthority => (None, golish_core::current_agent_tool_context()),
        ShellRunMode::ManagedYield | ShellRunMode::StartupYield => (
            golish_core::current_agent_session(),
            golish_core::current_agent_tool_context(),
        ),
    };
    let job_id = if let Some(reconciler) = reconciler {
        crate::background_jobs::manager().try_spawn_for_session_and_tool_with_reconciler(
            command,
            workspace,
            None,
            tool_context,
            reconciler,
        )
    } else {
        crate::background_jobs::manager().try_spawn_for_session_and_tool(
            command,
            workspace,
            Duration::ZERO,
            None,
            tool_context,
        )
    }
    .map_err(|error| anyhow::anyhow!("{}: {}", error.code(), error))?;

    tracing::info!(
        "[run_pty_cmd] spawn: command={}, mode={:?}, initial_yield_ms={}, automatic_kill=false, job_id={}, session={:?}",
        command,
        mode,
        initial_yield_ms,
        job_id,
        desired_session_id
    );

    // Codex-style backgrounding still needs a tiny startup confirmation window:
    // if the child exits immediately with a usage/runtime error, return that
    // inline so the model can correct its command. Once the window expires and
    // the child is still running, hand back the job handle and let it continue.
    let effective_yield_ms = match mode {
        ShellRunMode::StartupYield => env_ms(
            "GOLISH_TOOL_BACKGROUND_STARTUP_GRACE_MS",
            DEFAULT_BACKGROUND_STARTUP_GRACE_MS,
        )
        .min(initial_yield_ms),
        ShellRunMode::ManagedYield => initial_yield_ms,
        ShellRunMode::SynchronousAuthority => initial_yield_ms,
    };

    if effective_yield_ms > 0 {
        let yield_deadline = started_at + Duration::from_millis(effective_yield_ms);
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
                        exit_code: Some(130),
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
                    let typed_result = crate::background_jobs::manager()
                        .reconciliation(&job_id)
                        .map(|value| value.tool_result);
                    crate::background_jobs::manager().remove(&job_id);
                    return Ok(typed_result.unwrap_or_else(|| finished_job_value(command, snap)));
                }
            }
            if mode != ShellRunMode::SynchronousAuthority
                && std::time::Instant::now() >= yield_deadline
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    debug_assert_ne!(mode, ShellRunMode::SynchronousAuthority);

    if let Some(session_id) = desired_session_id {
        if !crate::background_jobs::manager().promote_to_session(&job_id, session_id) {
            if let Some(snap) = crate::background_jobs::manager().snapshot(&job_id) {
                let typed_result = crate::background_jobs::manager()
                    .reconciliation(&job_id)
                    .map(|value| value.tool_result);
                crate::background_jobs::manager().remove(&job_id);
                return Ok(typed_result.unwrap_or_else(|| finished_job_value(command, snap)));
            }
        }
    }

    // Still running → hand back a background handle. Deliberately no `error` and
    // no non-zero `exit_code`, so the agentic loop treats this as a successful
    // tool result and the model reads `status: "backgrounded"`.
    let (partial_stdout, partial_stderr) = crate::background_jobs::manager()
        .snapshot(&job_id)
        .map(|s| (truncate_output(s.stdout), truncate_output(s.stderr)))
        .unwrap_or_default();
    let activity = crate::background_jobs::manager().activity(&job_id);
    let hint = if mode == ShellRunMode::StartupYield {
        format!(
            "Managed process {job_id} is still running after the requested startup yield. This is \
             the same process, not a detached or respawned copy. Its result is auto-delivered on \
             exit; use `check_job` for another bounded read and `kill_job` only after deciding the \
             live process is stuck or no longer useful."
        )
    } else {
        format!(
            "Managed process {job_id} is still running, so this call returned control after its \
             bounded initial yield. The same process remains alive and was not killed or respawned. \
             Use `check_job` for another bounded read; use `kill_job` only after evaluating liveness, \
             workload, and output activity."
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
        "stdout_total_bytes": activity.map(|value| value.stdout_total_bytes).unwrap_or(0),
        "stderr_total_bytes": activity.map(|value| value.stderr_total_bytes).unwrap_or(0),
        "last_output_age_ms": activity.and_then(|value| value.last_output_age_ms),
        "initial_yield_ms": effective_yield_ms,
        "automatic_kill": false,
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

fn job_snapshot_value(
    manager: &crate::background_jobs::BackgroundJobManager,
    job_id: &str,
    snap: crate::background_jobs::JobSnapshot,
) -> Value {
    let activity = manager.activity(job_id);
    let termination_reason = manager
        .termination_reason(job_id)
        .map(|reason| reason.as_str());
    json!({
        "job_id": job_id,
        "status": snap.status.as_str(),
        "running": !snap.finished,
        "job_exit_code": snap.exit_code,
        "command": snap.command,
        "stdout_tail": tail_output(&snap.stdout, WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES),
        "stderr_tail": tail_output(&snap.stderr, WAIT_BACKGROUND_JOBS_OUTPUT_TAIL_BYTES),
        "duration_ms": snap.duration_ms,
        "stdout_total_bytes": activity.map(|value| value.stdout_total_bytes).unwrap_or(0),
        "stderr_total_bytes": activity.map(|value| value.stderr_total_bytes).unwrap_or(0),
        "last_output_age_ms": activity.and_then(|value| value.last_output_age_ms),
        "termination_reason": termination_reason,
        "automatic_kill": false,
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
                "yield_time_ms": {
                    "type": "integer",
                    "description": "How long this call should observe the managed process before returning its live job handle, in milliseconds. Default 10000, range 250..30000. This never kills the process."
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

        let configured_default = env_ms(
            "GOLISH_TOOL_INITIAL_YIELD_MS",
            env_ms(
                "GOLISH_TOOL_INLINE_WAIT_MS",
                env_ms("GOLISH_TOOL_SOFT_TIMEOUT_MS", DEFAULT_INITIAL_YIELD_MS),
            ),
        );
        let initial_yield_ms = args
            .get("yield-time_ms")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                args.get("timeout")
                    .and_then(|v| v.as_u64())
                    .map(|seconds| seconds.saturating_mul(1_000))
            })
            .unwrap_or(configured_default)
            .clamp(MIN_INITIAL_YIELD_MS, MAX_INITIAL_YIELD_MS);
        let legacy_background = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        run_shell_command_detail(command, workspace, initial_yield_ms, legacy_background).await
    }
}

/// Tool that lets the AI perform another bounded read from the same managed
/// shell/pentest process returned by [`run_shell_command_detail`].
pub struct CheckJobTool;

#[async_trait::async_trait]
impl Tool for CheckJobTool {
    fn name(&self) -> &'static str {
        "check_job"
    }

    fn description(&self) -> &'static str {
        "Read a managed process for a bounded yield without changing its lifetime. Returns running/terminal \
         status, elapsed time, cumulative stdout/stderr bytes, time since the last output, and retained \
         output. A quiet process is not automatically stuck; compare its workload and activity before \
         explicitly using kill_job. Managed jobs are never killed merely because elapsed time passed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job_id returned in a tool result with status \"backgrounded\"."
                },
                "yield-time_ms": {
                    "type": "integer",
                    "description": "How long to wait for new output or process exit before returning. Default 10000ms; 0 returns an immediate snapshot; max 300000ms. This never kills the process."
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

        let yield_ms = args
            .get("yield-time_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_INITIAL_YIELD_MS)
            .min(MAX_JOB_READ_YIELD_MS);
        let manager = crate::background_jobs::manager();
        let started_at = Instant::now();
        let initial_activity = manager.activity(job_id);
        let poll_reason = if manager.snapshot(job_id).is_none() {
            "missing"
        } else if manager
            .snapshot(job_id)
            .is_some_and(|snapshot| snapshot.finished)
        {
            "terminal"
        } else if yield_ms == 0 {
            "snapshot"
        } else {
            loop {
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                if elapsed_ms >= yield_ms {
                    break "yield_elapsed";
                }
                tokio::time::sleep(Duration::from_millis(
                    50_u64.min(yield_ms.saturating_sub(elapsed_ms).max(1)),
                ))
                .await;
                let Some(snapshot) = manager.snapshot(job_id) else {
                    break "missing";
                };
                if snapshot.finished {
                    break "terminal";
                }
                let activity = manager.activity(job_id);
                let output_changed = match (initial_activity, activity) {
                    (Some(before), Some(after)) => {
                        before.stdout_total_bytes != after.stdout_total_bytes
                            || before.stderr_total_bytes != after.stderr_total_bytes
                    }
                    (None, Some(_)) => true,
                    _ => false,
                };
                if output_changed {
                    break "output";
                }
            }
        };

        match manager.snapshot(job_id) {
            Some(snap) => {
                let activity = manager.activity(job_id);
                let termination_reason = manager
                    .termination_reason(job_id)
                    .map(|reason| reason.as_str());
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
                "stdout_total_bytes": activity.map(|value| value.stdout_total_bytes).unwrap_or(0),
                "stderr_total_bytes": activity.map(|value| value.stderr_total_bytes).unwrap_or(0),
                "last_output_age_ms": activity.and_then(|value| value.last_output_age_ms),
                "termination_reason": termination_reason,
                "automatic_kill": false,
                "read_yield_ms": yield_ms,
                "waited_ms": started_at.elapsed().as_millis() as u64,
                "poll_reason": poll_reason,
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
/// dropping TCP zone transfers). Closes the Codex-style loop: `check_job` shows
/// no progress → `kill_job` it → continue or re-run differently, instead of
/// relying on an elapsed-time watchdog.
pub struct KillJobTool;

#[async_trait::async_trait]
impl Tool for KillJobTool {
    fn name(&self) -> &'static str {
        "kill_job"
    }

    fn description(&self) -> &'static str {
        "Cancel a managed shell/pentest job that is stuck or no longer needed. Use this AFTER \
         check_job shows the same process has been \
         running with no new output for a long time (e.g. a hung DNS AXFR / zone-transfer probe): \
         cancel it, then continue or re-run differently. \
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

/// Explicit recovery tool for manually observing background scans. Normal stage
/// closeout waits on manager lifecycle events and should not call this tool.
pub struct WaitForBackgroundJobsTool;

#[async_trait::async_trait]
impl Tool for WaitForBackgroundJobsTool {
    fn name(&self) -> &'static str {
        "wait_for_background_jobs"
    }

    fn description(&self) -> &'static str {
        "Recovery-only: manually wait for background jobs started by this AI session. Normal \
         stage closeout is event-driven and must not call this in a loop. Return as soon as all tracked \
         jobs finish, any tracked job finishes while others are still running, or the current \
         aggregate read yield ends. The result includes completed stdout/stderr tails plus still-running \
         jobs, so inspect landed output before deciding whether to wait again, narrow, kill, or submit."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "yield-time_ms": {
                    "type": "integer",
                    "description": "Maximum duration of this aggregate read, in milliseconds. Default 300000, max 900000. Ending the read never stops a process."
                },
                "quiet_yield_ms": {
                    "type": "integer",
                    "description": "Return control after this much time with no new stdout/stderr. Default 60000ms. Quiet is only an observation and never implies kill or business completion."
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

        let yield_ms = args
            .get("yield-time_ms")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                args.get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .map(|seconds| seconds.saturating_mul(1_000))
            })
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_TIMEOUT_MS)
            .min(MAX_WAIT_BACKGROUND_JOBS_TIMEOUT_MS);
        let poll_ms = args
            .get("poll_interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_POLL_MS)
            .clamp(100, 10_000);
        let quiet_yield_ms = args
            .get("quiet_yield_ms")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                args.get("idle_timeout_secs")
                    .and_then(|v| v.as_u64())
                    .map(|seconds| seconds.saturating_mul(1_000))
            })
            .unwrap_or(DEFAULT_WAIT_BACKGROUND_JOBS_IDLE_TIMEOUT_MS)
            .clamp(1_000, yield_ms.max(1_000));

        let manager = crate::background_jobs::manager();
        let started_at = Instant::now();
        let deadline = started_at + Duration::from_millis(yield_ms);
        let mut last_progress_at = started_at;
        let mut wait_reason = "yield_elapsed";
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
                    >= Duration::from_millis(quiet_yield_ms)
            {
                wait_reason = "quiet";
                break;
            }
        }

        let mut completed_jobs = Vec::new();
        let mut running_jobs = Vec::new();
        for job_id in tracked_ids {
            if let Some(snap) = manager.snapshot(&job_id) {
                if snap.finished {
                    completed_jobs.push(job_snapshot_value(manager, &job_id, snap));
                } else {
                    running_jobs.push(job_snapshot_value(manager, &job_id, snap));
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
            "recommended_action": if wait_reason == "quiet" {
                "inspect_liveness_workload_and_activity_before_deciding"
            } else if wait_reason == "job_completed" {
                "inspect_completed_output_then_wait_again_or_check_remaining"
            } else {
                "check_job_once_then_wait_again_if_progressing"
            },
            "note": "Some managed processes are still running. Inspect completed output and the remaining jobs' liveness, workload, cumulative bytes, and last-output age. A quiet read is not proof of a hung process. Wait again when useful, or explicitly kill only after deciding the process is stuck, mis-scoped, or no longer needed; cancellation does not create terminal coverage truth."
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

    struct InlineTypedReconciler;

    #[async_trait::async_trait]
    impl crate::background_jobs::BackgroundJobReconciler for InlineTypedReconciler {
        async fn reconcile(
            &self,
            terminal: crate::background_jobs::ManagedJobTerminal,
        ) -> anyhow::Result<crate::background_jobs::BackgroundJobReconciliation> {
            Ok(crate::background_jobs::BackgroundJobReconciliation {
                tool_result: json!({
                    "typed": true,
                    "stdout": terminal.output.stdout,
                    "termination_reason": terminal.termination_reason.as_str(),
                }),
                note: None,
                evidence_ids: Vec::new(),
                skip_generic_persistence: true,
            })
        }
    }

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

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_inline_completion_returns_typed_reconciliation() {
        let result = run_shell_command_detail_managed(
            "printf managed-inline",
            &ws(),
            5_000,
            Arc::new(InlineTypedReconciler),
        )
        .await
        .expect("managed command should return its typed completion");

        assert_eq!(result["typed"], true);
        assert_eq!(result["stdout"], "managed-inline");
        assert_eq!(result["termination_reason"], "exited");
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
        assert!(
            res.get("initial_yield_ms")
                .and_then(|value| value.as_u64())
                .is_some(),
            "tool result needs the effective initial yield for diagnostics: {res:?}"
        );
        assert_eq!(res.get("automatic_kill"), Some(&json!(false)));
        assert!(res.get("hard_timeout_ms").is_none());
        let hint = res
            .get("hint")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(hint.contains("same process"));
        assert!(hint.contains("check_job"));
        assert!(!hint.contains("moved to the background"));
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
    async fn foreground_policy_does_not_kill_on_elapsed_wait() {
        let res = run_shell_command_detail_foreground_only("sleep 0.2; printf survived", &ws(), 40)
            .await
            .expect("foreground-only command should return a structured result");

        assert_eq!(res.get("exit_code"), Some(&json!(0)));
        assert!(res["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("survived"));
        assert!(
            res.get("job_id").is_none(),
            "foreground policy remains inline even though elapsed time cannot kill it: {res:?}"
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

    #[cfg(unix)]
    #[tokio::test]
    async fn check_job_reports_server_authored_termination_reason() {
        let job_id = crate::background_jobs::manager().spawn(
            "printf finished",
            &ws(),
            Duration::from_millis(1),
        );

        for _ in 0..100 {
            if crate::background_jobs::manager()
                .snapshot(&job_id)
                .is_some_and(|snapshot| snapshot.finished)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let res = CheckJobTool
            .execute(json!({ "job_id": job_id }), &ws())
            .await
            .expect("check_job execute ok");
        assert_eq!(res.get("status"), Some(&json!("done")));
        assert_eq!(res.get("termination_reason"), Some(&json!("exited")));
        assert_eq!(res.get("automatic_kill"), Some(&json!(false)));
        assert_eq!(res.get("poll_reason"), Some(&json!("terminal")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn check_job_yield_returns_on_new_output_without_killing_process() {
        let job_id = crate::background_jobs::manager().spawn(
            "sleep 0.05; printf progress; sleep 30",
            &ws(),
            Duration::ZERO,
        );

        let res = CheckJobTool
            .execute(json!({ "job_id": job_id, "yield-time_ms": 2_000 }), &ws())
            .await
            .expect("check_job output-sensitive read should succeed");

        assert_eq!(res.get("poll_reason"), Some(&json!("output")));
        assert_eq!(res.get("running"), Some(&json!(true)));
        assert!(res["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("progress"));
        assert_eq!(res.get("automatic_kill"), Some(&json!(false)));
        assert!(
            crate::background_jobs::manager()
                .snapshot(res["job_id"].as_str().expect("job id"))
                .is_some_and(|snapshot| !snapshot.finished),
            "a completed read yield must leave the same managed process alive"
        );
        let _ = crate::background_jobs::manager()
            .kill(res["job_id"].as_str().expect("job id remains available"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn check_job_elapsed_yield_returns_same_live_handle() {
        let job_id = crate::background_jobs::manager().spawn("sleep 30", &ws(), Duration::ZERO);

        let res = CheckJobTool
            .execute(json!({ "job_id": job_id, "yield-time_ms": 50 }), &ws())
            .await
            .expect("check_job elapsed yield should succeed");

        assert_eq!(res.get("poll_reason"), Some(&json!("yield_elapsed")));
        assert_eq!(res.get("running"), Some(&json!(true)));
        assert_eq!(res.get("read_yield_ms"), Some(&json!(50)));
        assert_eq!(res.get("job_id"), Some(&json!(job_id.clone())));
        assert!(
            crate::background_jobs::manager()
                .snapshot(&job_id)
                .is_some_and(|snapshot| !snapshot.finished),
            "read yield expiration must not kill or replace the managed process"
        );
        let _ = crate::background_jobs::manager().kill(&job_id);
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
    async fn wait_for_background_jobs_returns_on_quiet_observation() {
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
        assert_eq!(res.get("wait_reason"), Some(&json!("quiet")));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "quiet observation should return before the full read yield: {res:?}"
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
