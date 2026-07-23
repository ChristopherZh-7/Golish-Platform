//! Background job manager for long-running AI shell/pentest commands.
//!
//! Replaces the old "timeout → kill the process → report a timeout error"
//! behaviour with a Codex-style "bounded yield → return the same live process
//! handle" model:
//!
//! - A command is always spawned through [`BackgroundJobManager::spawn`], which
//!   captures stdout/stderr incrementally into a capped buffer and **does not**
//!   `kill_on_drop` — the child outlives the awaiting future.
//! - The caller (`run_shell_command_detail`) waits through a bounded initial
//!   yield; if the command finishes in time it returns the full result as before,
//!   else it returns the same managed handle (`job_id`) and the command keeps
//!   running. `backgrounded` is only the compatibility status label.
//! - Runtime listeners reconcile terminal output and wake stage closeout only
//!   after evidence, structured outcomes, UI state, and the agent note land.
//! - Process lifetime is not tied to an elapsed-time watchdog. The agent or
//!   operator observes activity and explicitly cancels a job when appropriate;
//!   session shutdown remains an authoritative cancellation boundary.
//!
//! Design: `docs/design/2026-07-23-codex-same-session-process-yield.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, Notify};

use crate::pty_interactive::shell_command;

/// Cap each stream's retained buffer; keep the *tail* once exceeded so the most
/// recent output (usually the interesting part) survives.
const MAX_JOB_OUTPUT_BYTES: usize = 512 * 1024;
/// Full-output spool safety cap per stream. The child keeps running when the
/// cap is reached, but `complete=false` prevents evidence producers from
/// treating the partial archive as exhaustive output.
const MAX_JOB_SPOOL_BYTES: usize = 32 * 1024 * 1024;
/// Soft-delete finished jobs once the map grows past this, so a long session
/// doesn't accumulate unbounded completed-job state.
const MAX_RETAINED_JOBS: usize = 128;
/// Cap the stdout/stderr tail carried on a [`JobCompletion`] broadcast. The
/// retained job buffer can be up to 512KB; a completion *event* fed back to the
/// model / frontend should stay small.
const COMPLETION_TAIL_BYTES: usize = 8 * 1024;
/// Capacity of the completion broadcast channel. Completions are consumed by
/// per-session listeners; a slow listener that lags simply drops older
/// notifications (the job result is still pollable via `snapshot`).
const COMPLETION_CHANNEL_CAP: usize = 256;
/// Capacity of the live stdout/stderr broadcast channel. These chunks are best
/// effort UI telemetry; the retained job buffers remain the durable source.
const OUTPUT_CHANNEL_CAP: usize = 1024;
/// A direct child may exit while a grandchild still holds an inherited output
/// pipe open. Do not wait forever: bounded drain failure is terminal and must
/// never be exposed as a clean exit with incomplete output.
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub const ATTACK_VERIFIER_FOREGROUND_REQUIRED: &str = "ATTACK_VERIFIER_FOREGROUND_REQUIRED";

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum BackgroundJobSpawnError {
    #[error("Candidate verifier actions must execute in the foreground")]
    CandidateVerifierForegroundRequired,
}

impl BackgroundJobSpawnError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CandidateVerifierForegroundRequired => ATTACK_VERIFIER_FOREGROUND_REQUIRED,
        }
    }
}

/// High-level lifecycle state of a background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Killed,
}

/// Server-authored reason a managed child reached terminal state. There is no
/// elapsed-time/deadline variant by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobTerminationReason {
    Exited,
    OperatorCancelled,
    SessionCancelled,
    SpawnFailed,
    WaitFailed,
    OutputIncomplete,
}

impl JobTerminationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::OperatorCancelled => "operator_cancelled",
            Self::SessionCancelled => "session_cancelled",
            Self::SpawnFailed => "spawn_failed",
            Self::WaitFailed => "wait_failed",
            Self::OutputIncomplete => "output_incomplete",
        }
    }
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Killed => "killed",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, JobStatus::Running)
    }
}

struct JobState {
    command: String,
    /// Agent session that started the job (for routing the completion back to
    /// the right session), or `None` when not attributable.
    session_id: Option<String>,
    /// Agent tool call that started the job, when this job came from the
    /// agentic loop. Used to stream live chunks into the existing tool panel.
    tool_context: Option<golish_core::AgentToolContext>,
    status: JobStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_total_bytes: u64,
    stderr_total_bytes: u64,
    last_output_at: Option<Instant>,
    stdout_spool_path: Option<PathBuf>,
    stderr_spool_path: Option<PathBuf>,
    stdout_spool_truncated: bool,
    stderr_spool_truncated: bool,
    started_at: Instant,
    finished_at: Option<Instant>,
    termination_reason: Option<JobTerminationReason>,
    kill: Arc<Notify>,
    /// Terminal is not equivalent to fully consumed. Keep the job outstanding
    /// until the bridge listener has persisted every completion side effect.
    reconciled: bool,
    /// Shared by the original broadcast and any replay generated after a
    /// lagged listener so completion side effects remain exactly-once.
    processing_claim: Arc<AtomicBool>,
    reconciler: Option<Arc<dyn BackgroundJobReconciler>>,
    reconciliation: Option<BackgroundJobReconciliation>,
}

/// Immutable view of a job's current state, safe to hand back to callers.
#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub command: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub finished: bool,
}

/// Monotonic activity counters for a managed process. These let the agent
/// distinguish a live, progressing job from one that has been quiet without
/// conflating either state with an automatic timeout decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobActivity {
    pub stdout_total_bytes: u64,
    pub stderr_total_bytes: u64,
    pub last_output_age_ms: Option<u64>,
}

/// Full server-owned output for a terminal managed process. UI/model surfaces
/// should continue using bounded tails; typed reconcilers use this archive for
/// exact parsing and hashing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOutputArchive {
    pub stdout: String,
    pub stderr: String,
    pub stdout_total_bytes: u64,
    pub stderr_total_bytes: u64,
    pub complete: bool,
}

/// Immutable terminal input passed to a server-installed typed reconciler.
#[derive(Debug, Clone)]
pub struct ManagedJobTerminal {
    pub job_id: String,
    pub command: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub termination_reason: JobTerminationReason,
    pub duration_ms: u64,
    pub output: JobOutputArchive,
}

/// Business completion produced before the generic background listener runs.
#[derive(Debug, Clone)]
pub struct BackgroundJobReconciliation {
    pub tool_result: serde_json::Value,
    pub note: Option<String>,
    pub evidence_ids: Vec<i64>,
    pub skip_generic_persistence: bool,
}

#[async_trait::async_trait]
pub trait BackgroundJobReconciler: Send + Sync {
    async fn reconcile(
        &self,
        terminal: ManagedJobTerminal,
    ) -> anyhow::Result<BackgroundJobReconciliation>;
}

/// Lightweight view of a still-running job attributed to a session. Used by the
/// closeout reconciliation barrier (`submit_stage_deliverable`) to tell the
/// agent which background scans are still in flight before it concludes a stage.
#[derive(Debug, Clone)]
pub struct RunningJob {
    pub job_id: String,
    pub command: String,
    pub elapsed_ms: u64,
    pub stdout_total_bytes: u64,
    pub stderr_total_bytes: u64,
    pub last_output_age_ms: Option<u64>,
}

