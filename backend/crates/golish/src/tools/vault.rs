use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

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

pub fn obfuscate_value(plain: &str) -> String {
    obfuscate(plain)
}

pub fn deobfuscate_value(encoded: &str) -> Result<String, String> {
    deobfuscate(encoded)
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
    pub status: String,
    pub source_url: String,
    pub last_validated_at: Option<u64>,
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

fn ts_from_dt(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
struct VaultRow {
    id: Uuid,
    name: String,
    entry_type: String,
    username: String,
    notes: String,
    project: String,
    tags: serde_json::Value,
    status: String,
    source_url: String,
    last_validated_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<VaultRow> for VaultEntrySafe {
    fn from(r: VaultRow) -> Self {
        Self {
            id: r.id.to_string(),
            name: r.name,
            entry_type: VaultEntryType::from_str(&r.entry_type),
            username: r.username,
            notes: r.notes,
            project: r.project,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            status: r.status,
            source_url: r.source_url,
            last_validated_at: r.last_validated_at.map(|dt| ts_from_dt(dt)),
            created_at: ts_from_dt(r.created_at),
            updated_at: ts_from_dt(r.updated_at),
        }
    }
}

#[tauri::command]
pub async fn vault_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<Vec<VaultEntrySafe>, String> {
    let pool = state.db_pool_ready().await?;
    let rows: Vec<VaultRow> = sqlx::query_as(
        "SELECT id, name, entry_type::TEXT, username, notes, project, tags, status, source_url, last_validated_at, created_at, updated_at \
         FROM vault_entries WHERE project_path IS NOT DISTINCT FROM $1 OR project_path IS NULL ORDER BY created_at DESC",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(VaultEntrySafe::from).collect())
}

#[tauri::command]
pub async fn vault_add(
    state: tauri::State<'_, AppState>,
    name: String,
    entry_type: VaultEntryType,
    value: String,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    source_url: Option<String>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, String> {
    let pool = state.db_pool_ready().await?;
    let ts = now_ts();
    let id = Uuid::new_v4();
    let short_id = id.to_string()[..8].to_string();
    let un = username.unwrap_or_default();
    let nt = notes.unwrap_or_default();
    let pj = project.unwrap_or_default();
    let tg = tags.unwrap_or_default();
    let su = source_url.unwrap_or_default();
    let tags_json = serde_json::to_value(&tg).unwrap_or_else(|_| serde_json::json!([]));
    let enc_value = obfuscate(&value);

    sqlx::query(
        r#"INSERT INTO vault_entries (id, name, entry_type, value, username, notes, project, tags, source_url, project_path)
           VALUES ($1, $2, $3::vault_entry_type, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(id)
    .bind(&name)
    .bind(entry_type.as_str())
    .bind(&enc_value)
    .bind(&un)
    .bind(&nt)
    .bind(&pj)
    .bind(&tags_json)
    .bind(&su)
    .bind(&project_path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(VaultEntrySafe {
        id: short_id,
        name,
        entry_type,
        username: un,
        notes: nt,
        project: pj,
        tags: tg,
        status: "unknown".to_string(),
        source_url: su,
        last_validated_at: None,
        created_at: ts,
        updated_at: ts,
    })
}

#[tauri::command]
pub async fn vault_get_value(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    let enc: String = sqlx::query_scalar("SELECT value FROM vault_entries WHERE id = $1")
        .bind(uid)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    deobfuscate(&enc)
}

#[tauri::command]
pub async fn vault_update(
    state: tauri::State<'_, AppState>,
    id: String,
    name: Option<String>,
    value: Option<String>,
    username: Option<String>,
    notes: Option<String>,
    project: Option<String>,
    tags: Option<Vec<String>>,
    project_path: Option<String>,
) -> Result<VaultEntrySafe, String> {
    let pool = state.db_pool_ready().await?;
    let _ = &project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    if let Some(n) = &name {
        sqlx::query("UPDATE vault_entries SET name=$1, updated_at=NOW() WHERE id=$2")
            .bind(n).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if let Some(v) = &value {
        sqlx::query("UPDATE vault_entries SET value=$1, updated_at=NOW() WHERE id=$2")
            .bind(obfuscate(v)).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if let Some(u) = &username {
        sqlx::query("UPDATE vault_entries SET username=$1, updated_at=NOW() WHERE id=$2")
            .bind(u).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if let Some(n) = &notes {
        sqlx::query("UPDATE vault_entries SET notes=$1, updated_at=NOW() WHERE id=$2")
            .bind(n).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if let Some(p) = &project {
        sqlx::query("UPDATE vault_entries SET project=$1, updated_at=NOW() WHERE id=$2")
            .bind(p).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }
    if let Some(t) = &tags {
        let j = serde_json::to_value(t).unwrap_or_else(|_| serde_json::json!([]));
        sqlx::query("UPDATE vault_entries SET tags=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j).bind(uid).execute(pool).await.map_err(|e| e.to_string())?;
    }

    let row: VaultRow = sqlx::query_as(
        "SELECT id, name, entry_type::TEXT, username, notes, project, tags, status, source_url, last_validated_at, created_at, updated_at \
         FROM vault_entries WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(VaultEntrySafe::from(row))
}

#[tauri::command]
pub async fn vault_update_status(
    state: tauri::State<'_, AppState>,
    id: String,
    status: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let _ = &project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("UPDATE vault_entries SET status=$1, last_validated_at=NOW() WHERE id=$2")
        .bind(&status)
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vault_validate(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let _ = &project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let (enc_value, source_url, entry_type): (String, String, String) = sqlx::query_as(
        "SELECT value, source_url, entry_type::TEXT FROM vault_entries WHERE id=$1",
    )
    .bind(uid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    let value = deobfuscate(&enc_value)?;

    if source_url.is_empty() {
        return Err("No source URL to validate against".to_string());
    }

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(&source_url);
    match entry_type.as_str() {
        "token" => {
            if value.starts_with("Bearer ") {
                req = req.header("Authorization", &value);
            } else {
                req = req.header("Authorization", format!("Bearer {}", value));
            }
        }
        "api_key" => {
            req = req.header("X-API-Key", &value);
        }
        "cookie" => {
            req = req.header("Cookie", &value);
        }
        _ => {
            req = req.header("Authorization", format!("Bearer {}", value));
        }
    }

    let status = match req.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                "expired"
            } else if (200..400).contains(&code) {
                "valid"
            } else {
                "unknown"
            }
        }
        Err(_) => "unknown",
    };

    sqlx::query("UPDATE vault_entries SET status=$1, last_validated_at=NOW() WHERE id=$2")
        .bind(status)
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(status.to_string())
}

#[tauri::command]
pub async fn vault_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = state.db_pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM vault_entries WHERE id = $1")
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn vault_resolve(
    state: tauri::State<'_, AppState>,
    reference: String,
    project_path: Option<String>,
) -> Result<String, String> {
    let pool = state.db_pool_ready().await?;
    let name = reference.trim_start_matches("{{vault:").trim_end_matches("}}");
    let enc: String = if let Some(ref pp) = project_path {
        sqlx::query_scalar(
            "SELECT value FROM vault_entries WHERE (name=$1 OR id::TEXT=$1) AND project_path = $2",
        )
        .bind(name)
        .bind(pp)
        .fetch_one(pool)
        .await
    } else {
        sqlx::query_scalar(
            "SELECT value FROM vault_entries WHERE (name=$1 OR id::TEXT=$1) AND project_path IS NULL",
        )
        .bind(name)
        .fetch_one(pool)
        .await
    }
    .map_err(|_| format!("Vault entry '{}' not found", name))?;
    deobfuscate(&enc)
}
