use async_trait::async_trait;
use graph_flow::{Context, NextAction, Task, TaskResult};

use super::state::{ReconStage, ReconState, TargetReconData, STATE_KEY};

pub struct DnsLookupTask;

fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

fn parse_dig_ips(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with(';'))
        .map(|l| l.trim().to_string())
        .filter(|l| {
            l.parse::<std::net::Ipv4Addr>().is_ok()
                || l.parse::<std::net::Ipv6Addr>().is_ok()
        })
        .collect()
}

#[async_trait]
impl Task for DnsLookupTask {
    fn id(&self) -> &str {
        "dns_lookup"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut state: ReconState = context
            .get(STATE_KEY)
            .await
            .unwrap_or_default();

        state.stage = ReconStage::DnsLookup;
        state.results.clear();

        for target in &state.targets {
            let mut data = TargetReconData {
                value: target.clone(),
                ..Default::default()
            };

            if target.parse::<std::net::Ipv4Addr>().is_ok() {
                data.ips.push(target.clone());
            } else if state.tools.dig {
                if let Some(output) = run_cmd("dig", &["+short", target]) {
                    data.ips = parse_dig_ips(&output);
                    data.raw_outputs.push(("dig".into(), output));
                }
            } else if let Some(output) = run_cmd("nslookup", &[target]) {
                for line in output.lines() {
                    let line = line.trim();
                    if line.starts_with("Address:") && !line.contains('#') {
                        if let Some(ip) = line.strip_prefix("Address:") {
                            let ip = ip.trim();
                            if ip.parse::<std::net::Ipv4Addr>().is_ok() {
                                data.ips.push(ip.to_string());
                            }
                        }
                    }
                }
                data.raw_outputs.push(("nslookup".into(), output));
            }

            tracing::info!(
                "[recon] DNS for {}: {} IPs found",
                target,
                data.ips.len()
            );
            state.results.push(data);
        }

        let total_ips: usize = state.results.iter().map(|r| r.ips.len()).sum();
        context.set(STATE_KEY, state).await;
        Ok(TaskResult::new(
            Some(format!("DNS lookup complete: {} IPs resolved", total_ips)),
            NextAction::Continue,
        ))
    }
}
