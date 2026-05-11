use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use crate::state::DbState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodologyTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub items: Vec<CheckItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub checked: bool,
    pub notes: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMethodology {
    pub id: String,
    pub template_id: String,
    pub template_name: String,
    pub project_name: String,
    pub phases: Vec<Phase>,
    pub created_at: String,
    pub updated_at: String,
}

fn templates_dir() -> Result<PathBuf, GolishError> {
    let base =
        dirs::data_dir().ok_or_else(|| GolishError::Internal("Cannot resolve data dir".into()))?;
    Ok(base
        .join("golish-platform")
        .join("methodology")
        .join("templates"))
}

fn built_in_templates() -> Vec<MethodologyTemplate> {
    vec![
        MethodologyTemplate {
            id: "owasp-wstg".to_string(),
            name: "OWASP WSTG".to_string(),
            description: "OWASP Web Security Testing Guide — systematic methodology for web application security testing"
                .to_string(),
            phases: vec![
                Phase {
                    id: "info-gathering".to_string(),
                    name: "Information Gathering".to_string(),
                    description: "Collect the target's technical architecture, entry points, and attack surface".to_string(),
                    items: vec![
                        check(
                            "wstg-info-01",
                            "Search-Engine Reconnaissance",
                            "Use search engines to discover information leaks about the target",
                            &["subfinder", "amass"],
                        ),
                        check(
                            "wstg-info-02",
                            "Web Server Fingerprinting",
                            "Identify the web server type and version",
                            &["whatweb", "httpx"],
                        ),
                        check(
                            "wstg-info-03",
                            "Web Application Framework Fingerprinting",
                            "Identify the backend framework and tech stack",
                            &["whatweb"],
                        ),
                        check(
                            "wstg-info-04",
                            "Enumerate Web Application Entry Points",
                            "Enumerate all entry points and parameters of the application",
                            &["katana", "ffuf"],
                        ),
                        check(
                            "wstg-info-05",
                            "Web-page comments and metadata",
                            "Check for HTML comments and metadata leakage",
                            &[],
                        ),
                        check(
                            "wstg-info-06",
                            "Application Entry-Point Identification",
                            "Map all HTTP endpoints and parameters",
                            &["katana"],
                        ),
                        check("wstg-info-07", "Map Execution Paths", "Understand the application's execution flow", &[]),
                        check(
                            "wstg-info-08",
                            "Fingerprint Web Application Framework",
                            "Deep-analyze framework characteristics",
                            &["whatweb"],
                        ),
                        check(
                            "wstg-info-09",
                            "Map Application Architecture",
                            "Understand the network topology and infrastructure",
                            &["nmap"],
                        ),
                    ],
                },
                Phase {
                    id: "config-mgmt".to_string(),
                    name: "Configuration and Deployment Management Testing".to_string(),
                    description: "Test security weaknesses in application and infrastructure configuration".to_string(),
                    items: vec![
                        check(
                            "wstg-conf-01",
                            "Network Infrastructure Configuration",
                            "Test network-layer security configuration",
                            &["nmap"],
                        ),
                        check(
                            "wstg-conf-02",
                            "Application Platform Configuration",
                            "Review application server configuration",
                            &["nikto"],
                        ),
                        check(
                            "wstg-conf-03",
                            "File Extension Handling",
                            "Test handling of sensitive file extensions",
                            &["ffuf"],
                        ),
                        check(
                            "wstg-conf-04",
                            "Backup File Discovery",
                            "Search for old backups and temporary files",
                            &["ffuf", "gobuster"],
                        ),
                        check(
                            "wstg-conf-05",
                            "Enumerate Administrative Interfaces",
                            "Discover admin consoles and interfaces",
                            &["ffuf", "gobuster"],
                        ),
                        check(
                            "wstg-conf-06",
                            "HTTP Method Testing",
                            "Test allowed HTTP methods",
                            &["httpx"],
                        ),
                        check(
                            "wstg-conf-07",
                            "HTTP Strict Transport Security",
                            "Verify HSTS configuration",
                            &["httpx"],
                        ),
                        check("wstg-conf-08", "Cross-Origin Policy", "Review CORS and cross-origin configuration", &[]),
                        check("wstg-conf-09", "File Permission Testing", "Check sensitive file permissions", &[]),
                        check(
                            "wstg-conf-10",
                            "Subdomain Enumeration",
                            "Enumerate all related subdomains",
                            &["subfinder", "amass", "dnsx"],
                        ),
                    ],
                },
                Phase {
                    id: "identity-mgmt".to_string(),
                    name: "Identity Management Testing".to_string(),
                    description: "Test the security of authentication and session management".to_string(),
                    items: vec![
                        check(
                            "wstg-idnt-01",
                            "Role Definitions Testing",
                            "Review user roles and permission definitions",
                            &[],
                        ),
                        check(
                            "wstg-idnt-02",
                            "User Registration Process",
                            "Test security issues in the registration process",
                            &[],
                        ),
                        check("wstg-idnt-03", "Account Provisioning Process", "Review account provisioning and configuration", &[]),
                        check(
                            "wstg-idnt-04",
                            "Username Enumeration",
                            "Test whether valid usernames can be enumerated",
                            &[],
                        ),
                        check("wstg-authn-01", "Transport-Layer Encryption", "Verify the transport security of authentication credentials", &[]),
                        check("wstg-authn-02", "Default Credentials Testing", "Test for default credentials", &[]),
                        check(
                            "wstg-authn-03",
                            "Lockout Mechanism",
                            "Test account lockout and brute-force protections",
                            &[],
                        ),
                        check("wstg-authn-04", "Authentication Bypass Testing", "Attempt to bypass authentication", &[]),
                        check("wstg-authn-05", "Password Recovery Testing", "Test the security of password-reset flows", &[]),
                    ],
                },
                Phase {
                    id: "injection".to_string(),
                    name: "Injection Testing".to_string(),
                    description: "Test for various injection vulnerabilities".to_string(),
                    items: vec![
                        check(
                            "wstg-inpv-01",
                            "Reflected XSS",
                            "Test for reflected cross-site scripting",
                            &["XSStrike", "nuclei"],
                        ),
                        check(
                            "wstg-inpv-02",
                            "Stored XSS",
                            "Test for stored cross-site scripting",
                            &["XSStrike"],
                        ),
                        check("wstg-inpv-03", "HTTP Parameter Tampering", "Test HTTP parameter manipulation", &[]),
                        check("wstg-inpv-05", "SQL Injection", "Test for SQL injection vulnerabilities", &["nuclei"]),
                        check("wstg-inpv-06", "LDAP Injection", "Test for LDAP injection vulnerabilities", &[]),
                        check("wstg-inpv-07", "XML Injection", "Test for XML injection and XXE", &["nuclei"]),
                        check("wstg-inpv-08", "Server-Side Includes Injection", "Test for server-side includes injection", &[]),
                        check("wstg-inpv-09", "XPath Injection", "Test for XPath injection", &[]),
                        check(
                            "wstg-inpv-11",
                            "Code Injection",
                            "Test for server-side code injection",
                            &["nuclei"],
                        ),
                        check(
                            "wstg-inpv-12",
                            "Command Injection",
                            "Test for OS command injection",
                            &["nuclei"],
                        ),
                        check(
                            "wstg-inpv-13",
                            "Template Injection",
                            "Test for server-side template injection (SSTI)",
                            &["nuclei"],
                        ),
                        check("wstg-inpv-14", "SSRF", "Test for server-side request forgery", &["nuclei"]),
                    ],
                },
                Phase {
                    id: "business-logic".to_string(),
                    name: "Business Logic Testing".to_string(),
                    description: "Test application-specific business-logic flaws".to_string(),
                    items: vec![
                        check(
                            "wstg-busl-01",
                            "Data Validation Testing",
                            "Test input validation and data integrity",
                            &[],
                        ),
                        check("wstg-busl-02", "Request Forgery", "Test whether request parameters can be tampered with", &[]),
                        check(
                            "wstg-busl-03",
                            "Integrity Checks",
                            "Verify the application's integrity-check mechanism",
                            &[],
                        ),
                        check("wstg-busl-04", "Process Timing Testing", "Test race conditions and timing attacks", &[]),
                        check("wstg-busl-05", "Process Timing / Use Limits", "Test functional use-count limits", &[]),
                        check(
                            "wstg-busl-06",
                            "Workflow Bypass Testing",
                            "Test whether workflows can be bypassed",
                            &[],
                        ),
                        check(
                            "wstg-busl-07",
                            "Defenses Against Application Misuse",
                            "Test the application's defense against abnormal usage",
                            &[],
                        ),
                        check("wstg-busl-08", "File Upload Testing", "Test the security of file-upload features", &[]),
                    ],
                },
            ],
        },
        MethodologyTemplate {
            id: "ptes".to_string(),
            name: "PTES".to_string(),
            description: "Penetration Testing Execution Standard".to_string(),
            phases: vec![
                Phase {
                    id: "ptes-intel".to_string(),
                    name: "Intelligence Gathering".to_string(),
                    description: "Passive and active information gathering".to_string(),
                    items: vec![
                        check(
                            "ptes-intel-01",
                            "OSINT Collection",
                            "Open-source intelligence collection and analysis",
                            &["subfinder", "amass"],
                        ),
                        check(
                            "ptes-intel-02",
                            "DNS Reconnaissance",
                            "DNS record lookups and zone transfers",
                            &["dnsx", "subfinder"],
                        ),
                        check(
                            "ptes-intel-03",
                            "Port Scanning",
                            "TCP/UDP port scanning",
                            &["nmap", "rustscan", "masscan"],
                        ),
                        check("ptes-intel-04", "Service Enumeration", "Identify running services and versions", &["nmap"]),
                        check("ptes-intel-05", "Operating System Fingerprinting", "Remote OS Detection", &["nmap"]),
                        check(
                            "ptes-intel-06",
                            "Web Application Reconnaissance",
                            "Discover web application entry points",
                            &["httpx", "katana", "whatweb"],
                        ),
                    ],
                },
                Phase {
                    id: "ptes-vuln".to_string(),
                    name: "Vulnerability Analysis".to_string(),
                    description: "Vulnerability identification and verification".to_string(),
                    items: vec![
                        check(
                            "ptes-vuln-01",
                            "Automated Scanning",
                            "Detect vulnerabilities with security scanners",
                            &["nuclei", "nikto"],
                        ),
                        check("ptes-vuln-02", "Manual Verification", "Manually verify vulnerabilities discovered by the scanner", &[]),
                        check("ptes-vuln-03", "CVE Research", "Look up known CVEs and public exploits", &[]),
                        check("ptes-vuln-04", "Configuration Audit", "Review security configuration", &[]),
                    ],
                },
                Phase {
                    id: "ptes-exploit".to_string(),
                    name: "Exploitation".to_string(),
                    description: "Attempt to exploit discovered vulnerabilities".to_string(),
                    items: vec![
                        check(
                            "ptes-exploit-01",
                            "Known Exploit Usage",
                            "Validate vulnerabilities using public exploits",
                            &["metasploit"],
                        ),
                        check(
                            "ptes-exploit-02",
                            "Password Attacks",
                            "Brute-force and dictionary attacks",
                            &["john"],
                        ),
                        check(
                            "ptes-exploit-03",
                            "Web Application Attacks",
                            "Gain access through web vulnerabilities",
                            &["XSStrike"],
                        ),
                        check(
                            "ptes-exploit-04",
                            "Network Attacks",
                            "Network-layer attacks and man-in-the-middle",
                            &["chisel"],
                        ),
                    ],
                },
                Phase {
                    id: "ptes-post".to_string(),
                    name: "Post-Exploitation".to_string(),
                    description: "Post-exploitation actions after gaining initial access".to_string(),
                    items: vec![
                        check("ptes-post-01", "Privilege Escalation", "Attempt to escalate system privileges", &[]),
                        check("ptes-post-02", "Persistence", "Establish persistent access channels", &["chisel"]),
                        check("ptes-post-03", "Data Collection", "Collect sensitive data and credentials", &[]),
                        check("ptes-post-04", "Lateral Movement", "Lateral movement within the network", &[]),
                        check("ptes-post-05", "Trace Cleanup", "Clean up testing traces", &[]),
                    ],
                },
                Phase {
                    id: "ptes-report".to_string(),
                    name: "Reporting".to_string(),
                    description: "Write the testing report".to_string(),
                    items: vec![
                        check("ptes-report-01", "Executive Summary", "Write the executive summary for management", &[]),
                        check("ptes-report-02", "Technical Findings", "Document each finding in detail", &[]),
                        check("ptes-report-03", "Risk Rating", "Assign a risk severity to every finding", &[]),
                        check("ptes-report-04", "Remediation Recommendations", "Provide concrete remediation guidance", &[]),
                    ],
                },
            ],
        },
    ]
}

