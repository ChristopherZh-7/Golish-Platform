use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::db::open_db;

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

impl VaultEntryType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Token => "token",
            Self::SshKey => "ssh_key",
            Self::ApiKey => "api_key",
            Self::Cookie => "cookie",
            Self::Certificate => "certificate",
            Self::Other => "other",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "token" => Self::Token,
            "ssh_key" => Self::SshKey,
            "api_key" => Self::ApiKey,
            "cookie" => Self::Cookie,
            "certificate" => Self::Certificate,
            "other" => Self::Other,
            _ => Self::Password,
        }
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn row_to_safe(row: &rusqlite::Row) -> rusqlite::Result<VaultEntrySafe> {
    let et_str: String = row.get(2)?;
    let tags_json: String = row.get(6)?;
    Ok(VaultEntrySafe {
        id: row.get(0)?,
        name: row.get(1)?,
        entry_type: VaultEntryType::from_str(&et_str),
        username: row.get(3)?,
        notes: row.get(4)?,
        project: row.get(5)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[tauri::command]
pub async fn vault_list(project_path: Option<String>) -> Result<Vec<VaultEntrySafe>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let mut stmt = conn
            .prepare("SELECT id, name, entry_type, username, notes, project, tags, created_at, updated_at FROM vault_entries ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let entries: Vec<VaultEntrySafe> = stmt
            .query_map([], |row| row_to_safe(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(entries)
    })
    .await
    .map_err(|e| e.to_string())?
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
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let id = Uuid::new_v4().to_string()[..8].to_string();
        let un = username.unwrap_or_default();
        let nt = notes.unwrap_or_default();
        let pj = project.unwrap_or_default();
        let tg = tags.unwrap_or_default();
        let tags_json = serde_json::to_string(&tg).unwrap_or_else(|_| "[]".to_string());
        let enc_value = obfuscate(&value);
        conn.execute(
            "INSERT INTO vault_entries (id, name, entry_type, value, username, notes, project, tags, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![id, name, entry_type.as_str(), enc_value, un, nt, pj, tags_json, ts, ts],
        ).map_err(|e| e.to_string())?;
        Ok(VaultEntrySafe { id, name, entry_type, username: un, notes: nt, project: pj, tags: tg, created_at: ts, updated_at: ts })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn vault_get_value(id: String, project_path: Option<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let enc: String = conn
            .query_row("SELECT value FROM vault_entries WHERE id=?1", params![id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        deobfuscate(&enc)
    })
    .await
    .map_err(|e| e.to_string())?
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
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        if let Some(n) = &name { conn.execute("UPDATE vault_entries SET name=?1, updated_at=?2 WHERE id=?3", params![n, ts, id]).map_err(|e| e.to_string())?; }
        if let Some(v) = &value { conn.execute("UPDATE vault_entries SET value=?1, updated_at=?2 WHERE id=?3", params![obfuscate(v), ts, id]).map_err(|e| e.to_string())?; }
        if let Some(u) = &username { conn.execute("UPDATE vault_entries SET username=?1, updated_at=?2 WHERE id=?3", params![u, ts, id]).map_err(|e| e.to_string())?; }
        if let Some(n) = &notes { conn.execute("UPDATE vault_entries SET notes=?1, updated_at=?2 WHERE id=?3", params![n, ts, id]).map_err(|e| e.to_string())?; }
        if let Some(p) = &project { conn.execute("UPDATE vault_entries SET project=?1, updated_at=?2 WHERE id=?3", params![p, ts, id]).map_err(|e| e.to_string())?; }
        if let Some(t) = &tags {
            let j = serde_json::to_string(t).unwrap_or_else(|_| "[]".to_string());
            conn.execute("UPDATE vault_entries SET tags=?1, updated_at=?2 WHERE id=?3", params![j, ts, id]).map_err(|e| e.to_string())?;
        }
        let mut stmt = conn.prepare("SELECT id, name, entry_type, username, notes, project, tags, created_at, updated_at FROM vault_entries WHERE id=?1").map_err(|e| e.to_string())?;
        stmt.query_row(params![id], |row| row_to_safe(row)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn vault_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM vault_entries WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn vault_resolve(reference: String, project_path: Option<String>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let name = reference.trim_start_matches("{{vault:").trim_end_matches("}}");
        let conn = open_db(project_path.as_deref())?;
        let enc: String = conn
            .query_row(
                "SELECT value FROM vault_entries WHERE name=?1 OR id=?1",
                params![name],
                |r| r.get(0),
            )
            .map_err(|_| format!("Vault entry '{}' not found", name))?;
        deobfuscate(&enc)
    })
    .await
    .map_err(|e| e.to_string())?
}
