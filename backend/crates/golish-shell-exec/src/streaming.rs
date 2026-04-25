//! Streaming shell-command execution.
//!
//! [`execute_streaming`] is the streaming counterpart to the synchronous
//! [`crate::tool::RunPtyCmdTool`] — it emits chunks via an mpsc channel as
//! they arrive (time-buffered every [`FLUSH_INTERVAL_MS`]) so the
//! frontend / sub-agent layer can render real-time output.

use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::common::{resolve_cwd, truncate_output, MAX_OUTPUT_SIZE};
use crate::process_group::{configure_process_group, kill_process_group};
use crate::shell::get_shell_config;

/// Output chunk from a streaming command execution.
#[derive(Debug, Clone)]
pub struct OutputChunk {
    /// The output data (may contain ANSI codes).
    pub data: String,
    /// Which stream this came from.
    pub stream: OutputStream,
}

/// Which output stream a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    /// Convert to string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputStream::Stdout => "stdout",
            OutputStream::Stderr => "stderr",
        }
    }
}

/// Result of a streaming command execution.
#[derive(Debug, Clone)]
pub struct StreamingResult {
    /// Accumulated stdout.
    pub stdout: String,
    /// Accumulated stderr.
    pub stderr: String,
    /// Exit code.
    pub exit_code: i32,
    /// Whether the command timed out.
    pub timed_out: bool,
}

/// Flush interval for time-buffered output (100 ms).
const FLUSH_INTERVAL_MS: u64 = 100;

