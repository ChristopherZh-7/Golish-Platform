use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use tracing::debug;
use zip::write::SimpleFileOptions;

use crate::state::AppState;

fn app_data_dir() -> PathBuf {
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
    base
}

fn resolve_project_golish_dir(project_path: Option<&str>) -> Option<PathBuf> {
    project_path.map(|p| PathBuf::from(p).join(".golish"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: String,
    pub size_bytes: u64,
    pub files_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub files_count: usize,
    pub wiki_count: usize,
    pub tools_count: usize,
    pub has_targets: bool,
    pub has_vault: bool,
    pub has_skills: bool,
    pub has_methodology: bool,
    pub has_topology: bool,
    pub has_findings: bool,
    pub has_recordings: bool,
}

fn add_directory_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &std::path::Path,
    prefix: &str,
    count: &mut usize,
) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let walker = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok());
    for entry in walker {
        let path = entry.path();
        let rel = path.strip_prefix(dir).map_err(|e| e.to_string())?;
        let archive_path = format!("{}/{}", prefix, rel.to_string_lossy());
        if path.is_dir() {
            zip.add_directory(&archive_path, options).map_err(|e| e.to_string())?;
        } else {
            zip.start_file(&archive_path, options).map_err(|e| e.to_string())?;
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            zip.write_all(&data).map_err(|e| e.to_string())?;
            *count += 1;
        }
    }
    Ok(())
}

fn add_json_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    archive_name: &str,
    json: &[u8],
    count: &mut usize,
) -> Result<(), String> {
    if json == b"[]" || json == b"null" {
        return Ok(());
    }
    let options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(archive_name, options).map_err(|e| e.to_string())?;
    zip.write_all(json).map_err(|e| e.to_string())?;
    *count += 1;
    Ok(())
}

async fn export_table_as_json(
    pool: &sqlx::PgPool,
    query: &str,
) -> Result<Vec<u8>, String> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(query)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_vec_pretty(&rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn project_export(
    state: tauri::State<'_, AppState>,
    output_path: String,
    project_path: Option<String>,
) -> Result<ExportResult, String> {
    let base = app_data_dir();
    let golish_dir = resolve_project_golish_dir(project_path.as_deref());
    let output = PathBuf::from(&output_path);
    let pool = state.db_pool_ready().await?;

    let file = std::fs::File::create(&output).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let mut count = 0usize;

    // Global shared data (file-based, unchanged)
    add_directory_to_zip(&mut zip, &base.join("wiki"), "wiki", &mut count)?;
    add_directory_to_zip(&mut zip, &base.join("toolsconfig"), "toolsconfig", &mut count)?;
    add_directory_to_zip(&mut zip, &base.join("skills"), "skills", &mut count)?;

    // Export database tables as JSON
    let targets = export_table_as_json(pool, "SELECT row_to_json(t) FROM targets t").await?;
    add_json_to_zip(&mut zip, "golish/db/targets.json", &targets, &mut count)?;

    let findings = export_table_as_json(pool, "SELECT row_to_json(t) FROM findings t").await?;
    add_json_to_zip(&mut zip, "golish/db/findings.json", &findings, &mut count)?;

    let vault = export_table_as_json(pool, "SELECT row_to_json(t) FROM vault_entries t").await?;
    add_json_to_zip(&mut zip, "golish/db/vault_entries.json", &vault, &mut count)?;

    let notes = export_table_as_json(pool, "SELECT row_to_json(t) FROM notes t").await?;
    add_json_to_zip(&mut zip, "golish/db/notes.json", &notes, &mut count)?;

    let audit = export_table_as_json(pool, "SELECT row_to_json(t) FROM audit_log t").await?;
    add_json_to_zip(&mut zip, "golish/db/audit_log.json", &audit, &mut count)?;

    let topo = export_table_as_json(pool, "SELECT row_to_json(t) FROM topology_scans t").await?;
    add_json_to_zip(&mut zip, "golish/db/topology_scans.json", &topo, &mut count)?;

    let meth = export_table_as_json(pool, "SELECT row_to_json(t) FROM methodology_projects t").await?;
    add_json_to_zip(&mut zip, "golish/db/methodology_projects.json", &meth, &mut count)?;

    let pipes = export_table_as_json(pool, "SELECT row_to_json(t) FROM pipelines t").await?;
    add_json_to_zip(&mut zip, "golish/db/pipelines.json", &pipes, &mut count)?;

    let recordings = export_table_as_json(pool, "SELECT row_to_json(t) FROM recordings t").await?;
    add_json_to_zip(&mut zip, "golish/db/recordings.json", &recordings, &mut count)?;

    let vuln_feeds = export_table_as_json(pool, "SELECT row_to_json(t) FROM vuln_feeds t").await?;
    add_json_to_zip(&mut zip, "golish/db/vuln_feeds.json", &vuln_feeds, &mut count)?;

    let vuln_entries = export_table_as_json(pool, "SELECT row_to_json(t) FROM vuln_entries t").await?;
    add_json_to_zip(&mut zip, "golish/db/vuln_entries.json", &vuln_entries, &mut count)?;

    // File-based project data
    if let Some(ref gd) = golish_dir {
        add_directory_to_zip(&mut zip, &gd.join("evidence"), "golish/evidence", &mut count)?;
    }

    zip.finish().map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&output).map_err(|e| e.to_string())?;
    debug!("[project_export] Exported {} files to {}", count, output_path);

    Ok(ExportResult {
        path: output_path,
        size_bytes: meta.len(),
        files_count: count,
    })
}