fn check(id: &str, title: &str, desc: &str, tools: &[&str]) -> CheckItem {
    CheckItem {
        id: id.to_string(),
        title: title.to_string(),
        description: desc.to_string(),
        checked: false,
        notes: String::new(),
        tools: tools.iter().map(|s| s.to_string()).collect(),
    }
}

#[tauri::command]
pub async fn method_list_templates() -> Result<Vec<MethodologyTemplate>, GolishError> {
    let mut templates = built_in_templates();
    let custom_dir = templates_dir()?;
    if custom_dir.exists() {
        let mut entries = fs::read_dir(&custom_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().is_some_and(|e| e == "json") {
                if let Ok(content) = fs::read_to_string(entry.path()).await {
                    if let Ok(t) = serde_json::from_str::<MethodologyTemplate>(&content) {
                        templates.push(t);
                    }
                }
            }
        }
    }
    Ok(templates)
}

#[tauri::command]
pub async fn method_start_project(
    state: tauri::State<'_, DbState>,
    template_id: String,
    project_name: String,
    project_path: Option<String>,
) -> Result<ProjectMethodology, GolishError> {
    let pool = state.pool_ready().await?;
    let templates = built_in_templates();
    let template = templates
        .iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| GolishError::Internal("Template not found".into()))?;
    let now = chrono::Utc::now().to_rfc3339();
    let project = ProjectMethodology {
        id: Uuid::new_v4().to_string(),
        template_id: template.id.clone(),
        template_name: template.name.clone(),
        project_name,
        phases: template.phases.clone(),
        created_at: now.clone(),
        updated_at: now,
    };
    let data = serde_json::to_value(&project)?;
    let uid: Uuid = project.id.parse().unwrap_or_else(|_| Uuid::new_v4());
    sqlx::query("INSERT INTO methodology_projects (id, data, project_path) VALUES ($1, $2, $3)")
        .bind(uid)
        .bind(&data)
        .bind(project_path.as_deref())
        .execute(pool)
        .await?;
    Ok(project)
}

