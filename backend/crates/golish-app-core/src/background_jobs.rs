//! Background job manager for long-running AI shell/pentest commands.
//!
//! Replaces the old "timeout → kill the process → report a timeout error"
//! behaviour with a Cursor-style "soft timeout → keep running in the
//! background" model:
//!
//! - A command is always spawned through [`BackgroundJobManager::spawn`], which
//!   captures stdout/stderr incrementally into a capped buffer and **does not**
//!   `kill_on_drop` — the child outlives the awaiting future.
//! - The caller (`run_shell_command_detail`) waits up to a *soft* timeout; if
//!   the command finishes in time it returns the full result as before, else it
//!   returns a `backgrounded` handle (`job_id`) and the command keeps running.
//! - The AI polls [`BackgroundJobManager::snapshot`] via the `check_job` tool.
//! - A *hard* limit watchdog kills runaway jobs so nothing leaks forever.
//!
//! Design: `docs/design/2026-06-03-background-tool-execution.md` (P1).

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, Notify};

use crate::pty_interactive::shell_command;

/// Cap each stream's retained buffer; keep the *tail* once exceeded so the most
/// recent output (usually the interesting part) survives.
const MAX_JOB_OUTPUT_BYTES: usize = 512 * 1024;
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

/// High-level lifecycle state of a background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Killed,
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
    started_at: Instant,
    finished_at: Option<Instant>,
    kill: Arc<Notify>,
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

