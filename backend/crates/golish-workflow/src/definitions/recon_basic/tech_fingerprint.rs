use async_trait::async_trait;
use graph_flow::{Context, NextAction, Task, TaskResult};

use super::state::{ReconStage, ReconState, STATE_KEY};

pub struct TechFingerprintTask;

fn extract_tech_from_headers(server: &str) -> Vec<String> {
    let mut techs = Vec::new();
    let lower = server.to_lowercase();

    let patterns = [
        ("apache", "Apache"),
        ("nginx", "Nginx"),
        ("iis", "IIS"),
        ("cloudflare", "Cloudflare"),
        ("express", "Express.js"),
        ("openresty", "OpenResty"),
        ("litespeed", "LiteSpeed"),
        ("caddy", "Caddy"),
        ("tomcat", "Tomcat"),
        ("jetty", "Jetty"),
        ("gunicorn", "Gunicorn"),
        ("uvicorn", "Uvicorn"),
        ("php", "PHP"),
        ("asp.net", "ASP.NET"),
        ("werkzeug", "Flask/Werkzeug"),
    ];

    for (pattern, name) in patterns {
        if lower.contains(pattern) {
            techs.push(name.to_string());
        }
    }

    techs
}

fn whatweb_technologies(target: &str) -> Vec<String> {
    let url = if target.contains("://") {
        target.to_string()
    } else {
        format!("https://{target}")
    };

    let output = std::process::Command::new("whatweb")
        .args(["--color=never", "-q", &url])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            text.split(',')
                .filter_map(|part| {
                    let part = part.trim();
                    if part.is_empty() || part.starts_with("http") {
                        None
                    } else {
                        let name = part.split('[').next().unwrap_or(part).trim();
                        if name.is_empty() { None } else { Some(name.to_string()) }
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

#[async_trait]
impl Task for TechFingerprintTask {
    fn id(&self) -> &str {
        "tech_fingerprint"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut state: ReconState = context
            .get(STATE_KEY)
            .await
            .unwrap_or_default();

        state.stage = ReconStage::TechFingerprint;

        let mut total_techs = 0usize;

        for result in state.results.iter_mut() {
            if state.tools.whatweb {
                let techs = whatweb_technologies(&result.value);
                result.technologies.extend(techs);
            }

            if !result.http_server.is_empty() {
                let header_techs = extract_tech_from_headers(&result.http_server);
                for tech in header_techs {
                    if !result.technologies.contains(&tech) {
                        result.technologies.push(tech);
                    }
                }
            }

            total_techs += result.technologies.len();
            tracing::info!(
                "[recon] Tech fingerprint {}: {:?}",
                result.value,
                result.technologies
            );
        }

        context.set(STATE_KEY, state).await;
        Ok(TaskResult::new(
            Some(format!("Tech fingerprint complete: {total_techs} technologies identified")),
            NextAction::Continue,
        ))
    }
}
