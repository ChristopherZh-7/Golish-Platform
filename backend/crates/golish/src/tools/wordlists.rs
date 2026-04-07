use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use tokio::fs;

fn wordlists_base() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    #[cfg(target_os = "macos")]
    let base = home
        .join("Library")
        .join("Application Support")
        .join("golish-platform")
        .join("wordlists");
    #[cfg(target_os = "windows")]
    let base = home
        .join("AppData")
        .join("Local")
        .join("golish-platform")
        .join("wordlists");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = home.join(".golish-platform").join("wordlists");
    base
}

fn meta_path() -> PathBuf {
    wordlists_base().join("_meta.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordlistMeta {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub line_count: u64,
    pub file_size: u64,
    pub filename: String,
    pub tags: Vec<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MetaStore {
    wordlists: Vec<WordlistMeta>,
}

async fn load_meta() -> MetaStore {
    let p = meta_path();
    if let Ok(data) = fs::read_to_string(&p).await {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        MetaStore::default()
    }
}

async fn save_meta(store: &MetaStore) -> Result<(), String> {
    let base = wordlists_base();
    fs::create_dir_all(&base).await.map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(meta_path(), json).await.map_err(|e| e.to_string())
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub async fn wordlist_list() -> Result<Vec<WordlistMeta>, String> {
    Ok(load_meta().await.wordlists)
}

#[tauri::command]
pub async fn wordlist_import(
    name: String,
    category: String,
    description: String,
    content_base64: String,
    original_filename: String,
    tags: Option<Vec<String>>,
) -> Result<String, String> {
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &content_base64,
    )
    .map_err(|e| format!("Base64 decode error: {e}"))?;

    let text = String::from_utf8_lossy(&bytes);
    let line_count = text.lines().count() as u64;

    let id = uuid::Uuid::new_v4().to_string();
    let ext = original_filename
        .rsplit('.')
        .next()
        .unwrap_or("txt");
    let filename = format!("{id}.{ext}");

    let base = wordlists_base();
    fs::create_dir_all(&base).await.map_err(|e| e.to_string())?;
    fs::write(base.join(&filename), &bytes)
        .await
        .map_err(|e| e.to_string())?;

    let meta = WordlistMeta {
        id: id.clone(),
        name,
        category,
        description,
        line_count,
        file_size: bytes.len() as u64,
        filename,
        tags: tags.unwrap_or_default(),
        created_at: now_ts(),
    };

    let mut store = load_meta().await;
    store.wordlists.push(meta);
    save_meta(&store).await?;

    Ok(id)
}

#[tauri::command]
pub async fn wordlist_delete(id: String) -> Result<(), String> {
    let mut store = load_meta().await;
    if let Some(pos) = store.wordlists.iter().position(|w| w.id == id) {
        let wl = store.wordlists.remove(pos);
        let file_path = wordlists_base().join(&wl.filename);
        let _ = fs::remove_file(&file_path).await;
        save_meta(&store).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn wordlist_deduplicate(id: String) -> Result<WordlistMeta, String> {
    let mut store = load_meta().await;
    let wl = store
        .wordlists
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or("Wordlist not found")?;

    let file_path = wordlists_base().join(&wl.filename);
    let content = fs::read_to_string(&file_path)
        .await
        .map_err(|e| e.to_string())?;

    let mut seen = HashSet::new();
    let deduped: Vec<&str> = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && seen.insert(trimmed.to_string())
        })
        .collect();

    let new_count = deduped.len() as u64;
    let new_content = deduped.join("\n") + "\n";
    fs::write(&file_path, &new_content)
        .await
        .map_err(|e| e.to_string())?;

    wl.line_count = new_count;
    wl.file_size = new_content.len() as u64;
    let result = wl.clone();
    save_meta(&store).await?;

    Ok(result)
}

#[tauri::command]
pub async fn wordlist_merge(ids: Vec<String>, new_name: String, deduplicate: bool) -> Result<String, String> {
    let store = load_meta().await;
    let base = wordlists_base();

    let mut all_lines = BTreeSet::new();
    let mut ordered_lines: Vec<String> = Vec::new();

    for id in &ids {
        if let Some(wl) = store.wordlists.iter().find(|w| &w.id == id) {
            let file_path = base.join(&wl.filename);
            if let Ok(content) = fs::read_to_string(&file_path).await {
                for line in content.lines() {
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if deduplicate {
                        if all_lines.insert(trimmed.clone()) {
                            ordered_lines.push(trimmed);
                        }
                    } else {
                        ordered_lines.push(trimmed);
                    }
                }
            }
        }
    }

    let merged_content = ordered_lines.join("\n") + "\n";
    let new_id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{new_id}.txt");

    fs::write(base.join(&filename), &merged_content)
        .await
        .map_err(|e| e.to_string())?;

    let meta = WordlistMeta {
        id: new_id.clone(),
        name: new_name,
        category: "merged".to_string(),
        description: format!("Merged from {} wordlists", ids.len()),
        line_count: ordered_lines.len() as u64,
        file_size: merged_content.len() as u64,
        filename,
        tags: vec!["merged".to_string()],
        created_at: now_ts(),
    };

    let mut store = load_meta().await;
    store.wordlists.push(meta);
    save_meta(&store).await?;

    Ok(new_id)
}

#[tauri::command]
pub async fn wordlist_preview(id: String, lines: Option<u32>) -> Result<Vec<String>, String> {
    let store = load_meta().await;
    let wl = store
        .wordlists
        .iter()
        .find(|w| w.id == id)
        .ok_or("Wordlist not found")?;

    let file_path = wordlists_base().join(&wl.filename);
    let content = fs::read_to_string(&file_path)
        .await
        .map_err(|e| e.to_string())?;

    let limit = lines.unwrap_or(50) as usize;
    let preview: Vec<String> = content.lines().take(limit).map(|s| s.to_string()).collect();
    Ok(preview)
}

#[tauri::command]
pub async fn wordlist_path(id: String) -> Result<String, String> {
    let store = load_meta().await;
    let wl = store
        .wordlists
        .iter()
        .find(|w| w.id == id)
        .ok_or("Wordlist not found")?;

    let file_path = wordlists_base().join(&wl.filename);
    Ok(file_path.to_string_lossy().to_string())
}