/// Lightweight view of a still-running job attributed to a session. Used by the
/// closeout reconciliation barrier (`submit_stage_deliverable`) to tell the
/// agent which background scans are still in flight before it concludes a stage.
#[derive(Debug, Clone)]
pub struct RunningJob {
    pub job_id: String,
    pub command: String,
    pub elapsed_ms: u64,
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
    pub command: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    /// Size-capped tail of stdout (see [`COMPLETION_TAIL_BYTES`]).
    pub stdout_tail: String,
    /// Size-capped tail of stderr (see [`COMPLETION_TAIL_BYTES`]).
    pub stderr_tail: String,
    pub duration_ms: u64,
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

async fn pump<R>(
    mut reader: R,
    state: Arc<Mutex<JobState>>,
    outputs: broadcast::Sender<JobOutputChunk>,
    job_id: String,
    is_stderr: bool,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                let stream = if is_stderr { "stderr" } else { "stdout" };
                let output_event = {
                    let mut s = state.lock();
                    if is_stderr {
                        append_capped(&mut s.stderr, &chunk);
                    } else {
                        append_capped(&mut s.stdout, &chunk);
                    }
                    s.tool_context.as_ref().map(|ctx| JobOutputChunk {
                        job_id: job_id.clone(),
                        session_id: s.session_id.clone(),
                        request_id: ctx.request_id.clone(),
                        tool_name: ctx.tool_name.clone(),
                        source: ctx.source.clone(),
                        stream,
                        chunk: chunk.to_string(),
                    })
                };
                if let Some(event) = output_event {
                    let _ = outputs.send(event);
                }
            }
        }
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
    pub fn spawn(&self, command: &str, workspace: &Path, hard_limit: Duration) -> String {
        self.spawn_for_session(command, workspace, hard_limit, None)
    }

    /// Spawn `command` in the background, attributing it to `session_id` so the
    /// completion broadcast can be routed back to the right session. Returns
    /// immediately with a `job_id`. The child is reaped by an internal task;
    /// output streams into a capped buffer; the `hard_limit` watchdog kills it
    /// if it overruns; a [`JobCompletion`] is broadcast when it terminates.
    pub fn spawn_for_session(
        &self,
        command: &str,
        workspace: &Path,
        hard_limit: Duration,
        session_id: Option<String>,
    ) -> String {
        self.spawn_for_session_and_tool(command, workspace, hard_limit, session_id, None)
    }

    /// Spawn `command` in the background, additionally attributing live output
    /// to an agent tool call when `tool_context` is provided.
    pub fn spawn_for_session_and_tool(
        &self,
        command: &str,
        workspace: &Path,
        hard_limit: Duration,
        session_id: Option<String>,
        tool_context: Option<golish_core::AgentToolContext>,
    ) -> String {
        self.prune();

        let id = format!("job_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let kill = Arc::new(Notify::new());
        let state = Arc::new(Mutex::new(JobState {
            command: command.to_string(),
            session_id,
            tool_context,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            started_at: Instant::now(),
            finished_at: None,
            kill: kill.clone(),
        }));
        self.jobs.lock().insert(id.clone(), state.clone());

        let mut cmd = shell_command(command);
        cmd.current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Intentionally NOT kill_on_drop: the whole point is to keep running.

        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(out) = child.stdout.take() {
                    tokio::spawn(pump(
                        out,
                        state.clone(),
                        self.outputs.clone(),
                        id.clone(),
                        false,
                    ));
                }
                if let Some(err) = child.stderr.take() {
                    tokio::spawn(pump(
                        err,
                        state.clone(),
                        self.outputs.clone(),
                        id.clone(),
                        true,
                    ));
                }

                let st = state.clone();
                let completions = self.completions.clone();
                let job_id = id.clone();
                tokio::spawn(async move {
                    enum Outcome {
                        Exited(std::io::Result<std::process::ExitStatus>),
                        Stopped, // hard-timeout or explicit kill
                    }
                    let outcome = tokio::select! {
                        r = child.wait() => Outcome::Exited(r),
                        _ = tokio::time::sleep(hard_limit) => Outcome::Stopped,
                        _ = kill.notified() => Outcome::Stopped,
                    };
                    match outcome {
                        Outcome::Exited(Ok(status)) => {
                            let mut s = st.lock();
                            s.exit_code = status.code();
                            s.status = if status.success() {
                                JobStatus::Done
                            } else {
                                JobStatus::Failed
                            };
                            s.finished_at = Some(Instant::now());
                        }
                        Outcome::Exited(Err(_)) => {
                            let mut s = st.lock();
                            s.status = JobStatus::Failed;
                            s.finished_at = Some(Instant::now());
                        }
                        Outcome::Stopped => {
                            let _ = child.start_kill();
                            let _ = child.wait().await; // reap; no lock held
                            let mut s = st.lock();
                            s.status = JobStatus::Killed;
                            s.exit_code = Some(124);
                            s.finished_at = Some(Instant::now());
                        }
                    }
                    // Broadcast the terminal state to per-session listeners.
                    // Send errors (no subscribers) are expected and ignored.
                    let completion = {
                        let s = st.lock();
                        let end = s.finished_at.unwrap_or_else(Instant::now);
                        JobCompletion {
                            job_id,
                            session_id: s.session_id.clone(),
                            command: s.command.clone(),
                            status: s.status,
                            exit_code: s.exit_code,
                            stdout_tail: tail_capped(&s.stdout, COMPLETION_TAIL_BYTES),
                            stderr_tail: tail_capped(&s.stderr, COMPLETION_TAIL_BYTES),
                            duration_ms: end.duration_since(s.started_at).as_millis() as u64,
                        }
                    };
                    let _ = completions.send(completion);
                });
            }
            Err(e) => {
                let mut s = state.lock();
                s.status = JobStatus::Failed;
                s.stderr = format!("Failed to spawn command: {e}");
                s.exit_code = Some(1);
                s.finished_at = Some(Instant::now());
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
            finished: s.status.is_terminal(),
        })
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
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Request cancellation of a running job. Returns `true` if the job exists.
    pub fn kill(&self, job_id: &str) -> bool {
        if let Some(state) = self.jobs.lock().get(job_id).cloned() {
            state.lock().kill.notify_one();
            true
        } else {
            false
        }
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
        ids.iter().filter(|id| self.kill(id)).count()
    }

    /// Forget a job (used after its result has been consumed inline).
    pub fn remove(&self, job_id: &str) {
        self.jobs.lock().remove(job_id);
    }

    /// Drop the oldest finished jobs once the map grows too large.
    fn prune(&self) {
        let mut jobs = self.jobs.lock();
        if jobs.len() < MAX_RETAINED_JOBS {
            return;
        }
        let finished: Vec<String> = jobs
            .iter()
            .filter(|(_, st)| st.lock().status.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            jobs.remove(&id);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn hard_limit_kills_overrun() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("sleep 30", &ws(), Duration::from_millis(200));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Killed);
        assert_eq!(snap.exit_code, Some(124));
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
    async fn output_chunk_is_broadcast_with_tool_context() {
        let mgr = BackgroundJobManager::new();
        let mut rx = mgr.subscribe_output_chunks();
        let ctx = golish_core::AgentToolContext {
            request_id: "req-live".to_string(),
            tool_name: "pentest_run".to_string(),
            source: golish_core::events::ToolSource::Main,
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