/// Broadcast payload emitted when a background job reaches a terminal state.
///
/// Consumed by per-session listeners (wired in `golish-agent-app`) which turn
/// it into a `ToolBackgroundCompleted` AI event + a note fed back to the agent.
#[derive(Debug, Clone)]
pub struct JobCompletion {
    pub job_id: String,
    /// Session the job was attributed to at spawn, if any.
    pub session_id: Option<String>,
    /// Harness organization active when the launching tool call started, if any.
    pub organization_id: Option<uuid::Uuid>,
    pub command: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    /// Size-capped tail of stdout (see [`COMPLETION_TAIL_BYTES`]).
    pub stdout_tail: String,
    /// Size-capped tail of stderr (see [`COMPLETION_TAIL_BYTES`]).
    pub stderr_tail: String,
    pub duration_ms: u64,
    /// All broadcast clones share this claim. Listener-generation handoff may
    /// intentionally overlap subscriptions so no completion falls into a gap;
    /// exactly one generation is allowed to perform evidence/DB side effects.
    #[doc(hidden)]
    pub processing_claim: Arc<AtomicBool>,
}

impl JobCompletion {
    pub fn try_claim_processing(&self) -> bool {
        self.processing_claim
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

/// Broadcast payload emitted as stdout/stderr bytes arrive for an attributed
/// background job.
#[derive(Debug, Clone)]
pub struct JobOutputChunk {
    pub job_id: String,
    /// Session the job was attributed to at spawn, if any.
    pub session_id: Option<String>,
    pub request_id: String,
    pub tool_name: String,
    pub source: golish_core::events::ToolSource,
    pub stream: &'static str,
    pub chunk: String,
    #[doc(hidden)]
    pub processing_claim: Arc<AtomicBool>,
}

impl JobOutputChunk {
    pub fn try_claim_processing(&self) -> bool {
        self.processing_claim
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

/// Keep the trailing `max` bytes of `s` on a char boundary (the *end* of a
/// command's output is usually the interesting part for a completion notice).
fn tail_capped(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("…[+{} earlier bytes]\n{}", cut, &s[cut..])
}

/// Process-wide registry of background jobs.
pub struct BackgroundJobManager {
    jobs: Mutex<HashMap<String, Arc<Mutex<JobState>>>>,
    /// Fan-out of job-completion notifications to per-session listeners.
    completions: broadcast::Sender<JobCompletion>,
    /// Fan-out of live stdout/stderr chunks to per-session UI listeners.
    outputs: broadcast::Sender<JobOutputChunk>,
    /// Event-driven closeout barrier. Terminal publication and reconciliation
    /// acknowledgements both notify waiters; no model-authored polling loop is
    /// needed on the healthy path.
    state_changed: Arc<Notify>,
}

impl Default for BackgroundJobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Append `chunk` to `buf`, trimming from the front (on a char boundary) so the
/// retained size never exceeds `MAX_JOB_OUTPUT_BYTES`.
fn append_capped(buf: &mut String, chunk: &str) {
    buf.push_str(chunk);
    if buf.len() <= MAX_JOB_OUTPUT_BYTES {
        return;
    }
    let mut cut = buf.len() - MAX_JOB_OUTPUT_BYTES;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    *buf = buf[cut..].to_string();
}

fn terminal_completion(job_id: String, state: &JobState) -> JobCompletion {
    let end = state.finished_at.unwrap_or_else(Instant::now);
    JobCompletion {
        job_id,
        session_id: state.session_id.clone(),
        organization_id: state
            .tool_context
            .as_ref()
            .and_then(|context| context.organization_id),
        command: state.command.clone(),
        status: state.status,
        exit_code: state.exit_code,
        stdout_tail: tail_capped(&state.stdout, COMPLETION_TAIL_BYTES),
        stderr_tail: tail_capped(&state.stderr, COMPLETION_TAIL_BYTES),
        duration_ms: end.duration_since(state.started_at).as_millis() as u64,
        processing_claim: state.processing_claim.clone(),
    }
}

fn typed_reconciliation_complete(state: &JobState) -> bool {
    state.reconciler.is_none() || state.reconciliation.is_some()
}

fn remove_spool_files(state: &JobState) {
    for path in [
        state.stdout_spool_path.as_ref(),
        state.stderr_spool_path.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let _ = std::fs::remove_file(path);
    }
}

async fn pump<R>(
    mut reader: R,
    mut spool: Option<tokio::fs::File>,
    state: Arc<Mutex<JobState>>,
    outputs: broadcast::Sender<JobOutputChunk>,
    job_id: String,
    is_stderr: bool,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = [0u8; 8192];
    let mut spooled_bytes = 0usize;
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => {
                if let Some(file) = spool.as_mut() {
                    file.flush().await?;
                }
                return Ok(());
            }
            Err(error) => return Err(error),
            Ok(n) => {
                if let Some(file) = spool.as_mut() {
                    let remaining = MAX_JOB_SPOOL_BYTES.saturating_sub(spooled_bytes);
                    let write_len = remaining.min(n);
                    if write_len > 0 {
                        file.write_all(&buf[..write_len]).await?;
                        spooled_bytes = spooled_bytes.saturating_add(write_len);
                    }
                    if write_len < n {
                        let mut s = state.lock();
                        if is_stderr {
                            s.stderr_spool_truncated = true;
                        } else {
                            s.stdout_spool_truncated = true;
                        }
                    }
                }
                let chunk = String::from_utf8_lossy(&buf[..n]);
                let stream = if is_stderr { "stderr" } else { "stdout" };
                let output_event = {
                    let mut s = state.lock();
                    if is_stderr {
                        append_capped(&mut s.stderr, &chunk);
                        s.stderr_total_bytes = s.stderr_total_bytes.saturating_add(n as u64);
                    } else {
                        append_capped(&mut s.stdout, &chunk);
                        s.stdout_total_bytes = s.stdout_total_bytes.saturating_add(n as u64);
                    }
                    s.last_output_at = Some(Instant::now());
                    s.tool_context.as_ref().map(|ctx| JobOutputChunk {
                        job_id: job_id.clone(),
                        session_id: s.session_id.clone(),
                        request_id: ctx.request_id.clone(),
                        tool_name: ctx.tool_name.clone(),
                        source: ctx.source.clone(),
                        stream,
                        chunk: chunk.to_string(),
                        processing_claim: Arc::new(AtomicBool::new(false)),
                    })
                };
                if let Some(event) = output_event {
                    let _ = outputs.send(event);
                }
            }
        }
    }
}

async fn drain_output_pump(
    stream: &'static str,
    handle: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
) -> Option<String> {
    let Some(mut handle) = handle else {
        return Some(format!("{stream} output pipe was unavailable"));
    };
    match tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, &mut handle).await {
        Ok(Ok(Ok(()))) => None,
        Ok(Ok(Err(error))) => Some(format!("{stream} output read failed: {error}")),
        Ok(Err(error)) => Some(format!("{stream} output pump failed: {error}")),
        Err(_) => {
            handle.abort();
            let _ = handle.await;
            Some(format!(
                "{stream} output drain exceeded {}ms",
                OUTPUT_DRAIN_TIMEOUT.as_millis()
            ))
        }
    }
}

async fn drain_output_pumps(
    stdout: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    stderr: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
) -> Vec<String> {
    let (stdout_error, stderr_error) = tokio::join!(
        drain_output_pump("stdout", stdout),
        drain_output_pump("stderr", stderr)
    );
    stdout_error.into_iter().chain(stderr_error).collect()
}

async fn output_archive_for_state(state: &Arc<Mutex<JobState>>) -> JobOutputArchive {
    let (
        stdout_path,
        stderr_path,
        stdout_tail,
        stderr_tail,
        stdout_total_bytes,
        stderr_total_bytes,
        stdout_spool_truncated,
        stderr_spool_truncated,
    ) = {
        let s = state.lock();
        (
            s.stdout_spool_path.clone(),
            s.stderr_spool_path.clone(),
            s.stdout.clone(),
            s.stderr.clone(),
            s.stdout_total_bytes,
            s.stderr_total_bytes,
            s.stdout_spool_truncated,
            s.stderr_spool_truncated,
        )
    };
    let (stdout, stdout_read_complete) = match stdout_path {
        Some(path) => match tokio::fs::read_to_string(path).await {
            Ok(output) => (output, true),
            Err(_) => (stdout_tail.clone(), false),
        },
        None => (stdout_tail, false),
    };
    let (stderr, stderr_read_complete) = match stderr_path {
        Some(path) => match tokio::fs::read_to_string(path).await {
            Ok(output) => (output, true),
            Err(_) => (stderr_tail.clone(), false),
        },
        None => (stderr_tail, false),
    };
    let complete = stdout_read_complete
        && stderr_read_complete
        && !stdout_spool_truncated
        && !stderr_spool_truncated
        && stdout.len() as u64 == stdout_total_bytes
        && stderr.len() as u64 == stderr_total_bytes;
    JobOutputArchive {
        stdout,
        stderr,
        stdout_total_bytes,
        stderr_total_bytes,
        complete,
    }
}

async fn managed_terminal_for_state(
    job_id: String,
    state: &Arc<Mutex<JobState>>,
) -> ManagedJobTerminal {
    let output = output_archive_for_state(state).await;
    let s = state.lock();
    let end = s.finished_at.unwrap_or_else(Instant::now);
    ManagedJobTerminal {
        job_id,
        command: s.command.clone(),
        status: s.status,
        exit_code: s.exit_code,
        termination_reason: s
            .termination_reason
            .unwrap_or(JobTerminationReason::WaitFailed),
        duration_ms: end.duration_since(s.started_at).as_millis() as u64,
        output,
    }
}

impl BackgroundJobManager {
    pub fn new() -> Self {
        let (completions, _) = broadcast::channel(COMPLETION_CHANNEL_CAP);
        let (outputs, _) = broadcast::channel(OUTPUT_CHANNEL_CAP);
        Self {
            jobs: Mutex::new(HashMap::new()),
            completions,
            outputs,
            state_changed: Arc::new(Notify::new()),
        }
    }

