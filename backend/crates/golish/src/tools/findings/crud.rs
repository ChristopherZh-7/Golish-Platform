//! Findings CRUD operations (Tauri commands).

use crate::error::GolishError;
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;
use crate::state::DbState;
use super::{evidence_dir, now_ts, ts_from_dt, Finding, FindingRow, FindingsStore, Evidence, Severity, FindingStatus};

const SELECT_COLS: &str = "id, title, sev::TEXT, cvss, url, target, target_id, description, steps, remediation, tags, tool, template, refs, evidence, status::TEXT, source, created_at, updated_at";

#[tauri::command]
pub async fn findings_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<FindingsStore, GolishError> {
    let pool = state.pool_ready().await?;
    let sql = format!(
        "SELECT {} FROM findings WHERE project_path = $1 ORDER BY created_at DESC",
        SELECT_COLS
    );
    let rows: Vec<FindingRow> = sqlx::query_as(&sql)
        .bind(project_path.as_deref())
        .fetch_all(pool)
        .await
?;

    Ok(FindingsStore {
        findings: rows.into_iter().map(Finding::from).collect(),
    })
}

async fn insert_finding(pool: &sqlx::PgPool, f: &Finding, project_path: Option<&str>) -> Result<(), GolishError> {
    let uid: Uuid = f.id.parse().unwrap_or_else(|_| Uuid::new_v4());
    let tags_json = serde_json::to_value(&f.tags).unwrap_or_else(|_| serde_json::json!([]));
    let refs_json = serde_json::to_value(&f.references).unwrap_or_else(|_| serde_json::json!([]));
    let evidence_json = serde_json::to_value(&f.evidence).unwrap_or_else(|_| serde_json::json!([]));

    let tid: Option<Uuid> = f.target_id.as_ref().and_then(|s| s.parse().ok());
    sqlx::query(
        r#"INSERT INTO findings (id, title, sev, cvss, url, target, target_id, description, steps, remediation, tags, tool, template, refs, evidence, status, source, project_path, created_at, updated_at)
           VALUES ($1, $2, $3::severity, $4, $5, $6, $20, $7, $8, $9, $10, $11, $12, $13, $14, $15::finding_status, $16, $17,
                   to_timestamp($18::DOUBLE PRECISION), to_timestamp($19::DOUBLE PRECISION))
           ON CONFLICT (id) DO UPDATE SET
             title=$2, sev=$3::severity, cvss=$4, url=$5, target=$6, target_id=COALESCE($20, findings.target_id), description=$7, steps=$8, remediation=$9,
             tags=$10, tool=$11, template=$12, refs=$13, evidence=$14, status=$15::finding_status, source=$16, updated_at=to_timestamp($19::DOUBLE PRECISION)"#,
    )
    .bind(uid)
    .bind(&f.title)
    .bind(f.severity.as_str())
    .bind(f.cvss)
    .bind(&f.url)
    .bind(&f.target)
    .bind(&f.description)
    .bind(&f.steps)
    .bind(&f.remediation)
    .bind(&tags_json)
    .bind(&f.tool)
    .bind(&f.template)
    .bind(&refs_json)
    .bind(&evidence_json)
    .bind(f.status.as_str())
    .bind(&f.source)
    .bind(project_path)
    .bind(f.created_at as f64)
    .bind(f.updated_at as f64)
    .bind(tid)
    .execute(pool)
    .await
?;
    Ok(())
}

#[tauri::command]
pub async fn findings_add(
    state: tauri::State<'_, DbState>,
    finding: Finding,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let ts = now_ts();
    let id = if finding.id.is_empty() { Uuid::new_v4().to_string() } else { finding.id.clone() };
    let entry = Finding { id: id.clone(), created_at: ts, updated_at: ts, ..finding };
    insert_finding(pool, &entry, project_path.as_deref()).await?;
    Ok(id)
}

#[tauri::command]
pub async fn findings_update(
    state: tauri::State<'_, DbState>,
    finding: Finding,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = finding.id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let created: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT created_at FROM findings WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
?;
    let entry = Finding {
        updated_at: now_ts(),
        created_at: ts_from_dt(created),
        ..finding
    };
    insert_finding(pool, &entry, project_path.as_deref()).await
}

#[tauri::command]
pub async fn findings_delete(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM findings WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
?;
    Ok(())
}

#[tauri::command]
pub async fn findings_import_parsed(
    state: tauri::State<'_, DbState>,
    items: Vec<std::collections::HashMap<String, String>>,
    tool_name: Option<String>,
    project_path: Option<String>,
) -> Result<u32, GolishError> {
    let pool = state.pool_ready().await?;
    let ts = now_ts();
    let tool = tool_name.unwrap_or_default();
    let mut added = 0u32;

    for item in items {
        let title = item.get("title").cloned().unwrap_or_default();
        let url = item.get("url").cloned().unwrap_or_default();
        if title.is_empty() && url.is_empty() { continue; }

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM findings WHERE title=$1 AND url=$2",
        )
        .bind(&title)
        .bind(&url)
        .fetch_one(pool)
        .await
?;
        if count > 0 { continue; }

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
            target_id: None,
            description: item.get("description").cloned().unwrap_or_default(),
            steps: String::new(),
            remediation: String::new(),
            tags: Vec::new(),
            tool: tool.clone(),
            template: item.get("template").cloned().unwrap_or_default(),
            references: item.get("reference").map(|r| r.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
            evidence: Vec::new(),
            status: FindingStatus::Open,
            source: "automated".to_string(),
            created_at: ts,
            updated_at: ts,
        };
        insert_finding(pool, &f, project_path.as_deref()).await?;
        added += 1;
    }
    Ok(added)
}

