use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

fn evidence_dir(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("evidence");
        }
    }
    let home = dirs::home_dir().expect("cannot resolve home directory");
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    base.join("evidence")
}

fn findings_path(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("findings.json");
        }
    }
    let home = dirs::home_dir().expect("cannot resolve home directory");
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform");
    #[cfg(target_os = "windows")]
    let base = home.join("AppData").join("Local").join("golish-platform");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform");
    base.join("findings.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    #[serde(default)]
    pub cvss: Option<f64>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: String,
    #[serde(default)]
    pub remediation: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    pub status: FindingStatus,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub caption: String,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    Open,
    Confirmed,
    FalsePositive,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FindingsStore {
    #[serde(default)]
    pub findings: Vec<Finding>,
}

async fn load_store(project_path: Option<&str>) -> FindingsStore {
    let path = findings_path(project_path);
    match fs::read_to_string(&path).await {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => FindingsStore::default(),
    }
}

async fn save_store(store: &FindingsStore, project_path: Option<&str>) -> Result<(), String> {
    let path = findings_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, json).await.map_err(|e| e.to_string())
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub async fn findings_list(project_path: Option<String>) -> Result<FindingsStore, String> {
    Ok(load_store(project_path.as_deref()).await)
}

#[tauri::command]
pub async fn findings_add(finding: Finding, project_path: Option<String>) -> Result<String, String> {
    let mut store = load_store(project_path.as_deref()).await;
    let id = if finding.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        finding.id.clone()
    };
    let ts = now_ts();
    let entry = Finding {
        id: id.clone(),
        created_at: ts,
        updated_at: ts,
        ..finding
    };
    store.findings.push(entry);
    save_store(&store, project_path.as_deref()).await?;
    Ok(id)
}

#[tauri::command]
pub async fn findings_update(finding: Finding, project_path: Option<String>) -> Result<(), String> {
    let mut store = load_store(project_path.as_deref()).await;
    if let Some(existing) = store.findings.iter_mut().find(|f| f.id == finding.id) {
        let created = existing.created_at;
        *existing = Finding {
            updated_at: now_ts(),
            created_at: created,
            ..finding
        };
        save_store(&store, project_path.as_deref()).await?;
        Ok(())
    } else {
        Err("Finding not found".to_string())
    }
}

#[tauri::command]
pub async fn findings_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    let mut store = load_store(project_path.as_deref()).await;
    store.findings.retain(|f| f.id != id);
    save_store(&store, project_path.as_deref()).await
}

/// Batch-import parsed output items as findings.
/// Deduplicates by (title + url) to avoid creating duplicate entries.
#[tauri::command]
pub async fn findings_import_parsed(
    items: Vec<std::collections::HashMap<String, String>>,
    tool_name: Option<String>,
    project_path: Option<String>,
) -> Result<u32, String> {
    let mut store = load_store(project_path.as_deref()).await;
    let ts = now_ts();
    let tool = tool_name.unwrap_or_default();
    let mut added = 0u32;

    for item in items {
        let title = item.get("title").cloned().unwrap_or_default();
        let url = item.get("url").cloned().unwrap_or_default();
        if title.is_empty() && url.is_empty() {
            continue;
        }
        let already_exists = store
            .findings
            .iter()
            .any(|f| f.title == title && f.url == url);
        if already_exists {
            continue;
        }

        let severity = match item.get("severity").map(|s| s.to_lowercase()).as_deref() {
            Some("critical") => Severity::Critical,
            Some("high") => Severity::High,
            Some("medium") => Severity::Medium,
            Some("low") => Severity::Low,
            _ => Severity::Info,
        };

        store.findings.push(Finding {
            id: Uuid::new_v4().to_string(),
            title,
            severity,
            cvss: item.get("cvss").and_then(|v| v.parse().ok()),
            url,
            description: item.get("description").cloned().unwrap_or_default(),
            steps: String::new(),
            remediation: String::new(),
            tags: Vec::new(),
            tool: tool.clone(),
            template: item.get("template").cloned().unwrap_or_default(),
            references: item
                .get("reference")
                .map(|r| r.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            evidence: Vec::new(),
            status: FindingStatus::Open,
            created_at: ts,
            updated_at: ts,
        });
        added += 1;
    }

    save_store(&store, project_path.as_deref()).await?;
    Ok(added)
}

/// Save evidence file (base64-encoded) and attach it to a finding.
#[tauri::command]
pub async fn findings_add_evidence(
    finding_id: String,
    filename: String,
    mime_type: String,
    caption: String,
    data_base64: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let mut store = load_store(project_path.as_deref()).await;
    let finding = store
        .findings
        .iter_mut()
        .find(|f| f.id == finding_id)
        .ok_or_else(|| "Finding not found".to_string())?;

    let evidence_id = Uuid::new_v4().to_string();
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("bin");
    let stored_name = format!("{}.{}", evidence_id, ext);

    let dir = evidence_dir(project_path.as_deref()).join(&finding_id);
    fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("Invalid base64: {}", e))?;
    fs::write(dir.join(&stored_name), &bytes)
        .await
        .map_err(|e| e.to_string())?;

    finding.evidence.push(Evidence {
        id: evidence_id.clone(),
        filename: stored_name,
        mime_type,
        caption,
        added_at: now_ts(),
    });
    finding.updated_at = now_ts();

    save_store(&store, project_path.as_deref()).await?;
    Ok(evidence_id)
}

/// Remove evidence from a finding and delete the file.
#[tauri::command]
pub async fn findings_remove_evidence(
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let mut store = load_store(project_path.as_deref()).await;
    let finding = store
        .findings
        .iter_mut()
        .find(|f| f.id == finding_id)
        .ok_or_else(|| "Finding not found".to_string())?;

    if let Some(ev) = finding.evidence.iter().find(|e| e.id == evidence_id) {
        let file = evidence_dir(project_path.as_deref())
            .join(&finding_id)
            .join(&ev.filename);
        let _ = fs::remove_file(&file).await;
    }

    finding.evidence.retain(|e| e.id != evidence_id);
    finding.updated_at = now_ts();
    save_store(&store, project_path.as_deref()).await
}

/// Get the filesystem path for an evidence file (for frontend to load via convertFileSrc).
#[tauri::command]
pub async fn findings_evidence_path(
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let store = load_store(project_path.as_deref()).await;
    let finding = store
        .findings
        .iter()
        .find(|f| f.id == finding_id)
        .ok_or_else(|| "Finding not found".to_string())?;
    let ev = finding
        .evidence
        .iter()
        .find(|e| e.id == evidence_id)
        .ok_or_else(|| "Evidence not found".to_string())?;

    let path = evidence_dir(project_path.as_deref())
        .join(&finding_id)
        .join(&ev.filename);
    Ok(path.to_string_lossy().to_string())
}