/// Execute a shell command with streaming output.
///
/// This function is similar to `RunPtyCmdTool::execute` but emits output
/// chunks as they arrive via the provided channel, enabling real-time
/// feedback for long-running commands.
///
/// # Arguments
/// * `command` — the shell command to execute.
/// * `cwd` — optional working directory (relative to `workspace`).
/// * `timeout_secs` — timeout in seconds.
/// * `workspace` — workspace root path.
/// * `shell_override` — optional shell path override.
/// * `chunk_tx` — channel sender for output chunks.
///
/// # Returns
/// The final result with accumulated stdout/stderr and exit code.
pub async fn execute_streaming(
    command: &str,
    cwd: Option<&str>,
    timeout_secs: u64,
    workspace: &Path,
    shell_override: Option<&str>,
    chunk_tx: mpsc::Sender<OutputChunk>,
) -> Result<StreamingResult> {
    let working_dir = resolve_cwd(cwd, workspace);

    if !working_dir.exists() {
        return Ok(StreamingResult {
            stdout: String::new(),
            stderr: format!("Working directory not found: {}", working_dir.display()),
            exit_code: 1,
            timed_out: false,
        });
    }

    // Determine shell and command to use.
    let (shell, wrapped_command) = if cfg!(target_os = "windows") {
        ("cmd".to_string(), command.to_string())
    } else {
        let (shell_path, shell_type, home) = get_shell_config(shell_override);
        shell_type.build_command(&shell_path, command, &home)
    };

    let shell_arg = if cfg!(target_os = "windows") { "/c" } else { "-c" };

    tracing::info!(
        shell = %shell,
        original_command = %command,
        wrapped_command = %wrapped_command,
        "Executing shell command (streaming)"
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
            return Ok(StreamingResult {
                stdout: String::new(),
                stderr: format!("Failed to spawn command: {}", e),
                exit_code: 1,
                timed_out: false,
            });
        }
    };

    let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);
    let flush_interval = tokio::time::Duration::from_millis(FLUSH_INTERVAL_MS);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Spawn tasks to read stdout and stderr with time-buffered output.
    let chunk_tx_stdout = chunk_tx.clone();
    let stdout_handle = tokio::spawn(async move {
        let mut accumulated = String::new();
        tracing::debug!("stdout reader started");
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            let mut buffer = String::new();
            let mut last_flush = tokio::time::Instant::now();

            loop {
                let mut line = String::new();
                match tokio::time::timeout(flush_interval, reader.read_line(&mut line)).await {
                    Ok(Ok(0)) => {
                        if !buffer.is_empty() {
                            tracing::info!("stdout EOF flush: {} bytes", buffer.len());
                            let _ = chunk_tx_stdout
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stdout,
                                })
                                .await;
                        }
                        break;
                    }
                    Ok(Ok(_)) => {
                        buffer.push_str(&line);
                        accumulated.push_str(&line);

                        if last_flush.elapsed() >= flush_interval {
                            tracing::info!("stdout time flush: {} bytes", buffer.len());
                            let _ = chunk_tx_stdout
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stdout,
                                })
                                .await;
                            buffer.clear();
                            last_flush = tokio::time::Instant::now();
                        }
                    }
                    Ok(Err(_)) => {
                        if !buffer.is_empty() {
                            tracing::info!("stdout error flush: {} bytes", buffer.len());
                            let _ = chunk_tx_stdout
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stdout,
                                })
                                .await;
                        }
                        break;
                    }
                    Err(_) => {
                        if !buffer.is_empty() {
                            let _ = chunk_tx_stdout
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stdout,
                                })
                                .await;
                            buffer.clear();
                            last_flush = tokio::time::Instant::now();
                        }
                    }
                }
            }
        }
        accumulated
    });
    let stdout_abort = stdout_handle.abort_handle();

    let chunk_tx_stderr = chunk_tx;
    let stderr_handle = tokio::spawn(async move {
        let mut accumulated = String::new();
        if let Some(stderr) = stderr {
            let mut reader = BufReader::new(stderr);
            let mut buffer = String::new();
            let mut last_flush = tokio::time::Instant::now();

            loop {
                let mut line = String::new();
                match tokio::time::timeout(flush_interval, reader.read_line(&mut line)).await {
                    Ok(Ok(0)) => {
                        if !buffer.is_empty() {
                            let _ = chunk_tx_stderr
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stderr,
                                })
                                .await;
                        }
                        break;
                    }
                    Ok(Ok(_)) => {
                        buffer.push_str(&line);
                        accumulated.push_str(&line);

                        if last_flush.elapsed() >= flush_interval {
                            let _ = chunk_tx_stderr
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stderr,
                                })
                                .await;
                            buffer.clear();
                            last_flush = tokio::time::Instant::now();
                        }
                    }
                    Ok(Err(_)) => {
                        if !buffer.is_empty() {
                            let _ = chunk_tx_stderr
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stderr,
                                })
                                .await;
                        }
                        break;
                    }
                    Err(_) => {
                        if !buffer.is_empty() {
                            let _ = chunk_tx_stderr
                                .send(OutputChunk {
                                    data: buffer.clone(),
                                    stream: OutputStream::Stderr,
                                })
                                .await;
                            buffer.clear();
                            last_flush = tokio::time::Instant::now();
                        }
                    }
                }
            }
        }
        accumulated
    });
    let stderr_abort = stderr_handle.abort_handle();

    // Wait for process with timeout.
    let result = tokio::time::timeout(timeout_duration, async {
        let stdout_result = stdout_handle.await.unwrap_or_default();
        let stderr_result = stderr_handle.await.unwrap_or_default();
        let status = child.wait().await;
        (stdout_result, stderr_result, status)
    })
    .await;

    match result {
        Ok((stdout, stderr, status)) => {
            let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            Ok(StreamingResult {
                stdout: truncate_output(stdout.as_bytes(), MAX_OUTPUT_SIZE),
                stderr: truncate_output(stderr.as_bytes(), MAX_OUTPUT_SIZE),
                exit_code,
                timed_out: false,
            })
        }
        Err(_) => {
            // Timeout — abort reader tasks and kill the process.
            stdout_abort.abort();
            stderr_abort.abort();
            kill_process_group(&mut child).await;
            Ok(StreamingResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout_secs),
                exit_code: 124,
                timed_out: true,
            })
        }
    }
}