async fn import_json_rows(
    pool: &sqlx::PgPool,
    table: &str,
    json_data: &[u8],
) -> Result<usize, String> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(json_data).map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut imported = 0usize;
    for row in &rows {
        let obj = row.as_object().ok_or("Expected JSON object")?;
        let columns: Vec<&String> = obj.keys().collect();
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("${i}")).collect();

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT DO NOTHING",
            table,
            columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(", "),
            placeholders.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for col in &columns {
            let val = &obj[col.as_str()];
            query = query.bind(val.clone());
        }

        match query.execute(pool).await {
            Ok(r) if r.rows_affected() > 0 => imported += 1,
            Err(e) => {
                debug!("[project_import] Skipping row in {table}: {e}");
            }
            _ => {}
        }
    }
    Ok(imported)
}

struct ExtractedEntry {
    name: String,
    is_dir: bool,
    data: Vec<u8>,
}

#[tauri::command]
pub async fn project_import(
    state: tauri::State<'_, AppState>,
    zip_path: String,
    overwrite: bool,
    project_path: Option<String>,
) -> Result<ImportResult, String> {
    let base = app_data_dir();
    let golish_dir = resolve_project_golish_dir(project_path.as_deref());
    let pool = state.db_pool_ready().await?;

    // Phase 1: read entire zip synchronously (ZipFile is not Send)
    let entries = {
        let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().to_string();
            let is_dir = entry.is_dir();
            let mut data = Vec::new();
            if !is_dir {
                entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
            }
            out.push(ExtractedEntry { name, is_dir, data });
        }
        out
    };

    let mut result = ImportResult {
        files_count: 0,
        wiki_count: 0,
        tools_count: 0,
        has_targets: false,
        has_vault: false,
        has_skills: false,
        has_methodology: false,
        has_topology: false,
        has_findings: false,
        has_recordings: false,
    };

    let db_table_map: &[(&str, &str)] = &[
        ("golish/db/targets.json", "targets"),
        ("golish/db/findings.json", "findings"),
        ("golish/db/vault_entries.json", "vault_entries"),
        ("golish/db/notes.json", "notes"),
        ("golish/db/audit_log.json", "audit_log"),
        ("golish/db/topology_scans.json", "topology_scans"),
        ("golish/db/methodology_projects.json", "methodology_projects"),
        ("golish/db/pipelines.json", "pipelines"),
        ("golish/db/recordings.json", "recordings"),
        ("golish/db/vuln_feeds.json", "vuln_feeds"),
        ("golish/db/vuln_entries.json", "vuln_entries"),
    ];

    // Phase 2: process entries (async-safe now)
    for entry in &entries {
        if entry.is_dir {
            if let Some(d) = resolve_import_path(&entry.name, &base, golish_dir.as_ref()) {
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            }
            continue;
        }

        // Handle DB JSON imports
        if let Some((_, table)) = db_table_map.iter().find(|(path, _)| *path == entry.name) {
            let count = import_json_rows(pool, table, &entry.data).await?;
            result.files_count += count;

            match *table {
                "targets" => result.has_targets = true,
                "findings" => result.has_findings = true,
                "vault_entries" => result.has_vault = true,
                "topology_scans" => result.has_topology = true,
                "methodology_projects" => result.has_methodology = true,
                "recordings" => result.has_recordings = true,
                _ => {}
            }
            continue;
        }

        // Handle file-based entries (wiki, toolsconfig, skills, evidence)
        let target = match resolve_import_path(&entry.name, &base, golish_dir.as_ref()) {
            Some(p) => p,
            None => continue,
        };

        if !overwrite && target.exists() {
            debug!("[project_import] Skipping existing file: {}", entry.name);
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        std::fs::write(&target, &entry.data).map_err(|e| e.to_string())?;
        result.files_count += 1;

        if entry.name.starts_with("wiki/") {
            result.wiki_count += 1;
        }
        if entry.name.starts_with("toolsconfig/") {
            result.tools_count += 1;
        }
        if entry.name.starts_with("skills/") {
            result.has_skills = true;
        }
    }

    debug!(
        "[project_import] Imported {} items from {}",
        result.files_count, zip_path
    );
    Ok(result)
}

fn resolve_import_path(
    archive_name: &str,
    global_base: &std::path::Path,
    golish_dir: Option<&PathBuf>,
) -> Option<PathBuf> {
    if archive_name.starts_with("golish/db/") {
        return None;
    }
    if let Some(rest) = archive_name.strip_prefix("golish/") {
        return golish_dir.map(|gd| gd.join(rest));
    }
    Some(global_base.join(archive_name))
}
