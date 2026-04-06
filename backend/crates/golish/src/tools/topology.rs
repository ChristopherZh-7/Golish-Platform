use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoNode {
    pub id: String,
    pub label: String,
    pub node_type: String, // "host", "network", "service", "gateway"
    pub ip: Option<String>,
    pub ports: Vec<TopoPort>,
    pub os: Option<String>,
    pub status: String, // "up", "down", "filtered"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoPort {
    pub port: u16,
    pub protocol: String,
    pub state: String,
    pub service: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoEdge {
    pub source: String,
    pub target: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyData {
    pub nodes: Vec<TopoNode>,
    pub edges: Vec<TopoEdge>,
    pub scan_info: Option<String>,
}

fn topo_dir(project_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return Ok(PathBuf::from(pp).join(".golish").join("topology"));
        }
    }
    let base = dirs::data_dir().ok_or("Cannot resolve data dir")?;
    Ok(base.join("golish-platform").join("topology"))
}

/// Parse nmap-style text output into topology data.
fn parse_nmap_output(raw: &str) -> TopologyData {
    let mut nodes: Vec<TopoNode> = Vec::new();
    let mut edges: Vec<TopoEdge> = Vec::new();
    let mut current_host: Option<TopoNode> = None;
    let mut scan_info: Option<String> = None;

    for line in raw.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("Starting Nmap") || trimmed.starts_with("Nmap scan report") {
            if let Some(host) = current_host.take() {
                nodes.push(host);
            }

            if trimmed.starts_with("Nmap scan report for") {
                let rest = trimmed.strip_prefix("Nmap scan report for ").unwrap_or("");
                let (label, ip) = if let Some(paren_start) = rest.find('(') {
                    let hostname = rest[..paren_start].trim();
                    let ip_str = rest[paren_start + 1..].trim_end_matches(')').to_string();
                    (hostname.to_string(), Some(ip_str))
                } else {
                    (rest.to_string(), Some(rest.to_string()))
                };

                let node_id = ip.clone().unwrap_or_else(|| label.clone());
                current_host = Some(TopoNode {
                    id: node_id,
                    label,
                    node_type: "host".to_string(),
                    ip,
                    ports: Vec::new(),
                    os: None,
                    status: "up".to_string(),
                });
            }

            if trimmed.starts_with("Starting Nmap") {
                scan_info = Some(trimmed.to_string());
            }
        }

        if let Some(ref mut host) = current_host {
            // Parse port lines like "80/tcp   open  http"
            if let Some(slash_pos) = trimmed.find('/') {
                if slash_pos < 6 {
                    if let Ok(port_num) = trimmed[..slash_pos].parse::<u16>() {
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if parts.len() >= 3 {
                            let protocol = parts[0].split('/').nth(1).unwrap_or("tcp").to_string();
                            let state = parts[1].to_string();
                            let service = parts[2].to_string();
                            host.ports.push(TopoPort {
                                port: port_num,
                                protocol,
                                state,
                                service,
                            });
                        }
                    }
                }
            }

            if trimmed.starts_with("OS details:") || trimmed.starts_with("Running:") {
                host.os = Some(trimmed.to_string());
            }
        }
    }

    if let Some(host) = current_host {
        nodes.push(host);
    }

    // Build edges: create a network node and connect all hosts to it
    if !nodes.is_empty() {
        let network_prefixes = derive_networks(&nodes);
        for (net_id, net_label) in &network_prefixes {
            let net_node = TopoNode {
                id: net_id.clone(),
                label: net_label.clone(),
                node_type: "network".to_string(),
                ip: None,
                ports: Vec::new(),
                os: None,
                status: "up".to_string(),
            };
            nodes.push(net_node);
        }

        for node in nodes.iter().filter(|n| n.node_type == "host") {
            if let Some(ref ip) = node.ip {
                let net = find_network(ip, &network_prefixes);
                edges.push(TopoEdge {
                    source: net,
                    target: node.id.clone(),
                    label: None,
                });
            }
        }
    }

    TopologyData {
        nodes,
        edges,
        scan_info,
    }
}

fn derive_networks(nodes: &[TopoNode]) -> Vec<(String, String)> {
    let mut prefixes: HashMap<String, u32> = HashMap::new();
    for node in nodes {
        if let Some(ref ip) = node.ip {
            let parts: Vec<&str> = ip.split('.').collect();
            if parts.len() >= 3 {
                let prefix = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
                *prefixes.entry(prefix).or_insert(0) += 1;
            }
        }
    }
    prefixes
        .into_iter()
        .map(|(prefix, _)| {
            let id = format!("net-{}", prefix);
            let label = format!("{}.0/24", prefix);
            (id, label)
        })
        .collect()
}

fn find_network(ip: &str, networks: &[(String, String)]) -> String {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() >= 3 {
        let prefix = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
        let net_id = format!("net-{}", prefix);
        if networks.iter().any(|(id, _)| id == &net_id) {
            return net_id;
        }
    }
    "net-unknown".to_string()
}

#[tauri::command]
pub async fn topo_parse(raw_output: String) -> Result<TopologyData, String> {
    Ok(parse_nmap_output(&raw_output))
}

#[tauri::command]
pub async fn topo_save(name: String, data: TopologyData, project_path: Option<String>) -> Result<(), String> {
    let dir = topo_dir(project_path.as_deref())?;
    fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", name));
    let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    fs::write(&path, json).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn topo_list(project_path: Option<String>) -> Result<Vec<String>, String> {
    let dir = topo_dir(project_path.as_deref())?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    let mut entries = fs::read_dir(&dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    Ok(names)
}

#[tauri::command]
pub async fn topo_load(name: String, project_path: Option<String>) -> Result<TopologyData, String> {
    let dir = topo_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", name));
    let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn topo_delete(name: String, project_path: Option<String>) -> Result<(), String> {
    let dir = topo_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", name));
    if path.exists() {
        fs::remove_file(&path).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
