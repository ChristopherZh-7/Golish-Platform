use async_trait::async_trait;
use graph_flow::{Context, NextAction, Task, TaskResult};

use super::state::{ReconStage, ReconState, STATE_KEY};

pub struct HttpProbeTask;

fn curl_head(target: &str) -> Option<(u16, String, String)> {
    let url = if target.contains("://") {
        target.to_string()
    } else {
        format!("https://{target}")
    };

    let output = std::process::Command::new("curl")
        .args(["-sI", "-L", "--connect-timeout", "5", "--max-time", "10", &url])
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let mut status = 0u16;
    let mut server = String::new();
    let mut location = String::new();

    for line in text.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("http/") {
            if let Some(code) = line.split_whitespace().nth(1) {
                status = code.parse().unwrap_or(0);
            }
        } else if lower.starts_with("server:") {
            server = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        } else if lower.starts_with("location:") {
            location = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        }
    }

    Some((status, server, location))
}

#[async_trait]
impl Task for HttpProbeTask {
    fn id(&self) -> &str {
        "http_probe"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut state: ReconState = context
            .get(STATE_KEY)
            .await
            .unwrap_or_default();

        state.stage = ReconStage::HttpProbe;

        if !state.tools.curl {
            state.errors.push("curl not available, skipping HTTP probe".into());
            context.set(STATE_KEY, state).await;
            return Ok(TaskResult::new(
                Some("Skipped: curl not available".into()),
                NextAction::Continue,
            ));
        }

        let mut probed = 0;
        for result in state.results.iter_mut() {
            if let Some((status, server, redirect)) = curl_head(&result.value) {
                result.http_status = Some(status);
                result.http_server = server;
                result.http_redirect = redirect;
                probed += 1;
                tracing::info!(
                    "[recon] HTTP {} → {} (server: {})",
                    result.value,
                    status,
                    result.http_server
                );
            }
        }

        context.set(STATE_KEY, state).await;
        Ok(TaskResult::new(
            Some(format!("HTTP probe complete: {probed} targets probed")),
            NextAction::Continue,
        ))
    }
}
