//! Docker exec helpers used by the SWE-bench test tool.
//!
//! Lives in its own module so that `execute.rs` can stay focused on the
//! tool-call orchestration while this file owns the bollard interaction and
//! output truncation logic.

use anyhow::{Context, Result};

/// Run a command inside the named Docker container.
///
/// The command is executed under bash with the testbed conda env activated.
/// Returns `(stdout, stderr, exit_code)`.
pub(super) async fn run_in_container(
    container_name: &str,
    command: &str,
) -> Result<(String, String, i64)> {
    use bollard::exec::{CreateExecOptions, StartExecResults};
    use bollard::Docker;
    use futures::StreamExt;

    let docker = Docker::connect_with_local_defaults().context("Failed to connect to Docker")?;

    let full_command = format!(
        "source /opt/miniconda3/etc/profile.d/conda.sh && conda activate testbed && {}",
        command
    );
    let exec_options = CreateExecOptions {
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        cmd: Some(vec!["bash", "-c", &full_command]),
        ..Default::default()
    };

    let exec = docker
        .create_exec(container_name, exec_options)
        .await
        .context("Failed to create exec")?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    match docker.start_exec(&exec.id, None).await? {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(Ok(msg)) = output.next().await {
                match msg {
                    bollard::container::LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }
        StartExecResults::Detached => {
            anyhow::bail!("Exec started in detached mode unexpectedly");
        }
    }

    let inspect = docker.inspect_exec(&exec.id).await?;
    let exit_code = inspect.exit_code.unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

/// Truncate output to a maximum length, preserving a tail marker.
pub(super) fn truncate_output(output: &str, max_len: usize) -> String {
    if output.len() <= max_len {
        output.to_string()
    } else {
        format!(
            "{}...\n\n[Output truncated, {} bytes total]",
            &output[..max_len],
            output.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_output_short_string_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_output(s, 100), s);
    }

    #[test]
    fn truncate_output_long_string_appends_marker() {
        let s = "x".repeat(200);
        let truncated = truncate_output(&s, 50);
        assert!(truncated.starts_with(&"x".repeat(50)));
        assert!(truncated.contains("[Output truncated"));
        assert!(truncated.contains("200 bytes total"));
    }
}