#[tauri::command]
pub async fn findings_add_evidence(
    state: tauri::State<'_, DbState>,
    finding_id: String,
    filename: String,
    mime_type: String,
    caption: String,
    data_base64: String,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let evidence_id = Uuid::new_v4().to_string();
    let ext = filename.rsplit('.').next().unwrap_or("bin").to_string();
    let stored_name = format!("{}.{}", evidence_id, ext);

    let dir = evidence_dir(project_path.as_deref()).join(&finding_id);
    fs::create_dir_all(&dir).await?;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data_base64)
        .map_err(|e| format!("Invalid base64: {}", e))?;
    fs::write(dir.join(&stored_name), &bytes).await?;

    let uid: Uuid = finding_id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let ts = now_ts();
    let evidence_val: serde_json::Value = sqlx::query_scalar(
        "SELECT evidence FROM findings WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
?;

    let mut evidence_list: Vec<Evidence> = serde_json::from_value(evidence_val).unwrap_or_default();
    evidence_list.push(Evidence {
        id: evidence_id.clone(),
        filename: stored_name,
        mime_type,
        caption,
        added_at: ts,
    });
    let new_json = serde_json::to_value(&evidence_list).unwrap_or_else(|_| serde_json::json!([]));

    sqlx::query("UPDATE findings SET evidence=$1, updated_at=NOW() WHERE id=$2")
        .bind(&new_json)
        .bind(uid)
        .execute(pool)
        .await
?;

    Ok(evidence_id)
}

#[tauri::command]
pub async fn findings_remove_evidence(
    state: tauri::State<'_, DbState>,
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = finding_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let evidence_val: serde_json::Value = sqlx::query_scalar(
        "SELECT evidence FROM findings WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
?;

    let mut list: Vec<Evidence> = serde_json::from_value(evidence_val).unwrap_or_default();
    let fname = list.iter().find(|e| e.id == evidence_id).map(|e| e.filename.clone());
    list.retain(|e| e.id != evidence_id);
    let new_json = serde_json::to_value(&list).unwrap_or_else(|_| serde_json::json!([]));

    sqlx::query("UPDATE findings SET evidence=$1, updated_at=NOW() WHERE id=$2")
        .bind(&new_json)
        .bind(uid)
        .execute(pool)
        .await
?;

    if let Some(fname) = fname {
        let file = evidence_dir(project_path.as_deref()).join(&finding_id).join(&fname);
        let _ = fs::remove_file(&file).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn findings_evidence_path(
    state: tauri::State<'_, DbState>,
    finding_id: String,
    evidence_id: String,
    project_path: Option<String>,
) -> Result<String, GolishError> {
    let pool = state.pool_ready().await?;
    let uid: Uuid = finding_id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let evidence_val: serde_json::Value = sqlx::query_scalar(
        "SELECT evidence FROM findings WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
?;

    let list: Vec<Evidence> = serde_json::from_value(evidence_val).unwrap_or_default();
    let ev = list.iter().find(|e| e.id == evidence_id).ok_or_else(|| GolishError::Internal("Evidence not found".into()))?;
    let path = evidence_dir(project_path.as_deref()).join(&finding_id).join(&ev.filename);
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn findings_for_host(
    state: tauri::State<'_, DbState>,
    host: String,
    project_path: Option<String>,
) -> Result<Vec<Finding>, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let pattern = format!("%{}%", host.to_lowercase());
    let sql = format!(
        "SELECT {} FROM findings WHERE LOWER(url) LIKE $1 OR LOWER(target) LIKE $1 OR LOWER(title) LIKE $1",
        SELECT_COLS
    );
    let rows: Vec<FindingRow> = sqlx::query_as(&sql)
        .bind(&pattern)
        .fetch_all(pool)
        .await
?;

    Ok(rows.into_iter().map(Finding::from).collect())
}

#[tauri::command]
pub async fn findings_deduplicate(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<u32, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let sql = format!("SELECT {} FROM findings ORDER BY created_at ASC", SELECT_COLS);
    let rows: Vec<FindingRow> = sqlx::query_as(&sql)
        .fetch_all(pool)
        .await
?;

    let mut findings: Vec<Finding> = rows.into_iter().map(Finding::from).collect();
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

                let dup_uid: Uuid = dup_id.parse().unwrap_or_else(|_| Uuid::new_v4());
                sqlx::query("DELETE FROM findings WHERE id=$1")
                    .bind(dup_uid)
                    .execute(pool)
                    .await
?;
                findings.remove(j);
                removed += 1;
            } else {
                j += 1;
            }
        }
        if removed > 0 {
            insert_finding(pool, &findings[i], None).await?;
        }
        i += 1;
    }
    Ok(removed)
}
