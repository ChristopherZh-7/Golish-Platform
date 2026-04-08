use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::db::open_db;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoNode {
    pub id: String,
    pub label: String,
    pub node_type: String,
    pub ip: Option<String>,
    pub ports: Vec<TopoPort>,
    pub os: Option<String>,
    pub status: String,
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

fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn parse_nmap_output(raw: &str) -> TopologyData {
    let clean = strip_ansi(raw);
    let mut nodes: Vec<TopoNode> = Vec::new();
    let mut edges: Vec<TopoEdge> = Vec::new();
    let mut current_host: Option<TopoNode> = None;
    let mut scan_info: Option<String> = None;

    for line in clean.lines() {
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
                    id: node_id, label, node_type: "host".to_string(),
                    ip, ports: Vec::new(), os: None, status: "up".to_string(),
                });
            }
            if trimmed.starts_with("Starting Nmap") { scan_info = Some(trimmed.to_string()); }
        }

        if let Some(ref mut host) = current_host {
            if let Some(slash_pos) = trimmed.find('/') {
                if slash_pos < 6 {
                    if let Ok(port_num) = trimmed[..slash_pos].parse::<u16>() {
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if parts.len() >= 3 {
                            host.ports.push(TopoPort {
                                port: port_num,
                                protocol: parts[0].split('/').nth(1).unwrap_or("tcp").to_string(),
                                state: parts[1].to_string(),
                                service: parts[2].to_string(),
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

    if let Some(host) = current_host { nodes.push(host); }

    if !nodes.is_empty() {
        let network_prefixes = derive_networks(&nodes);
        for (net_id, net_label) in &network_prefixes {
            nodes.push(TopoNode {
                id: net_id.clone(), label: net_label.clone(), node_type: "network".to_string(),
                ip: None, ports: Vec::new(), os: None, status: "up".to_string(),
            });
        }
        for node in nodes.iter().filter(|n| n.node_type == "host") {
            if let Some(ref ip) = node.ip {
                edges.push(TopoEdge { source: find_network(ip, &network_prefixes), target: node.id.clone(), label: None });
            }
        }
    }

    TopologyData { nodes, edges, scan_info }
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
    prefixes.into_iter().map(|(prefix, _)| (format!("net-{}", prefix), format!("{}.0/24", prefix))).collect()
}

fn find_network(ip: &str, networks: &[(String, String)]) -> String {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() >= 3 {
        let net_id = format!("net-{}.{}.{}", parts[0], parts[1], parts[2]);
        if networks.iter().any(|(id, _)| id == &net_id) { return net_id; }
    }
    "net-unknown".to_string()
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub async fn topo_parse(raw_output: String) -> Result<TopologyData, String> {
    Ok(parse_nmap_output(&raw_output))
}

#[tauri::command]
pub async fn topo_save(name: String, data: TopologyData, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let json = serde_json::to_string(&data).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO topology_scans (name, data, created_at) VALUES (?1,?2,?3)",
            params![name, json, now_ts()],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn topo_list(project_path: Option<String>) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let mut stmt = conn.prepare("SELECT name FROM topology_scans ORDER BY created_at DESC").map_err(|e| e.to_string())?;
        let names: Vec<String> = stmt.query_map([], |row| row.get(0)).map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();
        Ok(names)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn topo_load(name: String, project_path: Option<String>) -> Result<TopologyData, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let json: String = conn.query_row("SELECT data FROM topology_scans WHERE name=?1", params![name], |r| r.get(0)).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn topo_delete(name: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM topology_scans WHERE name=?1", params![name]).map_err(|e| e.to_string())?;
        Ok(())
    }).await.map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopoDiff {
    pub new_hosts: Vec<String>,
    pub removed_hosts: Vec<String>,
    pub new_ports: Vec<String>,
    pub removed_ports: Vec<String>,
    pub changed_services: Vec<String>,
}

#[tauri::command]
pub async fn topo_diff(name_a: String, name_b: String, project_path: Option<String>) -> Result<TopoDiff, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;

        let json_a: String = conn.query_row("SELECT data FROM topology_scans WHERE name=?1", params![name_a], |r| r.get(0)).map_err(|e| e.to_string())?;
        let json_b: String = conn.query_row("SELECT data FROM topology_scans WHERE name=?1", params![name_b], |r| r.get(0)).map_err(|e| e.to_string())?;
        let data_a: TopologyData = serde_json::from_str(&json_a).map_err(|e| e.to_string())?;
        let data_b: TopologyData = serde_json::from_str(&json_b).map_err(|e| e.to_string())?;

        let hosts_a: HashMap<String, &TopoNode> = data_a.nodes.iter().filter(|n| n.node_type == "host").map(|n| (n.ip.clone().unwrap_or_else(|| n.label.clone()), n)).collect();
        let hosts_b: HashMap<String, &TopoNode> = data_b.nodes.iter().filter(|n| n.node_type == "host").map(|n| (n.ip.clone().unwrap_or_else(|| n.label.clone()), n)).collect();

        let new_hosts: Vec<String> = hosts_b.keys().filter(|h| !hosts_a.contains_key(*h)).cloned().collect();
        let removed_hosts: Vec<String> = hosts_a.keys().filter(|h| !hosts_b.contains_key(*h)).cloned().collect();
        let mut new_ports = Vec::new();
        let mut removed_ports = Vec::new();
        let mut changed_services = Vec::new();

        for (host, node_b) in &hosts_b {
            if let Some(node_a) = hosts_a.get(host) {
                let ports_a: HashMap<(u16, &str), &str> = node_a.ports.iter().map(|p| ((p.port, p.protocol.as_str()), p.service.as_str())).collect();
                let ports_b: HashMap<(u16, &str), &str> = node_b.ports.iter().map(|p| ((p.port, p.protocol.as_str()), p.service.as_str())).collect();
                for ((port, proto), svc_b) in &ports_b {
                    if let Some(svc_a) = ports_a.get(&(*port, *proto)) {
                        if svc_a != svc_b { changed_services.push(format!("{}:{}/{} {} → {}", host, port, proto, svc_a, svc_b)); }
                    } else { new_ports.push(format!("{}:{}/{}", host, port, proto)); }
                }
                for (port, proto) in ports_a.keys() {
                    if !ports_b.contains_key(&(*port, *proto)) { removed_ports.push(format!("{}:{}/{}", host, port, proto)); }
                }
            }
        }

        Ok(TopoDiff { new_hosts, removed_hosts, new_ports, removed_ports, changed_services })
    }).await.map_err(|e| e.to_string())?
}