    /// Subscribe to job-completion notifications. Each terminal job (done /
    /// failed / killed, via the background reaper) publishes exactly one
    /// [`JobCompletion`]. Listeners filter by `session_id`.
    pub fn subscribe_completions(&self) -> broadcast::Receiver<JobCompletion> {
        self.completions.subscribe()
    }

    /// Subscribe to live stdout/stderr chunks from attributed background jobs.
    /// Chunks are best-effort UI telemetry; callers can still recover the
    /// complete retained tail via [`Self::snapshot`].
    pub fn subscribe_output_chunks(&self) -> broadcast::Receiver<JobOutputChunk> {
        self.outputs.subscribe()
    }

    /// Spawn `command` in the background, unattributed. See
    /// [`Self::spawn_for_session`].
    pub fn spawn(&self, command: &str, workspace: &Path, legacy_limit: Duration) -> String {
        self.spawn_for_session(command, workspace, legacy_limit, None)
    }

    /// Spawn `command` in the background, attributing it to `session_id` so the
    /// completion broadcast can be routed back to the right session. Returns
    /// immediately with a `job_id`. The child is reaped by an internal task;
    /// output streams into a capped buffer; the legacy duration argument is
    /// retained for source compatibility but does not control process lifetime;
    /// a [`JobCompletion`] is broadcast when the child exits or is explicitly
    /// cancelled.
    pub fn spawn_for_session(
        &self,
        command: &str,
        workspace: &Path,
        legacy_limit: Duration,
        session_id: Option<String>,
    ) -> String {
        self.spawn_for_session_and_tool(command, workspace, legacy_limit, session_id, None)
    }

    /// Spawn `command` in the background, additionally attributing live output
    /// to an agent tool call when `tool_context` is provided.
    pub fn spawn_for_session_and_tool(
        &self,
        command: &str,
        workspace: &Path,
        legacy_limit: Duration,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
    ) -> String {
        self.spawn_for_session_and_tool_unchecked(
            command,
            workspace,
            legacy_limit,
            session_id,
            tool_context,
            None,
        )
    }

    /// Attributed production spawn path. Candidate verifier contexts are
    /// durably scheduled and must never escape into the in-memory background
    /// process map.
    pub fn try_spawn_for_session_and_tool(
        &self,
        command: &str,
        workspace: &Path,
        legacy_limit: Duration,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
    ) -> Result<String, BackgroundJobSpawnError> {
        if tool_context
            .as_ref()
            .and_then(|context| context.candidate_attempt.as_ref())
            .is_some()
        {
            return Err(BackgroundJobSpawnError::CandidateVerifierForegroundRequired);
        }
        Ok(self.spawn_for_session_and_tool_unchecked(
            command,
            workspace,
            legacy_limit,
            session_id,
            tool_context,
            None,
        ))
    }

    /// Server-only managed spawn with a typed completion reconciler. The
    /// reconciler is installed before process launch and therefore cannot race
    /// a fast child exit.
    pub fn spawn_for_session_and_tool_with_reconciler(
        &self,
        command: &str,
        workspace: &Path,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
        reconciler: Arc<dyn BackgroundJobReconciler>,
    ) -> String {
        self.spawn_for_session_and_tool_unchecked(
            command,
            workspace,
            Duration::ZERO,
            session_id,
            tool_context,
            Some(reconciler),
        )
    }

    pub fn try_spawn_for_session_and_tool_with_reconciler(
        &self,
        command: &str,
        workspace: &Path,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
        reconciler: Arc<dyn BackgroundJobReconciler>,
    ) -> Result<String, BackgroundJobSpawnError> {
        if tool_context
            .as_ref()
            .and_then(|context| context.candidate_attempt.as_ref())
            .is_some()
        {
            return Err(BackgroundJobSpawnError::CandidateVerifierForegroundRequired);
        }
        Ok(self.spawn_for_session_and_tool_unchecked(
            command,
            workspace,
            Duration::ZERO,
            session_id,
            tool_context,
            Some(reconciler),
        ))
    }

