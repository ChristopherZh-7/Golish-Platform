use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

fn resolve_vault_path(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("vault.json");
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
    base.join("vault.json")
}

fn derive_key() -> Vec<u8> {
    let seed = format!(
        "golish-vault-{}",
        dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default()
    );
    let mut key = Vec::with_capacity(32);
    let bytes = seed.as_bytes();
    for i in 0..32 {
        key.push(bytes[i % bytes.len()].wrapping_add(i as u8).wrapping_mul(7));
    }
    key
}

fn obfuscate(plain: &str) -> String {
    let key = derive_key();
    let encrypted: Vec<u8> = plain
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    B64.encode(&encrypted)
}

fn deobfuscate(encoded: &str) -> Result<String, String> {
    let key = derive_key();
    let data = B64.decode(encoded).map_err(|e| e.to_string())?;
    let plain: Vec<u8> = data
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    String::from_utf8(plain).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntry {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: VaultEntryType,
    pub value: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEntrySafe {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: VaultEntryType,
    pub username: String,
    pub notes: String,
    pub project: String,
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VaultEntryType {
    Password,
    Token,
    #[serde(rename = "ssh_key")]
    SshKey,
    #[serde(rename = "api_key")]
    ApiKey,
    Cookie,
    Certificate,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultStore {
    pub entries: Vec<VaultEntry>,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn load_store(project_path: Option<&str>) -> Result<VaultStore, String> {
    let path = resolve_vault_path(project_path);
    if !path.exists() {
        return Ok(VaultStore::default());
    }
    let data = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

async fn save_store(store: &VaultStore, project_path: Option<&str>) -> Result<(), String> {
    let path = resolve_vault_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, data).await.map_err(|e| e.to_string())
}

impl VaultEntry {
    fn to_safe(&self) -> VaultEntrySafe {
        VaultEntrySafe {
            id: self.id.clone(),
            name: self.name.clone(),
            entry_type: self.entry_type.clone(),
            username: self.username.clone(),
            notes: self.notes.clone(),
            project: self.project.clone(),
            tags: self.tags.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[tauri::command]
pub async fn vault_list(project_path: Option<String>) -> Result<Vec<VaultEntrySafe>, String> {
    let store = load_store(project_path.as_deref()).await?;
    Ok(store.entries.iter().map(|e| e.to_safe()).collect())
}

#[tauri::command]
pub async fn vault_add(
    name: String,
    entry_type: VaultEntryType,
    value: String,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    let ts = now_ts();
    let entry = VaultEntry {
        id: Uuid::new_v4().to_string()[..8].to_string(),
        name,
        entry_type,
        value: obfuscate(&value),
        username: username.unwrap_or_default(),
        notes: notes.unwrap_or_default(),
        project: project.unwrap_or_default(),
        tags: tags.unwrap_or_default(),
        created_at: ts,
        updated_at: ts,
    };
    let safe = entry.to_safe();
    store.entries.push(entry);
    save_store(&store, pp).await?;
    Ok(safe)
}

#[tauri::command]
pub async fn vault_get_value(id: String, project_path: Option<String>) -> Result<String, String> {
    let store = load_store(project_path.as_deref()).await?;
    let entry = store.entries.iter().find(|e| e.id == id).ok_or("Entry not found")?;
    deobfuscate(&entry.value)
}

#[tauri::command]
pub async fn vault_update(
    id: String,
    name: Option<String>,
    value: Option<String>,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    let entry = store.entries.iter_mut().find(|e| e.id == id).ok_or("Entry not found")?;
    if let Some(n) = name { entry.name = n; }
    if let Some(v) = value { entry.value = obfuscate(&v); }
    if let Some(u) = username { entry.username = u; }
    if let Some(n) = notes { entry.notes = n; }
    if let Some(p) = project { entry.project = p; }
    if let Some(t) = tags { entry.tags = t; }
    entry.updated_at = now_ts();
    let safe = entry.to_safe();
    save_store(&store, pp).await?;
    Ok(safe)
}

#[tauri::command]
pub async fn vault_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    store.entries.retain(|e| e.id != id);
    save_store(&store, pp).await
}

#[tauri::command]
pub async fn vault_resolve(reference: String, project_path: Option<String>) -> Result<String, String> {
    let name = reference
        .trim_start_matches("{{vault:")
        .trim_end_matches("}}");
    let store = load_store(project_path.as_deref()).await?;
    let entry = store
        .entries
        .iter()
        .find(|e| e.name == name || e.id == name)
        .ok_or_else(|| format!("Vault entry '{}' not found", name))?;
    deobfuscate(&entry.value)
}
