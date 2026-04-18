use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{Emitter, State};
use uuid::Uuid;

use crate::state::AppState;

const DEFAULT_SENSITIVE_PATHS: &[&str] = &[
    ".env", ".env.local", ".env.production", ".env.backup",
    ".git/config", ".git/HEAD", ".gitignore",
    ".svn/entries", ".svn/wc.db",
    ".DS_Store", "Thumbs.db",
    ".htaccess", ".htpasswd",
    "web.config", "crossdomain.xml",
    "robots.txt", "sitemap.xml", "security.txt", ".well-known/security.txt",
    "wp-config.php", "wp-config.php.bak", "wp-login.php",
    "config.php", "config.inc.php", "config.yml", "config.json",
    "database.yml", "settings.py", "application.yml", "application.properties",
    "composer.json", "package.json", "Gemfile", "requirements.txt", "go.mod",
    "phpinfo.php", "info.php", "test.php",
    "backup.sql", "dump.sql", "database.sql", "db.sql",
    "backup.zip", "backup.tar.gz", "backup.rar",
    "server-status", "server-info",
    ".bash_history", ".ssh/id_rsa", ".ssh/id_rsa.pub",
    "id_rsa", "id_dsa",
    "admin/", "administrator/", "admin.php", "login.php",
    "phpmyadmin/", "pma/", "adminer.php",
    "swagger-ui.html", "swagger.json", "api-docs", "openapi.json",
    "actuator", "actuator/health", "actuator/env",
    "debug/", "trace/", "console/",
    "graphql", "graphiql",
    ".dockerenv", "Dockerfile", "docker-compose.yml",
    "Makefile", "Rakefile", "Vagrantfile",
    "error_log", "access_log", "debug.log",
    "xmlrpc.php", "wp-cron.php",
    "CHANGELOG.md", "CHANGELOG.txt", "VERSION", "README.md",
    "license.txt", "LICENSE",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveScanConfig {
    pub target_url: String,
    pub wordlist_id: Option<String>,
    pub rate_per_second: u32,
    pub use_sitemap_dirs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveScanResult {
    pub id: String,
    pub base_url: String,
    pub probe_path: String,
    pub full_url: String,
    pub status_code: i32,
    pub content_length: i32,
    pub content_type: String,
    pub is_confirmed: bool,
    pub ai_verdict: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub total: usize,
    pub completed: usize,
    pub hits: usize,
    pub current_url: String,
    pub running: bool,
    pub dirs_found: usize,
}

static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);
static SCAN_CANCELLED: AtomicBool = AtomicBool::new(false);

fn extract_dirs_from_sitemap(data: &serde_json::Value) -> Vec<String> {
    let entries = match data.as_array() {
        Some(a) => a,
        None => return vec![],
    };
    let mut dirs = std::collections::BTreeSet::new();
    for entry in entries {
        let url_str = entry.get("url").or_else(|| entry.get("uri"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Ok(parsed) = url::Url::parse(url_str) {
            let base = format!("{}://{}", parsed.scheme(), parsed.authority());
            let path = parsed.path();
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            dirs.insert(format!("{}/", base));
            let mut accum = String::new();
            for seg in &segments[..segments.len().saturating_sub(1)] {
                accum.push_str(seg);
                accum.push('/');
                dirs.insert(format!("{}/{}", base, accum));
            }
        }
    }
    dirs.into_iter().collect()
}

async fn load_wordlist_lines(wordlist_id: &str) -> Result<Vec<String>, String> {
    let path = super::wordlists::wordlist_path(wordlist_id.to_string()).await?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read wordlist: {}", e))?;
    Ok(content.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(|s| s.to_string()).collect())
}

async fn get_already_scanned(pool: &PgPool, base_url: &str, wordlist_id: &str, _project_path: Option<&str>) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM sensitive_scan_history WHERE base_url = $1 AND wordlist_id = $2 AND project_path = $3)",
    )
    .bind(base_url)
    .bind(wordlist_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false)
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

    let pool = app_state.db_pool_ready().await?;
    let scan_id = Uuid::new_v4().to_string();

    let probe_paths: Vec<String> = if let Some(ref wl_id) = config.wordlist_id {
        load_wordlist_lines(wl_id).await?
    } else {
        DEFAULT_SENSITIVE_PATHS.iter().map(|s| s.to_string()).collect()
    };

    let wordlist_label = config.wordlist_id.clone().unwrap_or_else(|| "builtin".to_string());

    let dirs: Vec<String> = if config.use_sitemap_dirs {
        let sitemap_data = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
        )
        .bind(project_path.as_deref())
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .unwrap_or(serde_json::json!([]));
        extract_dirs_from_sitemap(&sitemap_data)
    } else {
        let target = config.target_url.trim_end_matches('/');
        vec![format!("{}/", target)]
    };

    let pool2 = pool.clone();
    let pp = project_path.clone();
    let app2 = app.clone();
    let sid = scan_id.clone();

    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_default();

        let delay = if config.rate_per_second > 0 {
            Duration::from_millis(1000 / config.rate_per_second as u64)
        } else {
            Duration::from_millis(50)
        };

        let mut tasks: Vec<(String, String)> = Vec::new();
        for dir in &dirs {
            if get_already_scanned(&pool2, dir, &wordlist_label, pp.as_deref()).await {
                continue;
            }
            for path in &probe_paths {
                let full = format!("{}{}", dir, path.trim_start_matches('/'));
                tasks.push((dir.clone(), full));
            }
        }

        let total = tasks.len();
        let mut completed = 0usize;
        let mut hits = 0usize;

        let _ = app2.emit("sensitive-scan-progress", serde_json::json!({
            "scanId": &sid, "total": total, "completed": 0, "hits": 0,
            "currentUrl": "", "running": true, "dirsFound": dirs.len(),
        }));

        let mut current_dir = String::new();
        let mut dir_probe_count = 0u32;
        let mut dir_hit_count = 0u32;

        for (base_dir, full_url) in &tasks {
            if SCAN_CANCELLED.load(Ordering::SeqCst) {
                break;
            }

            if *base_dir != current_dir {
                if !current_dir.is_empty() {
                    let _ = sqlx::query(
                        "INSERT INTO sensitive_scan_history (base_url, wordlist_id, probe_count, hit_count, project_path)
                         VALUES ($1, $2, $3, $4, $5)
                         ON CONFLICT (base_url, wordlist_id, project_path) DO UPDATE SET probe_count = $3, hit_count = $4, scanned_at = NOW()",
                    )
                    .bind(&current_dir).bind(&wordlist_label)
                    .bind(dir_probe_count as i32).bind(dir_hit_count as i32)
                    .bind(pp.as_deref())
                    .execute(&pool2).await;
                }
                current_dir = base_dir.clone();
                dir_probe_count = 0;
                dir_hit_count = 0;
            }
            dir_probe_count += 1;

            let resp = match client.get(full_url).send().await {
                Ok(r) => r,
                Err(_) => {
                    completed += 1;
                    continue;
                }
            };

            let status = resp.status().as_u16() as i32;
            let ct = resp.headers().get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let cl = resp.headers().get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);

            if status >= 200 && status < 400 && status != 301 && status != 302 {
                hits += 1;
                dir_hit_count += 1;
                let probe_path = full_url.strip_prefix(base_dir).unwrap_or(full_url).to_string();
                let _ = sqlx::query(
                    "INSERT INTO sensitive_scan_results (base_url, probe_path, full_url, status_code, content_length, content_type, wordlist_id, project_path)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                     ON CONFLICT (full_url, project_path) DO NOTHING",
                )
                .bind(base_dir).bind(&probe_path).bind(full_url)
                .bind(status).bind(cl).bind(&ct)
                .bind(&wordlist_label).bind(pp.as_deref())
                .execute(&pool2).await;
            }

            completed += 1;
            if completed % 10 == 0 || completed == total {
                let _ = app2.emit("sensitive-scan-progress", serde_json::json!({
                    "scanId": &sid, "total": total, "completed": completed, "hits": hits,
                    "currentUrl": full_url, "running": true, "dirsFound": dirs.len(),
                }));
            }

            tokio::time::sleep(delay).await;
        }

        if !current_dir.is_empty() {
            let _ = sqlx::query(
                "INSERT INTO sensitive_scan_history (base_url, wordlist_id, probe_count, hit_count, project_path)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (base_url, wordlist_id, project_path) DO UPDATE SET probe_count = $3, hit_count = $4, scanned_at = NOW()",
            )
            .bind(&current_dir).bind(&wordlist_label)
            .bind(dir_probe_count as i32).bind(dir_hit_count as i32)
            .bind(pp.as_deref())
            .execute(&pool2).await;
        }

        SCAN_RUNNING.store(false, Ordering::SeqCst);
        let _ = app2.emit("sensitive-scan-progress", serde_json::json!({
            "scanId": &sid, "total": total, "completed": completed, "hits": hits,
            "currentUrl": "", "running": false, "dirsFound": dirs.len(),
        }));
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
