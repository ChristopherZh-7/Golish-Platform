use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

use super::db::open_db;

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
    pub target: String,
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

impl Severity {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    Open,
    Confirmed,
    FalsePositive,
    Resolved,
}

impl FindingStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Confirmed => "confirmed",
            Self::FalsePositive => "falsepositif",
            Self::Resolved => "resolved",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "confirmed" => Self::Confirmed,
            "falsepositif" | "falsepositive" => Self::FalsePositive,
            "resolved" => Self::Resolved,
            _ => Self::Open,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FindingsStore {
    #[serde(default)]
    pub findings: Vec<Finding>,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn row_to_finding(row: &rusqlite::Row) -> rusqlite::Result<Finding> {
    let tags_json: String = row.get(9)?;
    let refs_json: String = row.get(12)?;
    let evidence_json: String = row.get(13)?;
    let sev_str: String = row.get(2)?;
    let status_str: String = row.get(14)?;
    Ok(Finding {
        id: row.get(0)?,
        title: row.get(1)?,
        severity: Severity::from_str(&sev_str),
        cvss: row.get(3)?,
        url: row.get(4)?,
        target: row.get(5)?,
        description: row.get(6)?,
        steps: row.get(7)?,
        remediation: row.get(8)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        tool: row.get(10)?,
        template: row.get(11)?,
        references: serde_json::from_str(&refs_json).unwrap_or_default(),
        evidence: serde_json::from_str(&evidence_json).unwrap_or_default(),
        status: FindingStatus::from_str(&status_str),
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

const SELECT_COLS: &str = "id, title, severity, cvss, url, target, description, steps, remediation, tags, tool, template, refs, evidence, status, created_at, updated_at";

#[tauri::command]
pub async fn findings_list(project_path: Option<String>) -> Result<FindingsStore, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let sql = format!("SELECT {} FROM findings ORDER BY created_at DESC", SELECT_COLS);
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let findings: Vec<Finding> = stmt
            .query_map([], |row| row_to_finding(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(FindingsStore { findings })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn insert_finding(conn: &rusqlite::Connection, f: &Finding) -> Result<(), String> {
    let tags_json = serde_json::to_string(&f.tags).unwrap_or_else(|_| "[]".to_string());
    let refs_json = serde_json::to_string(&f.references).unwrap_or_else(|_| "[]".to_string());
    let evidence_json = serde_json::to_string(&f.evidence).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT OR REPLACE INTO findings (id, title, severity, cvss, url, target, description, steps, remediation, tags, tool, template, refs, evidence, status, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            f.id, f.title, f.severity.as_str(), f.cvss, f.url, f.target,
            f.description, f.steps, f.remediation, tags_json,
            f.tool, f.template, refs_json, evidence_json,
            f.status.as_str(), f.created_at, f.updated_at
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn findings_add(finding: Finding, project_path: Option<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let id = if finding.id.is_empty() { Uuid::new_v4().to_string() } else { finding.id.clone() };
        let entry = Finding { id: id.clone(), created_at: ts, updated_at: ts, ..finding };
        insert_finding(&conn, &entry)?;
        Ok(id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_update(finding: Finding, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let created: u64 = conn
            .query_row("SELECT created_at FROM findings WHERE id=?1", params![finding.id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let entry = Finding { updated_at: now_ts(), created_at: created, ..finding };
        insert_finding(&conn, &entry)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM findings WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_import_parsed(
    items: Vec<std::collections::HashMap<String, String>>,
    tool_name: Option<String>,
    project_path: Option<String>,
) -> Result<u32, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let tool = tool_name.unwrap_or_default();
        let mut added = 0u32;

        for item in items {
            let title = item.get("title").cloned().unwrap_or_default();
            let url = item.get("url").cloned().unwrap_or_default();
            if title.is_empty() && url.is_empty() { continue; }

            let exists: bool = conn
                .query_row("SELECT COUNT(*) FROM findings WHERE title=?1 AND url=?2", params![title, url], |r| r.get::<_, i64>(0))
                .map(|c| c > 0)
                .unwrap_or(false);
            if exists { continue; }

            let severity = match item.get("severity").map(|s| s.to_lowercase()).as_deref() {
                Some("critical") => Severity::Critical,
                Some("high") => Severity::High,
                Some("medium") => Severity::Medium,
                Some("low") => Severity::Low,
                _ => Severity::Info,
            };

            let f = Finding {
                id: Uuid::new_v4().to_string(),
                title,
                severity,
                cvss: item.get("cvss").and_then(|v| v.parse().ok()),
                url,
                target: item.get("target").cloned().unwrap_or_default(),
                description: item.get("description").cloned().unwrap_or_default(),
                steps: String::new(),
                remediation: String::new(),
                tags: Vec::new(),
                tool: tool.clone(),
                template: item.get("template").cloned().unwrap_or_default(),
                references: item.get("reference").map(|r| r.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
                evidence: Vec::new(),
                status: FindingStatus::Open,
                created_at: ts,
                updated_at: ts,
            };
            insert_finding(&conn, &f)?;
            added += 1;
        }
        Ok(added)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_add_evidence(
    finding_id: String,
    filename: String,
    mime_type: String,
    caption: String,
    data_base64: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let pp = project_path.clone();
    let evidence_id = Uuid::new_v4().to_string();
    let ext = filename.rsplit('.').next().unwrap_or("bin").to_string();
    let stored_name = format!("{}.{}", evidence_id, ext);

    let dir = evidence_dir(pp.as_deref()).join(&finding_id);
    fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("Invalid base64: {}", e))?;
    fs::write(dir.join(&stored_name), &bytes).await.map_err(|e| e.to_string())?;

    let eid = evidence_id.clone();
    let sname = stored_name.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let evidence_json: String = conn
            .query_row("SELECT evidence FROM findings WHERE id=?1", params![finding_id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut evidence_list: Vec<Evidence> = serde_json::from_str(&evidence_json).unwrap_or_default();
        evidence_list.push(Evidence { id: eid.clone(), filename: sname, mime_type, caption, added_at: ts });
        let new_json = serde_json::to_string(&evidence_list).unwrap_or_else(|_| "[]".to_string());
        conn.execute("UPDATE findings SET evidence=?1, updated_at=?2 WHERE id=?3", params![new_json, ts, finding_id])
            .map_err(|e| e.to_string())?;
        Ok(eid)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_remove_evidence(
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pp_clone = project_path.clone();
    let fid = finding_id.clone();

    let filename: Option<String> = tokio::task::spawn_blocking(move || {
        let conn = open_db(pp_clone.as_deref())?;
        let ts = now_ts();
        let evidence_json: String = conn
            .query_row("SELECT evidence FROM findings WHERE id=?1", params![fid], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let mut list: Vec<Evidence> = serde_json::from_str(&evidence_json).unwrap_or_default();
        let fname = list.iter().find(|e| e.id == evidence_id).map(|e| e.filename.clone());
        list.retain(|e| e.id != evidence_id);
        let new_json = serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string());
        conn.execute("UPDATE findings SET evidence=?1, updated_at=?2 WHERE id=?3", params![new_json, ts, fid])
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(fname)
    })
    .await
    .map_err(|e| e.to_string())??;

    if let Some(fname) = filename {
        let file = evidence_dir(project_path.as_deref()).join(&finding_id).join(&fname);
        let _ = fs::remove_file(&file).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn findings_evidence_path(
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let pp = project_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_db(pp.as_deref())?;
        let evidence_json: String = conn
            .query_row("SELECT evidence FROM findings WHERE id=?1", params![finding_id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let list: Vec<Evidence> = serde_json::from_str(&evidence_json).unwrap_or_default();
        let ev = list.iter().find(|e| e.id == evidence_id).ok_or("Evidence not found")?;
        let path = evidence_dir(project_path.as_deref()).join(&finding_id).join(&ev.filename);
        Ok(path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_for_host(host: String, project_path: Option<String>) -> Result<Vec<Finding>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let pattern = format!("%{}%", host.to_lowercase());
        let sql = format!(
            "SELECT {} FROM findings WHERE LOWER(url) LIKE ?1 OR LOWER(target) LIKE ?1 OR LOWER(title) LIKE ?1",
            SELECT_COLS
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let findings: Vec<Finding> = stmt
            .query_map(params![pattern], |row| row_to_finding(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(findings)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn findings_deduplicate(project_path: Option<String>) -> Result<u32, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let sql = format!("SELECT {} FROM findings ORDER BY created_at ASC", SELECT_COLS);
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut findings: Vec<Finding> = stmt
            .query_map([], |row| row_to_finding(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut removed = 0u32;
        let mut i = 0;
        while i < findings.len() {
            let key_title = findings[i].title.to_lowercase();
            let key_url = findings[i].url.to_lowercase();
            let key_target = findings[i].target.to_lowercase();

            let mut j = i + 1;
            while j < findings.len() {
                let dup_title = findings[j].title.to_lowercase();
                let dup_url = findings[j].url.to_lowercase();
                let dup_target = findings[j].target.to_lowercase();

                let is_dup = !key_title.is_empty()
                    && key_title == dup_title
                    && ((!key_url.is_empty() && key_url == dup_url)
                        || (!key_target.is_empty() && key_target == dup_target)
                        || (key_url.is_empty() && key_target.is_empty() && dup_url.is_empty() && dup_target.is_empty()));

                if is_dup {
                    let dup_tool = findings[j].tool.clone();
                    let dup_tags = findings[j].tags.clone();
                    let dup_evidence = findings[j].evidence.clone();
                    let dup_id = findings[j].id.clone();

                    let primary = &mut findings[i];
                    if !dup_tool.is_empty() && !primary.tool.contains(&dup_tool) {
                        if primary.tool.is_empty() { primary.tool = dup_tool; }
                        else { primary.tool = format!("{}, {}", primary.tool, dup_tool); }
                    }
                    for tag in dup_tags {
                        if !primary.tags.contains(&tag) { primary.tags.push(tag); }
                    }
                    for ev in dup_evidence { primary.evidence.push(ev); }
                    primary.updated_at = now_ts();

                    conn.execute("DELETE FROM findings WHERE id=?1", params![dup_id]).map_err(|e| e.to_string())?;
                    findings.remove(j);
                    removed += 1;
                } else {
                    j += 1;
                }
            }
            if removed > 0 {
                insert_finding(&conn, &findings[i])?;
            }
            i += 1;
        }
        Ok(removed)
    })
    .await
    .map_err(|e| e.to_string())?
}
