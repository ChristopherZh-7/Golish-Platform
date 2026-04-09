use async_trait::async_trait;
use graph_flow::{Context, NextAction, Task, TaskResult};

use super::state::{PortInfo, ReconStage, ReconState, STATE_KEY};

pub struct PortScanTask;

fn parse_nmap_ports(output: &str) -> Vec<PortInfo> {
    let mut ports = Vec::new();
    let mut in_ports = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("PORT") {
            in_ports = true;
            continue;
        }
        if in_ports && trimmed.is_empty() {
            break;
        }
        if in_ports {
            // Format: "80/tcp  open  http  Apache httpd 2.4.41"
            let parts: Vec<&str> = trimmed.splitn(4, char::is_whitespace).collect();
            if parts.len() >= 3 {
                let port_proto: Vec<&str> = parts[0].split('/').collect();
                if port_proto.len() == 2 {
                    if let Ok(port) = port_proto[0].parse::<u16>() {
                        ports.push(PortInfo {
                            port,
                            protocol: port_proto[1].to_string(),
                            state: parts[1].trim().to_string(),
                            service: parts[2].trim().to_string(),
                            version: parts.get(3).unwrap_or(&"").trim().to_string(),
                        });
                    }
                }
            }
        }
    }
    ports
}

fn nc_check_port(host: &str, port: u16) -> bool {
    std::process::Command::new("nc")
        .args(["-z", "-w", "2", host, &port.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

static COMMON_PORTS: &[u16] = &[
    21, 22, 23, 25, 53, 80, 110, 111, 135, 139, 143, 443, 445, 993, 995,
    1723, 3306, 3389, 5432, 5900, 8080, 8443, 8888, 9090,
];

#[async_trait]
impl Task for PortScanTask {
    fn id(&self) -> &str {
        "port_scan"
    }

    async fn run(&self, context: Context) -> graph_flow::Result<TaskResult> {
        let mut state: ReconState = context
            .get(STATE_KEY)
            .await
            .unwrap_or_default();

        state.stage = ReconStage::PortScan;

        let mut total_ports = 0usize;

        for result in state.results.iter_mut() {
            let scan_target = result.ips.first().unwrap_or(&result.value);

            if state.tools.nmap {
                let output = std::process::Command::new("nmap")
                    .args(["-sC", "-sV", "-T4", "--top-ports", "100", "--open", scan_target])
                    .output();

                match output {
                    Ok(o) if o.status.success() => {
                        let text = String::from_utf8_lossy(&o.stdout).to_string();
                        result.ports = parse_nmap_ports(&text);
                        result.raw_outputs.push(("nmap".into(), text));
                    }
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                        state.errors.push(format!("nmap failed for {}: {}", result.value, stderr));
                    }
                    Err(e) => {
                        state.errors.push(format!("nmap error for {}: {}", result.value, e));
                    }
                }
            } else if state.tools.nc {
                for &port in COMMON_PORTS {
                    if nc_check_port(scan_target, port) {
                        result.ports.push(PortInfo {
                            port,
                            protocol: "tcp".into(),
                            state: "open".into(),
                            service: String::new(),
                            version: String::new(),
                        });
                    }
                }
            } else {
                state.errors.push(format!(
                    "No port scanner available for {} (need nmap or nc)",
                    result.value
                ));
            }

            total_ports += result.ports.len();
            tracing::info!(
                "[recon] Port scan {}: {} open ports",
                result.value,
                result.ports.len()
            );
        }

        context.set(STATE_KEY, state).await;
        Ok(TaskResult::new(
            Some(format!("Port scan complete: {total_ports} open ports found")),
            NextAction::Continue,
        ))
    }
}
