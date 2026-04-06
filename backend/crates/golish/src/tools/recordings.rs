use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;

fn recordings_dir(project_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(pp) = project_path {
        let dir = PathBuf::from(pp).join(".golish").join("recordings");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        return Ok(dir);
    }
    let home = dirs::home_dir().ok_or("cannot resolve home directory")?;
    #[cfg(target_os = "macos")]
    let dir = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform")
        .join("recordings");
    #[cfg(target_os = "windows")]
    let dir = home
        .join("AppData")
        .join("Local")
        .join("golish-platform")
        .join("recordings");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let dir = home.join(".golish-platform").join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingMeta {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub width: u16,
    pub height: u16,
    pub duration_ms: u64,
    pub event_count: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub meta: RecordingMeta,
    pub events: Vec<(f64, String)>, // (elapsed_seconds, data)
}

#[tauri::command]
pub async fn recording_save(
    recording: Recording,
    project_path: Option<String>,
) -> Result<String, String> {
    let dir = recordings_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", recording.meta.id));
    let json = serde_json::to_string(&recording).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    debug!(
        "[recording_save] Saved recording {} ({} events)",
        recording.meta.id, recording.meta.event_count
    );
    Ok(recording.meta.id.clone())
}

#[tauri::command]
pub async fn recording_load(
    id: String,
    project_path: Option<String>,
) -> Result<Recording, String> {
    let dir = recordings_dir(project_path.as_deref())?;
    let path = dir.join(format!("{id}.json"));
    if !path.exists() {
        return Err(format!("Recording {id} not found"));
    }
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recording_list(
    project_path: Option<String>,
) -> Result<Vec<RecordingMeta>, String> {
    let dir = recordings_dir(project_path.as_deref())?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut list = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(rec) = serde_json::from_str::<Recording>(&data) {
                    list.push(rec.meta);
                }
            }
        }
    }
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(list)
}

#[tauri::command]
pub async fn recording_delete(
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let dir = recordings_dir(project_path.as_deref())?;
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        debug!("[recording_delete] Deleted recording {id}");
    }
    Ok(())
}
