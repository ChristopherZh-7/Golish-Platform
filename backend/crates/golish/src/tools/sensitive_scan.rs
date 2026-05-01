use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::state::AppState;
pub use golish_pentest::sensitive_scan::{
    build_probe_targets, extract_dirs_from_sitemap, run_probe_loop, ProbeHit, ScanProgress,
    SensitiveScanConfig, SensitiveScanResult, DEFAULT_SENSITIVE_PATHS,
};

static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);
static SCAN_CANCELLED: AtomicBool = AtomicBool::new(false);

async fn load_wordlist_lines(wordlist_id: &str) -> Result<Vec<String>, String> {
    let path = super::wordlists::wordlist_path(wordlist_id.to_string()).await?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read wordlist: {}", e))?;
    Ok(content
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.to_string())
        .collect())
}

#[tauri::command]
pub async fn sensitive_scan_start(
    app: tauri::AppHandle,
    app_state: State<'_, AppState>,
    config: SensitiveScanConfig,
    project_path: Option<String>,
) -> Result<String, String> {
    if SCAN_RUNNING.load(Ordering::SeqCst) {
        return Err("A sensitive scan is already running".to_string());
    }
    SCAN_RUNNING.store(true, Ordering::SeqCst);
    SCAN_CANCELLED.store(false, Ordering::SeqCst);

    let pool = app_state.db_pool_ready().await?.clone();
    let scan_id = Uuid::new_v4().to_string();

    let probe_paths: Vec<String> = if let Some(ref wl_id) = config.wordlist_id {
        load_wordlist_lines(wl_id).await?
    } else {
        DEFAULT_SENSITIVE_PATHS
            .iter()
            .map(|s| s.to_string())
            .collect()
    };

    let wordlist_label = config
        .wordlist_id
        .clone()
        .unwrap_or_else(|| "builtin".to_string());

    let dirs: Vec<String> = if config.use_sitemap_dirs {
        let sitemap_data = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
        )
        .bind(project_path.as_deref())
        .fetch_optional(&pool)
        .await
        .unwrap_or(None)
        .unwrap_or(serde_json::json!([]));
        extract_dirs_from_sitemap(&sitemap_data)
    } else {
        let target = config.target_url.trim_end_matches('/');
        vec![format!("{}/", target)]
    };

    let pp = project_path.clone();
    let sid = scan_id.clone();
    let rate = config.rate_per_second;

    tokio::spawn(async move {
        let targets = build_probe_targets(&dirs, &probe_paths);
        let dirs_count = dirs.len();

        let _ = app.emit(
            "sensitive-scan-progress",
            serde_json::json!({
                "scanId": &sid, "total": targets.len(), "completed": 0, "hits": 0,
                "currentUrl": "", "running": true, "dirsFound": dirs_count,
            }),
        );

        let app_ref = &app;
        let sid_ref = &sid;
        let hits = run_probe_loop(
            &targets,
            rate,
            || SCAN_CANCELLED.load(Ordering::SeqCst),
            |completed, hit_count, url| {
                let _ = app_ref.emit(
                    "sensitive-scan-progress",
                    serde_json::json!({
                        "scanId": sid_ref, "total": targets.len(),
                        "completed": completed, "hits": hit_count,
                        "currentUrl": url, "running": true, "dirsFound": dirs_count,
                    }),
                );
            },
        )
        .await;

        for hit in &hits {
            let _ = sqlx::query(
                "INSERT INTO sensitive_scan_results (base_url, probe_path, full_url, status_code, content_length, content_type, wordlist_id, project_path) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (full_url, project_path) DO NOTHING",
            )
            .bind(&hit.base_dir).bind(&hit.probe_path).bind(&hit.full_url)
            .bind(hit.status_code).bind(hit.content_length).bind(&hit.content_type)
            .bind(&wordlist_label).bind(pp.as_deref())
            .execute(&pool).await;
        }

        SCAN_RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit(
            "sensitive-scan-progress",
            serde_json::json!({
                "scanId": &sid, "total": targets.len(),
                "completed": targets.len(), "hits": hits.len(),
                "currentUrl": "", "running": false, "dirsFound": dirs_count,
            }),
        );
    });

    Ok(scan_id)
}