    fn spawn_for_session_and_tool_unchecked(
        &self,
        command: &str,
        workspace: &Path,
        _legacy_limit: Duration,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
        reconciler: Option<Arc<dyn BackgroundJobReconciler>>,
    ) -> String {
        self.prune();

        let id = format!("job_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let spool_dir = workspace.join(".golish").join("background-jobs");
        let spool_ready = std::fs::create_dir_all(&spool_dir).is_ok();
        let stdout_spool_path = spool_ready.then(|| spool_dir.join(format!("{id}.stdout")));
        let stderr_spool_path = spool_ready.then(|| spool_dir.join(format!("{id}.stderr")));
        let stdout_spool = stdout_spool_path
            .as_ref()
            .and_then(|path| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .ok()
            })
            .map(tokio::fs::File::from_std);
        let stderr_spool = stderr_spool_path
            .as_ref()
            .and_then(|path| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .ok()
            })
            .map(tokio::fs::File::from_std);
        let kill = Arc::new(Notify::new());
        // Unattributed jobs have no session bridge consumer, so terminal state
        // itself is fully reconciled for retention purposes.
        let reconciled = session_id.is_none();
        let state = Arc::new(Mutex::new(JobState {
            command: command.to_string(),
            session_id,
            tool_context,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_total_bytes: 0,
            stderr_total_bytes: 0,
            last_output_at: None,
            stdout_spool_path: stdout_spool.as_ref().and(stdout_spool_path),
            stderr_spool_path: stderr_spool.as_ref().and(stderr_spool_path),
            stdout_spool_truncated: stdout_spool.is_none(),
            stderr_spool_truncated: stderr_spool.is_none(),
            started_at: Instant::now(),
            finished_at: None,
            termination_reason: None,
            kill: kill.clone(),
            reconciled,
            processing_claim: Arc::new(AtomicBool::new(false)),
            reconciler,
            reconciliation: None,
        }));
        self.jobs.lock().insert(id.clone(), state.clone());

        let mut cmd = shell_command(command);
        cmd.current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        golish_platform::process::configure_process_group(&mut cmd);
        // Intentionally NOT kill_on_drop: the whole point is to keep running.

        match cmd.spawn() {
            Ok(mut child) => {
                let stdout_pump = child.stdout.take().map(|out| {
                    tokio::spawn(pump(
                        out,
                        stdout_spool,
                        state.clone(),
                        self.outputs.clone(),
                        id.clone(),
                        false,
                    ))
                });
                let stderr_pump = child.stderr.take().map(|err| {
                    tokio::spawn(pump(
                        err,
                        stderr_spool,
                        state.clone(),
                        self.outputs.clone(),
                        id.clone(),
                        true,
                    ))
                });

                let st = state.clone();
                let completions = self.completions.clone();
                let state_changed = self.state_changed.clone();
                let job_id = id.clone();
                tokio::spawn(async move {
                    enum Outcome {
                        Exited(std::io::Result<std::process::ExitStatus>),
                        Stopped,
                    }
                    let outcome = tokio::select! {
                        r = child.wait() => Outcome::Exited(r),
                        _ = kill.notified() => Outcome::Stopped,
                    };
                    let (mut status, mut exit_code, mut diagnostics, mut termination_reason) =
                        match outcome {
                            Outcome::Exited(Ok(status)) => (
                                if status.success() {
                                    JobStatus::Done
                                } else {
                                    JobStatus::Failed
                                },
                                status.code(),
                                Vec::new(),
                                JobTerminationReason::Exited,
                            ),
                            Outcome::Exited(Err(error)) => {
                                golish_platform::process::kill_process_group(&mut child).await;
                                let reap_error = child.wait().await.err();
                                (
                                    JobStatus::Failed,
                                    Some(1),
                                    std::iter::once(format!("child wait failed: {error}"))
                                        .chain(
                                            reap_error
                                                .map(|error| format!("child reap failed: {error}")),
                                        )
                                        .collect(),
                                    JobTerminationReason::WaitFailed,
                                )
                            }
                            Outcome::Stopped => {
                                golish_platform::process::kill_process_group(&mut child).await;
                                let wait_error = child.wait().await.err(); // reap; no lock held
                                (
                                    JobStatus::Killed,
                                    Some(130),
                                    wait_error
                                        .map(|error| format!("child reap failed: {error}"))
                                        .into_iter()
                                        .collect(),
                                    st.lock()
                                        .termination_reason
                                        .unwrap_or(JobTerminationReason::OperatorCancelled),
                                )
                            }
                        };

                    // `child.wait()` only proves the process exited. Pipe pump
                    // tasks may still be draining kernel buffers, so terminal
                    // state and completion broadcast must wait for both EOFs.
                    diagnostics.extend(drain_output_pumps(stdout_pump, stderr_pump).await);
                    if !diagnostics.is_empty() {
                        termination_reason = JobTerminationReason::OutputIncomplete;
                        if status == JobStatus::Done {
                            status = JobStatus::Failed;
                        }
                        if exit_code.unwrap_or(0) == 0 {
                            exit_code = Some(1);
                        }
                    }

                    {
                        let mut s = st.lock();
                        for diagnostic in diagnostics {
                            append_capped(
                                &mut s.stderr,
                                &format!("\n[golish] output drain incomplete: {diagnostic}"),
                            );
                        }
                        s.status = status;
                        s.exit_code = exit_code;
                        s.finished_at = Some(Instant::now());
                        s.termination_reason = Some(termination_reason);
                    }
                    let reconciler = { st.lock().reconciler.clone() };
                    if let Some(reconciler) = reconciler {
                        let terminal = managed_terminal_for_state(job_id.clone(), &st).await;
                        let reconciliation = match reconciler.reconcile(terminal).await {
                            Ok(value) => value,
                            Err(error) => BackgroundJobReconciliation {
                                tool_result: serde_json::json!({
                                    "status": "reconciliation_failed",
                                    "exit_code": 1,
                                    "error": error.to_string(),
                                }),
                                note: Some(format!(
                                    "Managed job finished, but typed reconciliation failed: {error}"
                                )),
                                evidence_ids: Vec::new(),
                                skip_generic_persistence: true,
                            },
                        };
                        st.lock().reconciliation = Some(reconciliation);
                    }
                    // Broadcast the terminal state to per-session listeners.
                    // Send errors (no subscribers) are expected and ignored.
                    let completion = {
                        let s = st.lock();
                        terminal_completion(job_id, &s)
                    };
                    let _ = completions.send(completion);
                    state_changed.notify_waiters();
                });
            }
            Err(e) => {
                let reconciler = {
                    let mut s = state.lock();
                    s.status = JobStatus::Failed;
                    s.stderr = format!("Failed to spawn command: {e}");
                    s.stderr_total_bytes = s.stderr.len() as u64;
                    s.exit_code = Some(1);
                    s.finished_at = Some(Instant::now());
                    s.termination_reason = Some(JobTerminationReason::SpawnFailed);
                    s.reconciler.clone()
                };
                if let Some(reconciler) = reconciler {
                    let st = state.clone();
                    let job_id = id.clone();
                    let completions = self.completions.clone();
                    let state_changed = self.state_changed.clone();
                    tokio::spawn(async move {
                        let terminal = managed_terminal_for_state(job_id.clone(), &st).await;
                        let reconciliation = match reconciler.reconcile(terminal).await {
                            Ok(value) => value,
                            Err(error) => BackgroundJobReconciliation {
                                tool_result: serde_json::json!({
                                    "status": "reconciliation_failed",
                                    "exit_code": 1,
                                    "error": error.to_string(),
                                }),
                                note: Some(format!(
                                    "Managed job could not spawn, and typed reconciliation failed: {error}"
                                )),
                                evidence_ids: Vec::new(),
                                skip_generic_persistence: true,
                            },
                        };
                        st.lock().reconciliation = Some(reconciliation);
                        let completion = {
                            let s = st.lock();
                            terminal_completion(job_id, &s)
                        };
                        let _ = completions.send(completion);
                        state_changed.notify_waiters();
                    });
                } else {
                    let completion = {
                        let s = state.lock();
                        terminal_completion(id.clone(), &s)
                    };
                    let _ = self.completions.send(completion);
                    self.state_changed.notify_waiters();
                }
            }
        }

