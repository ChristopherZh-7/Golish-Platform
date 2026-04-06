use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use tracing::debug;
use zip::write::SimpleFileOptions;

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
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let walker = walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok());
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

fn add_file_to_zip(
    zip: &mut zip::ZipWriter<std::fs::File>,
    file: &std::path::Path,
    archive_name: &str,
    count: &mut usize,
) -> Result<(), String> {
    if !file.exists() {
        return Ok(());
    }
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(archive_name, options).map_err(|e| e.to_string())?;
    let data = std::fs::read(file).map_err(|e| e.to_string())?;
    zip.write_all(&data).map_err(|e| e.to_string())?;
    *count += 1;
    Ok(())
}

#[tauri::command]
pub async fn project_export(
    output_path: String,
    project_path: Option<String>,
) -> Result<ExportResult, String> {
    let base = app_data_dir();
    let golish_dir = resolve_project_golish_dir(project_path.as_deref());
    let output = PathBuf::from(&output_path);

    let file = std::fs::File::create(&output).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let mut count = 0usize;

    // Global shared data
    add_directory_to_zip(&mut zip, &base.join("wiki"), "wiki", &mut count)?;
    add_directory_to_zip(&mut zip, &base.join("toolsconfig"), "toolsconfig", &mut count)?;
    add_directory_to_zip(&mut zip, &base.join("skills"), "skills", &mut count)?;

    // Project-specific data from .golish/ directory
    if let Some(ref gd) = golish_dir {
        add_directory_to_zip(&mut zip, &gd.join("targets"), "golish/targets", &mut count)?;
        add_directory_to_zip(&mut zip, &gd.join("vault"), "golish/vault", &mut count)?;
        add_directory_to_zip(&mut zip, &gd.join("methodology"), "golish/methodology", &mut count)?;
        add_directory_to_zip(&mut zip, &gd.join("topology"), "golish/topology", &mut count)?;
        let findings_file = gd.join("findings.json");
        if findings_file.exists() {
            add_file_to_zip(&mut zip, &findings_file, "golish/findings.json", &mut count)?;
        }
        add_directory_to_zip(&mut zip, &gd.join("recordings"), "golish/recordings", &mut count)?;
        add_directory_to_zip(&mut zip, &gd.join("evidence"), "golish/evidence", &mut count)?;
        add_directory_to_zip(&mut zip, &gd.join("pipelines"), "golish/pipelines", &mut count)?;
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

#[tauri::command]
pub async fn project_import(
    zip_path: String,
    overwrite: bool,
    project_path: Option<String>,
) -> Result<ImportResult, String> {
    let base = app_data_dir();
    let golish_dir = resolve_project_golish_dir(project_path.as_deref());
    let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

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

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            let dir = resolve_import_path(&name, &base, golish_dir.as_ref());
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            continue;
        }

        let target = resolve_import_path(&name, &base, golish_dir.as_ref());

        if !overwrite && target.exists() {
            debug!("[project_import] Skipping existing file: {}", name);
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| e.to_string())?;
        std::fs::write(&target, &data).map_err(|e| e.to_string())?;
        result.files_count += 1;

        if name.starts_with("wiki/") { result.wiki_count += 1; }
        if name.starts_with("toolsconfig/") { result.tools_count += 1; }
        if name.starts_with("skills/") { result.has_skills = true; }
        if name.starts_with("golish/targets/") { result.has_targets = true; }
        if name.starts_with("golish/vault/") { result.has_vault = true; }
        if name.starts_with("golish/methodology/") { result.has_methodology = true; }
        if name.starts_with("golish/topology/") { result.has_topology = true; }
        if name.starts_with("golish/findings") { result.has_findings = true; }
        if name.starts_with("golish/recordings/") { result.has_recordings = true; }
    }

    debug!("[project_import] Imported {} files from {}", result.files_count, zip_path);
    Ok(result)
}

/// Routes archive paths: `golish/*` entries go to the project's `.golish/` dir,
/// everything else goes to the global app data dir.
fn resolve_import_path(
    archive_name: &str,
    global_base: &std::path::Path,
    golish_dir: Option<&PathBuf>,
) -> PathBuf {
    if let Some(rest) = archive_name.strip_prefix("golish/") {
        if let Some(gd) = golish_dir {
            return gd.join(rest);
        }
    }
    global_base.join(archive_name)
}