#[tauri::command]
pub async fn sensitive_scan_stop() -> Result<(), String> {
    SCAN_CANCELLED.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn sensitive_scan_status() -> Result<bool, String> {
    Ok(SCAN_RUNNING.load(Ordering::SeqCst))
}

#[tauri::command]
pub async fn sensitive_scan_results(
    app_state: State<'_, AppState>,
    project_path: Option<String>,
    confirmed_only: Option<bool>,
) -> Result<Vec<SensitiveScanResult>, String> {
    let pool = app_state.db_pool_ready().await?;
    let rows = if confirmed_only.unwrap_or(false) {
        sqlx::query_as::<_, SensitiveScanRow>(
            "SELECT id, base_url, probe_path, full_url, status_code, content_length, content_type, is_confirmed, ai_verdict, created_at
             FROM sensitive_scan_results WHERE project_path = $1 AND is_confirmed = TRUE ORDER BY created_at DESC",
        )
        .bind(project_path.as_deref())
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, SensitiveScanRow>(
            "SELECT id, base_url, probe_path, full_url, status_code, content_length, content_type, is_confirmed, ai_verdict, created_at
             FROM sensitive_scan_results WHERE project_path = $1 ORDER BY created_at DESC",
        )
        .bind(project_path.as_deref())
        .fetch_all(pool)
        .await
    };
    rows.map(|r| r.into_iter().map(|row| row.into()).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sensitive_scan_clear(
    app_state: State<'_, AppState>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = app_state.db_pool_ready().await?;
    sqlx::query("DELETE FROM sensitive_scan_results WHERE project_path = $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM sensitive_scan_history WHERE project_path = $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn sensitive_scan_confirm(
    app_state: State<'_, AppState>,
    ids: Vec<String>,
    confirmed: bool,
) -> Result<(), String> {
    let pool = app_state.db_pool_ready().await?;
    for id in &ids {
        let uuid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
        sqlx::query("UPDATE sensitive_scan_results SET is_confirmed = $1 WHERE id = $2")
            .bind(confirmed)
            .bind(uuid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn sensitive_scan_default_paths() -> Result<Vec<String>, String> {
    Ok(DEFAULT_SENSITIVE_PATHS.iter().map(|s| s.to_string()).collect())
}

/// Apply AI verdicts to scan results and auto-import true positives as findings.
#[tauri::command]
pub async fn sensitive_scan_apply_verdicts(
    app: tauri::AppHandle,
    app_state: State<'_, AppState>,
    verdicts: Vec<serde_json::Value>,
    project_path: Option<String>,
) -> Result<serde_json::Value, String> {
    let pool = app_state.db_pool_ready().await?;

    let rows = sqlx::query_as::<_, SensitiveScanRow>(
        "SELECT id, base_url, probe_path, full_url, status_code, content_length, content_type, is_confirmed, ai_verdict, created_at
         FROM sensitive_scan_results WHERE project_path = $1",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut tp_count = 0u32;
    let mut applied = 0u32;
    for v in &verdicts {
        let path = v.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let verdict = v.get("verdict").and_then(|v| v.as_str()).unwrap_or("needs_review");
        let reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or("");

        if let Some(row) = rows.iter().find(|r| r.probe_path == path || r.full_url == path) {
            let _ = sqlx::query("UPDATE sensitive_scan_results SET ai_verdict = $1 WHERE id = $2")
                .bind(verdict)
                .bind(row.id)
                .execute(pool)
                .await;
            applied += 1;

            if verdict == "true_positive" {
                tp_count += 1;
                let title = format!("Sensitive file: {}", row.probe_path);
                let _ = sqlx::query(
                    "INSERT INTO findings (title, sev, url, target, description, tool, project_path) \
                     VALUES ($1, 'medium'::severity, $2, $3, $4, 'sensitive_scan', $5) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(&title)
                .bind(&row.full_url)
                .bind(&row.base_url)
                .bind(format!("AI analysis: {}. Status: {}, Size: {}", reason, row.status_code, row.content_length))
                .bind(project_path.as_deref())
                .execute(pool)
                .await;
            }
        }
    }

    let _ = app.emit("sensitive-scan-analyzed", serde_json::json!({
        "analyzed": applied,
        "truePositives": tp_count,
    }));

    Ok(serde_json::json!({
        "analyzed": applied,
        "true_positives": tp_count,
    }))
}

#[derive(sqlx::FromRow)]
struct SensitiveScanRow {
    id: Uuid,
    base_url: String,
    probe_path: String,
    full_url: String,
    status_code: i32,
    content_length: i32,
    content_type: String,
    is_confirmed: bool,
    ai_verdict: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<SensitiveScanRow> for SensitiveScanResult {
    fn from(r: SensitiveScanRow) -> Self {
        Self {
            id: r.id.to_string(),
            base_url: r.base_url,
            probe_path: r.probe_path,
            full_url: r.full_url,
            status_code: r.status_code,
            content_length: r.content_length,
            content_type: r.content_type,
            is_confirmed: r.is_confirmed,
            ai_verdict: r.ai_verdict,
            created_at: r.created_at.timestamp_millis(),
        }
    }
}