        id
    }

    /// Current view of a job, or `None` if the id is unknown.
    pub fn snapshot(&self, job_id: &str) -> Option<JobSnapshot> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        let s = state.lock();
        let end = s.finished_at.unwrap_or_else(Instant::now);
        Some(JobSnapshot {
            command: s.command.clone(),
            status: s.status,
            exit_code: s.exit_code,
            stdout: s.stdout.clone(),
            stderr: s.stderr.clone(),
            duration_ms: end.duration_since(s.started_at).as_millis() as u64,
            finished: s.status.is_terminal() && typed_reconciliation_complete(&s),
        })
    }

    /// Current process-output activity, retained for running and terminal jobs.
    pub fn activity(&self, job_id: &str) -> Option<JobActivity> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        let s = state.lock();
        Some(JobActivity {
            stdout_total_bytes: s.stdout_total_bytes,
            stderr_total_bytes: s.stderr_total_bytes,
            last_output_age_ms: s
                .last_output_at
                .map(|at| Instant::now().duration_since(at).as_millis() as u64),
        })
    }

    /// Attach a still-running inline job to the session background lifecycle.
    /// Returning false means the job is unknown or already terminal, so the
    /// caller must consume it inline instead of emitting a background handle.
    pub fn promote_to_session(&self, job_id: &str, session_id: String) -> bool {
        let Some(state) = self.jobs.lock().get(job_id).cloned() else {
            return false;
        };
        let mut s = state.lock();
        if s.status.is_terminal() && typed_reconciliation_complete(&s) {
            return false;
        }
        s.session_id = Some(session_id);
        s.reconciled = false;
        true
    }

    pub fn termination_reason(&self, job_id: &str) -> Option<JobTerminationReason> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        let reason = state.lock().termination_reason;
        reason
    }

    pub fn reconciliation(&self, job_id: &str) -> Option<BackgroundJobReconciliation> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        let reconciliation = state.lock().reconciliation.clone();
        reconciliation
    }

    /// Read the full terminal output archive. Returns `None` for unknown or
    /// still-running jobs, or when the spool files are unavailable.
    pub async fn read_output_archive(&self, job_id: &str) -> Option<JobOutputArchive> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        if !state.lock().status.is_terminal() {
            return None;
        }
        Some(output_archive_for_state(&state).await)
    }

    /// Await the exact job's terminal state. Terminal publication occurs only
    /// after `start_kill`/`wait` and both output pumps have drained, so callers
    /// may safely persist timeout/cancellation truth after this returns.
    pub async fn wait_terminal(&self, job_id: &str) -> Option<JobSnapshot> {
        let mut completions = self.subscribe_completions();
        loop {
            let snapshot = self.snapshot(job_id)?;
            if snapshot.finished {
                return Some(snapshot);
            }
            match completions.recv().await {
                Ok(completion) if completion.job_id == job_id => {}
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return self.snapshot(job_id),
            }
        }
    }

    /// Snapshot of the jobs still `Running` that were attributed to `session_id`,
    /// newest activity irrelevant (caller-order). Finished/killed jobs and jobs
    /// from other sessions are excluded. Used by the closeout reconciliation
    /// barrier so the agent doesn't conclude a stage while its own backgrounded
    /// scans are still in flight (their evidence hasn't landed yet).
    pub fn running_for_session(&self, session_id: &str) -> Vec<RunningJob> {
        let now = Instant::now();
        let jobs = self.jobs.lock();
        jobs.iter()
            .filter_map(|(id, state)| {
                let s = state.lock();
                if s.status == JobStatus::Running && s.session_id.as_deref() == Some(session_id) {
                    Some(RunningJob {
                        job_id: id.clone(),
                        command: s.command.clone(),
                        elapsed_ms: now.duration_since(s.started_at).as_millis() as u64,
                        stdout_total_bytes: s.stdout_total_bytes,
                        stderr_total_bytes: s.stderr_total_bytes,
                        last_output_age_ms: s
                            .last_output_at
                            .map(|at| now.duration_since(at).as_millis() as u64),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Jobs whose completion is not safe to close over yet. A job remains here
    /// while its process is running *and* after termination until the bridge
    /// listener acknowledges that all completion side effects have landed.
    pub fn outstanding_for_session(&self, session_id: &str) -> Vec<RunningJob> {
        let now = Instant::now();
        let jobs = self.jobs.lock();
        jobs.iter()
            .filter_map(|(id, state)| {
                let s = state.lock();
                if s.session_id.as_deref() == Some(session_id)
                    && (s.status == JobStatus::Running || !s.reconciled)
                {
                    Some(RunningJob {
                        job_id: id.clone(),
                        command: s.command.clone(),
                        elapsed_ms: now.duration_since(s.started_at).as_millis() as u64,
                        stdout_total_bytes: s.stdout_total_bytes,
                        stderr_total_bytes: s.stderr_total_bytes,
                        last_output_age_ms: s
                            .last_output_at
                            .map(|at| now.duration_since(at).as_millis() as u64),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Regenerate completion payloads that a lagged broadcast receiver may
    /// have missed. They reuse the original processing claim, so overlapping
    /// listener generations cannot duplicate evidence or notes.
    pub fn terminal_unreconciled_for_session(&self, session_id: &str) -> Vec<JobCompletion> {
        let jobs = self.jobs.lock();
        jobs.iter()
            .filter_map(|(id, state)| {
                let s = state.lock();
                if s.session_id.as_deref() == Some(session_id)
                    && s.status.is_terminal()
                    && typed_reconciliation_complete(&s)
                    && !s.reconciled
                {
                    Some(terminal_completion(id.clone(), &s))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Acknowledge a terminal completion only after all bridge side effects
    /// have landed. Returns false for unknown or still-running jobs.
    pub fn mark_reconciled(&self, job_id: &str) -> bool {
        let Some(state) = self.jobs.lock().get(job_id).cloned() else {
            return false;
        };
        let changed = {
            let mut s = state.lock();
            if !s.status.is_terminal() || !typed_reconciliation_complete(&s) {
                return false;
            }
            let changed = !s.reconciled;
            s.reconciled = true;
            changed
        };
        if changed {
            self.state_changed.notify_waiters();
        }
        true
    }

    /// Wait once for every background job owned by `session_id` to terminate
    /// and be reconciled. The returned list is empty on success, or contains
    /// the still-outstanding jobs when the system deadline expires.
    pub async fn wait_for_session_reconciled(
        &self,
        session_id: &str,
        max_wait: Duration,
    ) -> Vec<RunningJob> {
        let deadline = Instant::now() + max_wait;
        loop {
            // Register before checking state, preventing an acknowledgement in
            // the check→await gap from being lost.
            let notified = self.state_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let outstanding = self.outstanding_for_session(session_id);
            if outstanding.is_empty() {
                return outstanding;
            }
            let now = Instant::now();
            if now >= deadline {
                return outstanding;
            }
            if tokio::time::timeout(deadline.duration_since(now), notified)
                .await
                .is_err()
            {
                return self.outstanding_for_session(session_id);
            }
        }
    }

    /// Request cancellation of a running job. Returns `true` if the job exists.
    pub fn kill(&self, job_id: &str) -> bool {
        self.cancel_with_reason(job_id, JobTerminationReason::OperatorCancelled)
    }

    fn cancel_with_reason(&self, job_id: &str, reason: JobTerminationReason) -> bool {
        let Some(state) = self.jobs.lock().get(job_id).cloned() else {
            return false;
        };
        let kill = {
            let mut s = state.lock();
            if s.status.is_terminal() {
                return false;
            }
            s.termination_reason = Some(reason);
            s.kill.clone()
        };
        kill.notify_one();
        true
    }

    /// Request cancellation of every running job attributed to `session_id`.
    /// Returns how many job handles were signalled.
    pub fn kill_running_for_session(&self, session_id: &str) -> usize {
        let ids: Vec<String> = {
            let jobs = self.jobs.lock();
            jobs.iter()
                .filter_map(|(id, state)| {
                    let s = state.lock();
                    if s.status == JobStatus::Running && s.session_id.as_deref() == Some(session_id)
                    {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        ids.iter()
            .filter(|id| self.cancel_with_reason(id, JobTerminationReason::SessionCancelled))
            .count()
    }

    /// Forget a job (used after its result has been consumed inline).
    pub fn remove(&self, job_id: &str) {
        if let Some(state) = self.jobs.lock().remove(job_id) {
            remove_spool_files(&state.lock());
        }
        self.state_changed.notify_waiters();
    }

    /// Drop the oldest finished jobs once the map grows too large.
    fn prune(&self) {
        let mut jobs = self.jobs.lock();
        if jobs.len() < MAX_RETAINED_JOBS {
            return;
        }
        let finished: Vec<String> = jobs
            .iter()
            .filter(|(_, st)| {
                let state = st.lock();
                state.status.is_terminal()
                    && typed_reconciliation_complete(&state)
                    && state.reconciled
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            if let Some(state) = jobs.remove(&id) {
                remove_spool_files(&state.lock());
            }
        }
    }
}

/// Process-wide singleton.
pub fn manager() -> &'static BackgroundJobManager {
    static MANAGER: OnceLock<BackgroundJobManager> = OnceLock::new();
    MANAGER.get_or_init(BackgroundJobManager::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn ws() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    async fn wait_terminal(mgr: &BackgroundJobManager, id: &str, max: Duration) -> JobSnapshot {
        let deadline = Instant::now() + max;
        loop {
            let snap = mgr.snapshot(id).expect("job exists");
            if snap.finished || Instant::now() >= deadline {
                return snap;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("echo hello-bg", &ws(), Duration::from_secs(10));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Done);
        assert_eq!(snap.exit_code, Some(0));
        assert!(
            snap.stdout.contains("hello-bg"),
            "stdout was: {:?}",
            snap.stdout
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_job_has_no_elapsed_deadline() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("sleep 5", &ws(), Duration::from_millis(40));

        tokio::time::sleep(Duration::from_millis(120)).await;

        let snap = mgr.snapshot(&id).expect("managed job remains observable");
        assert_eq!(
            snap.status,
            JobStatus::Running,
            "elapsed time must not terminate a live managed process"
        );
        assert!(mgr.kill(&id));
        let terminal = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(terminal.status, JobStatus::Killed);
        assert_eq!(
            mgr.termination_reason(&id),
            Some(JobTerminationReason::OperatorCancelled)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_job_reports_last_output_activity() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn(
            "printf first; sleep 0.2; printf second",
            &ws(),
            Duration::ZERO,
        );

        tokio::time::sleep(Duration::from_millis(80)).await;
        let first = mgr.activity(&id).expect("activity remains queryable");
        assert!(first.stdout_total_bytes >= 5);
        assert_eq!(first.stderr_total_bytes, 0);
        assert!(first.last_output_age_ms.is_some_and(|age| age < 200));

        let terminal = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(terminal.status, JobStatus::Done);
        let final_activity = mgr.activity(&id).expect("terminal activity is retained");
        assert!(final_activity.stdout_total_bytes >= 11);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_job_spool_preserves_output_beyond_tail() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn(
            "python3 -c 'import sys; sys.stdout.write(\"HEAD_SENTINEL\" + \"x\" * 600000 + \"TAIL_SENTINEL\")'",
            &ws(),
            Duration::ZERO,
        );

        let terminal = wait_terminal(&mgr, &id, Duration::from_secs(10)).await;
        assert_eq!(terminal.status, JobStatus::Done);
        assert!(
            !terminal.stdout.contains("HEAD_SENTINEL"),
            "the bounded in-memory tail should have dropped the oldest bytes"
        );

        let archive = mgr
            .read_output_archive(&id)
            .await
            .expect("terminal spool remains readable");
        assert!(archive.complete);
        assert!(archive.stdout.starts_with("HEAD_SENTINEL"));
        assert!(archive.stdout.ends_with("TAIL_SENTINEL"));
        assert_eq!(archive.stdout_total_bytes, archive.stdout.len() as u64);
    }

    struct FixtureReconciler {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl BackgroundJobReconciler for FixtureReconciler {
        async fn reconcile(
            &self,
            terminal: ManagedJobTerminal,
        ) -> anyhow::Result<BackgroundJobReconciliation> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(terminal.termination_reason, JobTerminationReason::Exited);
            assert!(terminal.output.complete);
            Ok(BackgroundJobReconciliation {
                tool_result: serde_json::json!({"typed": true}),
                note: Some("fixture reconciled".to_string()),
                evidence_ids: vec![41],
                skip_generic_persistence: true,
            })
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn typed_reconciler_runs_once_before_completion() {
        let mgr = BackgroundJobManager::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut completions = mgr.subscribe_completions();
        let id = mgr.spawn_for_session_and_tool_with_reconciler(
            "printf typed-output",
            &ws(),
            None,
            None,
            Arc::new(FixtureReconciler {
                calls: calls.clone(),
            }),
        );

        let completion = tokio::time::timeout(Duration::from_secs(5), completions.recv())
            .await
            .expect("completion is broadcast")
            .expect("completion channel remains open");
        assert_eq!(completion.job_id, id);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let reconciled = mgr.reconciliation(&id).expect("typed result retained");
        assert_eq!(reconciled.tool_result["typed"], true);
        assert!(reconciled.skip_generic_persistence);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_snapshot_waits_for_large_stdout_and_stderr_pumps() {
        const BYTES_PER_STREAM: usize = 128 * 1024;

        // Repeat to catch the scheduler race where child.wait wins before one
        // of the pipe pumps gets its final turns.
        for _ in 0..3 {
            let mgr = BackgroundJobManager::new();
            let command = format!(
                "python3 -c 'import sys; sys.stdout.write(\"A\"*{BYTES_PER_STREAM}); sys.stderr.write(\"B\"*{BYTES_PER_STREAM})'"
            );
            let id = mgr.spawn(&command, &ws(), Duration::from_secs(10));
            let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;

            assert_eq!(snap.status, JobStatus::Done, "stderr={:?}", snap.stderr);
            assert_eq!(snap.exit_code, Some(0));
            assert_eq!(snap.stdout.len(), BYTES_PER_STREAM);
            assert_eq!(snap.stderr.len(), BYTES_PER_STREAM);
            assert!(snap.stdout.bytes().all(|byte| byte == b'A'));
            assert!(snap.stderr.bytes().all(|byte| byte == b'B'));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inherited_pipe_that_cannot_drain_fails_closed() {
        let mgr = BackgroundJobManager::new();
        // The direct child exits, but the detached grandchild retains both
        // inherited pipes beyond OUTPUT_DRAIN_TIMEOUT.
        let id = mgr.spawn("sh -c 'sleep 3 &'", &ws(), Duration::from_secs(10));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;

        assert_eq!(snap.status, JobStatus::Failed);
        assert_ne!(snap.exit_code, Some(0));
        assert!(
            snap.stderr.contains("output drain incomplete"),
            "stderr={:?}",
            snap.stderr
        );
    }

    #[tokio::test]
    async fn nonzero_exit_marks_failed() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("exit 3", &ws(), Duration::from_secs(10));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Failed);
        assert_eq!(snap.exit_code, Some(3));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn long_job_stays_running_then_can_be_killed() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("sleep 30", &ws(), Duration::from_secs(60));
        // Still running shortly after spawn.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let running = mgr.snapshot(&id).unwrap();
        assert_eq!(running.status, JobStatus::Running);
        assert!(!running.finished);
        // Cancel it and confirm it reaches a terminal Killed state.
        assert!(mgr.kill(&id));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Killed);
    }

    #[test]
    fn append_capped_keeps_tail_on_char_boundary() {
        let mut buf = String::new();
        let big = "é".repeat(MAX_JOB_OUTPUT_BYTES); // 2 bytes each → exceeds cap
        append_capped(&mut buf, &big);
        assert!(buf.len() <= MAX_JOB_OUTPUT_BYTES);
        // Still valid UTF-8 (no panic / no broken char) and non-empty.
        assert!(!buf.is_empty());
        assert!(buf.chars().all(|c| c == 'é'));
    }

    #[test]
    fn snapshot_none_for_unknown() {
        let mgr = BackgroundJobManager::new();
        assert!(mgr.snapshot("job_doesnotexist").is_none());
        assert!(!mgr.kill("job_doesnotexist"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn running_for_session_filters_by_session_and_terminal_state() {
        let mgr = BackgroundJobManager::new();
        // Two long jobs for sess-A, one for sess-B, plus a quick one for sess-A
        // that will finish and must drop out of the running list.
        let a_long = mgr.spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some("sess-A".to_string()),
        );
        let _a_other = mgr.spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some("sess-A".to_string()),
        );
        let _b_long = mgr.spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some("sess-B".to_string()),
        );
        let a_quick = mgr.spawn_for_session(
            "true",
            &ws(),
            Duration::from_secs(60),
            Some("sess-A".to_string()),
        );
        // Let the quick job finish.
        let _ = wait_terminal(&mgr, &a_quick, Duration::from_secs(5)).await;

        let running = mgr.running_for_session("sess-A");
        assert_eq!(
            running.len(),
            2,
            "sess-A has exactly two still-running jobs (the finished one drops out): {running:?}"
        );
        assert!(running.iter().any(|j| j.job_id == a_long));
        assert!(
            running.iter().all(|j| j.command == "sleep 30"),
            "the finished `true` job must not appear: {running:?}"
        );

        // Unknown session → empty.
        assert!(mgr.running_for_session("sess-unknown").is_empty());

        // Clean up the long-runners so the test process doesn't leak children.
        for j in mgr.running_for_session("sess-A") {
            mgr.kill(&j.job_id);
        }
        mgr.kill(&_b_long);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_running_for_session_only_signals_matching_jobs() {
        let mgr = BackgroundJobManager::new();
        let a_long = mgr.spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some("sess-A".to_string()),
        );
        let _b_long = mgr.spawn_for_session(
            "sleep 30",
            &ws(),
            Duration::from_secs(60),
            Some("sess-B".to_string()),
        );

        assert_eq!(mgr.kill_running_for_session("sess-A"), 1);
        let snap = wait_terminal(&mgr, &a_long, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Killed);
        assert_eq!(
            mgr.running_for_session("sess-B").len(),
            1,
            "other sessions must not be killed by session cancellation"
        );

        for j in mgr.running_for_session("sess-B") {
            mgr.kill(&j.job_id);
        }
    }

    #[tokio::test]
    async fn completion_is_broadcast_with_session_and_exit_code() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_completions();
        let id = mgr.spawn_for_session(
            "echo bg-complete",
            &ws(),
            Duration::from_secs(10),
            Some("sess-42".to_string()),
        );

        let completion = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("completion broadcast within timeout")
            .expect("completion received");

        assert_eq!(completion.job_id, id);
        assert_eq!(completion.session_id.as_deref(), Some("sess-42"));
        assert_eq!(completion.status, JobStatus::Done);
        assert_eq!(completion.exit_code, Some(0));
        assert!(
            completion.stdout_tail.contains("bg-complete"),
            "stdout_tail was: {:?}",
            completion.stdout_tail
        );
    }

    #[tokio::test]
    async fn terminal_job_remains_outstanding_until_completion_is_reconciled() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn_for_session(
            "printf landed",
            &ws(),
            Duration::from_secs(10),
            Some("sess-landed".to_string()),
        );
        let _ = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;

        assert!(
            mgr.outstanding_for_session("sess-landed")
                .iter()
                .any(|job| job.job_id == id),
            "terminal output is still outstanding until listener side effects land"
        );
        assert!(mgr.mark_reconciled(&id));
        assert!(mgr.outstanding_for_session("sess-landed").is_empty());
    }

    #[tokio::test]
    async fn session_reconciliation_wait_wakes_after_listener_ack() {
        let mgr = Arc::new(BackgroundJobManager::new());
        let id = mgr.spawn_for_session(
            "printf wake",
            &ws(),
            Duration::from_secs(10),
            Some("sess-wake".to_string()),
        );
        let _ = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;

        let waiter = {
            let mgr = mgr.clone();
            tokio::spawn(async move {
                mgr.wait_for_session_reconciled("sess-wake", Duration::from_secs(5))
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(mgr.mark_reconciled(&id));

        let remaining = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("ack wakes waiter")
            .expect("wait task succeeds");
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn overlapping_handoff_subscribers_receive_without_duplicate_processing() {
        let mgr = BackgroundJobManager::new();
        let mut old_generation = mgr.subscribe_completions();
        let mut prepared_generation = mgr.subscribe_completions();
        mgr.spawn_for_session(
            "echo handoff-complete",
            &ws(),
            Duration::from_secs(10),
            Some("sess-handoff".to_string()),
        );

        let old = tokio::time::timeout(Duration::from_secs(5), old_generation.recv())
            .await
            .expect("old subscriber receives")
            .expect("old completion");
        let prepared = tokio::time::timeout(Duration::from_secs(5), prepared_generation.recv())
            .await
            .expect("prepared subscriber receives")
            .expect("prepared completion");

        assert_eq!(old.job_id, prepared.job_id);
        assert_ne!(
            old.try_claim_processing(),
            prepared.try_claim_processing(),
            "overlapping generations share one exactly-once processing claim"
        );
    }

    #[tokio::test]
    async fn lag_replay_reuses_the_original_exactly_once_claim() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_completions();
        let id = mgr.spawn_for_session(
            "printf replay",
            &ws(),
            Duration::from_secs(10),
            Some("sess-replay".to_string()),
        );
        let broadcast = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("completion broadcast within timeout")
            .expect("completion received");
        let replay = mgr
            .terminal_unreconciled_for_session("sess-replay")
            .into_iter()
            .find(|completion| completion.job_id == id)
            .expect("terminal unreconciled job can be replayed");

        let claims = usize::from(broadcast.try_claim_processing())
            + usize::from(replay.try_claim_processing());
        assert_eq!(claims, 1, "broadcast and replay share one processing claim");
    }

    #[tokio::test]
    async fn controlled_handoff_drains_pre_subscription_event_and_claims_overlap_once() {
        fn completion(job_id: &str) -> JobCompletion {
            JobCompletion {
                job_id: job_id.to_string(),
                session_id: Some("sess-handoff".to_string()),
                organization_id: None,
                command: "echo handoff".to_string(),
                status: JobStatus::Done,
                exit_code: Some(0),
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                duration_ms: 1,
                processing_claim: Arc::new(AtomicBool::new(false)),
            }
        }

        let (tx, _) = broadcast::channel(8);
        let mut old_generation = tx.subscribe();
        tx.send(completion("before-subscribe")).unwrap();
        let mut prepared_generation = tx.subscribe();

        // Retirement drain owns the only copy of a completion that predates
        // candidate subscription, so it must consume it rather than exit first.
        let before = old_generation.try_recv().expect("old queue retains event");
        assert!(before.try_claim_processing());

        tx.send(completion("overlap")).unwrap();
        let overlap_old = old_generation.recv().await.unwrap();
        let overlap_new = prepared_generation.recv().await.unwrap();
        let overlap_claims = usize::from(overlap_old.try_claim_processing())
            + usize::from(overlap_new.try_claim_processing());
        assert_eq!(overlap_claims, 1);
    }

    #[tokio::test]
    async fn output_chunk_is_broadcast_with_tool_context() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_output_chunks();
        let ctx = golish_core::AgentToolContext {
            request_id: "req-live".to_string(),
            tool_call_record_id: None,
            tool_name: "pentest_run".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: None,
            stage_execution_id: None,
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };
        let id = mgr.spawn_for_session_and_tool(
            "printf live-out",
            &ws(),
            Duration::from_secs(10),
            Some("sess-live".to_string()),
            Some(ctx),
        );

        let chunk = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("output chunk within timeout")
            .expect("output chunk received");

        assert_eq!(chunk.job_id, id);
        assert_eq!(chunk.session_id.as_deref(), Some("sess-live"));
        assert_eq!(chunk.request_id, "req-live");
        assert_eq!(chunk.tool_name, "pentest_run");
        assert_eq!(chunk.stream, "stdout");
        assert_eq!(chunk.source, golish_core::events::ToolSource::Main);
        assert!(
            chunk.chunk.contains("live-out"),
            "chunk was: {:?}",
            chunk.chunk
        );

        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Done);
    }

    #[tokio::test]
    async fn candidate_context_rejects_background_execution_before_spawn() {
        let mgr = BackgroundJobManager::new();
        let unit_id = uuid::Uuid::new_v4();
        let context = golish_core::AgentToolContext {
            request_id: "candidate-action".to_string(),
            tool_call_record_id: None,
            tool_name: "verify_execute_candidate_action".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(uuid::Uuid::new_v4()),
            stage_execution_id: Some(uuid::Uuid::new_v4()),
            stage_run_unit_id: Some(unit_id),
            organization_id: Some(uuid::Uuid::new_v4()),
            worker_lease: Some(golish_core::WorkerLeaseContext {
                worker_run_id: uuid::Uuid::new_v4(),
                stage_run_unit_id: unit_id,
                lease_token: uuid::Uuid::new_v4(),
                attempt_epoch: 1,
            }),
            candidate_attempt: Some(golish_core::CandidateAttemptContextRef {
                candidate_id: uuid::Uuid::new_v4(),
                approval_id: uuid::Uuid::new_v4(),
                attempt_id: uuid::Uuid::new_v4(),
                candidate_plan_hash: "sha256:plan".to_string(),
            }),
        };

        let error = mgr
            .try_spawn_for_session_and_tool(
                "printf should-not-run",
                &ws(),
                Duration::from_secs(10),
                Some("candidate-session".to_string()),
                Some(context),
            )
            .unwrap_err();
        assert_eq!(error.code(), ATTACK_VERIFIER_FOREGROUND_REQUIRED);
        assert!(mgr.running_for_session("candidate-session").is_empty());
    }

    #[tokio::test]
    async fn completion_reports_failed_status() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_completions();
        let _id = mgr.spawn("exit 7", &ws(), Duration::from_secs(10));

        let completion = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("completion within timeout")
            .expect("completion received");

        assert_eq!(completion.status, JobStatus::Failed);
        assert_eq!(completion.exit_code, Some(7));
        // Unattributed spawn → no session routing.
        assert_eq!(completion.session_id, None);
    }

    #[tokio::test]
    async fn spawn_failure_still_broadcasts_one_terminal_completion() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_completions();
        let missing_workspace =
            std::env::temp_dir().join(format!("golish-missing-workspace-{}", uuid::Uuid::new_v4()));
        let job_id = mgr.spawn_for_session(
            "echo never-starts",
            &missing_workspace,
            Duration::from_secs(10),
            Some("spawn-failed-session".to_string()),
        );

        let completion = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("spawn failure completion arrives")
            .expect("completion channel open");
        assert_eq!(completion.job_id, job_id);
        assert_eq!(completion.status, JobStatus::Failed);
        assert_eq!(completion.exit_code, Some(1));
        assert!(completion.stderr_tail.contains("Failed to spawn command"));
        assert!(completion.try_claim_processing());
        assert!(!completion.try_claim_processing());
    }

    #[test]
    fn tail_capped_keeps_trailing_bytes_on_char_boundary() {
        let short = "hello";
        assert_eq!(tail_capped(short, 1024), "hello");

        let big = "é".repeat(10_000); // 2 bytes each
        let out = tail_capped(&big, 1024);
        assert!(out.contains("earlier bytes"));
        // The retained tail must be valid UTF-8 (only whole 'é' chars).
        let tail = out.split('\n').next_back().unwrap();
        assert!(tail.chars().all(|c| c == 'é'));
        assert!(tail.len() <= 1024 + 2);
    }
}