#[tauri::command]
pub async fn method_list_projects(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<ProjectMethodology>, GolishError> {
    let pool = state.pool_ready().await?;
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM methodology_projects WHERE project_path = $1 ORDER BY updated_at DESC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await?;

    let projects: Vec<ProjectMethodology> = rows
        .into_iter()
        .filter_map(|j| serde_json::from_value(j).ok())
        .collect();
    Ok(projects)
}

#[tauri::command]
pub async fn method_load_project(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<ProjectMethodology, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let data: serde_json::Value =
        sqlx::query_scalar("SELECT data FROM methodology_projects WHERE id=$1")
            .bind(uid)
            .fetch_one(pool)
            .await?;
    serde_json::from_value(data).map_err(GolishError::from)
}

#[tauri::command]
pub async fn method_update_item(
    state: tauri::State<'_, DbState>,
    project_id: String,
    phase_id: String,
    item_id: String,
    checked: Option<bool>,
    notes: Option<String>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = project_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let data: serde_json::Value =
        sqlx::query_scalar("SELECT data FROM methodology_projects WHERE id=$1")
            .bind(uid)
            .fetch_one(pool)
            .await?;

    let mut project: ProjectMethodology = serde_json::from_value(data)?;

    for phase in &mut project.phases {
        if phase.id == phase_id {
            for item in &mut phase.items {
                if item.id == item_id {
                    if let Some(c) = checked {
                        item.checked = c;
                    }
                    if let Some(ref n) = notes {
                        item.notes = n.clone();
                    }
                }
            }
        }
    }
    project.updated_at = chrono::Utc::now().to_rfc3339();
    let new_data = serde_json::to_value(&project)?;

    sqlx::query("UPDATE methodology_projects SET data=$1, updated_at=NOW() WHERE id=$2")
        .bind(&new_data)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn method_delete_project(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM methodology_projects WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}
