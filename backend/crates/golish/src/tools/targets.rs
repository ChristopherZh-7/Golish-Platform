use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

fn resolve_targets_path(project_path: Option<&str>) -> PathBuf {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return PathBuf::from(pp).join(".golish").join("targets.json");
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
    base.join("targets.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: TargetType,
    pub value: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub scope: Scope,
    #[serde(default)]
    pub group: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Domain,
    Ip,
    Cidr,
    Url,
    Wildcard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[serde(rename = "in")]
    InScope,
    #[serde(rename = "out")]
    OutOfScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetStore {
    pub targets: Vec<Target>,
    #[serde(default)]
    pub groups: Vec<String>,
}

impl Default for TargetStore {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            groups: vec!["default".to_string()],
        }
    }
}

async fn load_store(project_path: Option<&str>) -> Result<TargetStore, String> {
    let path = resolve_targets_path(project_path);
    if !path.exists() {
        return Ok(TargetStore::default());
    }
    let data = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

async fn save_store(store: &TargetStore, project_path: Option<&str>) -> Result<(), String> {
    let path = resolve_targets_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, data).await.map_err(|e| e.to_string())
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn detect_type(value: &str) -> TargetType {
    let v = value.trim();
    if v.starts_with("http://") || v.starts_with("https://") {
        return TargetType::Url;
    }
    if v.contains('/') {
        return TargetType::Cidr;
    }
    if v.starts_with("*.") {
        return TargetType::Wildcard;
    }
    if v.parse::<std::net::IpAddr>().is_ok() {
        return TargetType::Ip;
    }
    TargetType::Domain
}

#[tauri::command]
pub async fn target_list(project_path: Option<String>) -> Result<TargetStore, String> {
    load_store(project_path.as_deref()).await
}

#[tauri::command]
pub async fn target_add(
    name: String,
    value: String,
    target_type: Option<TargetType>,
    scope: Option<Scope>,
    group: Option<String>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    project_path: Option<String>,
) -> Result<Target, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    let tt = target_type.unwrap_or_else(|| detect_type(&value));
    let ts = now_ts();
    let target = Target {
        id: Uuid::new_v4().to_string()[..8].to_string(),
        name: if name.is_empty() { value.clone() } else { name },
        target_type: tt,
        value,
        tags: tags.unwrap_or_default(),
        notes: notes.unwrap_or_default(),
        scope: scope.unwrap_or(Scope::InScope),
        group: group.unwrap_or_else(|| "default".to_string()),
        created_at: ts,
        updated_at: ts,
    };
    store.targets.push(target.clone());
    save_store(&store, pp).await?;
    Ok(target)
}

#[tauri::command]
pub async fn target_batch_add(values: String, group: Option<String>, project_path: Option<String>) -> Result<Vec<Target>, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    let ts = now_ts();
    let grp = group.unwrap_or_else(|| "default".to_string());
    let mut added = Vec::new();

    for line in values.lines() {
        let v = line.trim();
        if v.is_empty() || v.starts_with('#') {
            continue;
        }
        if store.targets.iter().any(|t| t.value == v) {
            continue;
        }
        let tt = detect_type(v);
        let target = Target {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name: v.to_string(),
            target_type: tt,
            value: v.to_string(),
            tags: Vec::new(),
            notes: String::new(),
            scope: Scope::InScope,
            group: grp.clone(),
            created_at: ts,
            updated_at: ts,
        };
        store.targets.push(target.clone());
        added.push(target);
    }

    save_store(&store, pp).await?;
    Ok(added)
}

#[tauri::command]
pub async fn target_update(id: String, name: Option<String>, scope: Option<Scope>, group: Option<String>, tags: Option<Vec<String>>, notes: Option<String>, project_path: Option<String>) -> Result<Target, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    let target = store.targets.iter_mut().find(|t| t.id == id).ok_or("Target not found")?;
    if let Some(n) = name { target.name = n; }
    if let Some(s) = scope { target.scope = s; }
    if let Some(g) = group { target.group = g; }
    if let Some(t) = tags { target.tags = t; }
    if let Some(n) = notes { target.notes = n; }
    target.updated_at = now_ts();
    let result = target.clone();
    save_store(&store, pp).await?;
    Ok(result)
}

#[tauri::command]
pub async fn target_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    store.targets.retain(|t| t.id != id);
    save_store(&store, pp).await
}

#[tauri::command]
pub async fn target_add_group(name: String, project_path: Option<String>) -> Result<Vec<String>, String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    if !store.groups.contains(&name) {
        store.groups.push(name);
    }
    let groups = store.groups.clone();
    save_store(&store, pp).await?;
    Ok(groups)
}

#[tauri::command]
pub async fn target_delete_group(name: String, project_path: Option<String>) -> Result<(), String> {
    let pp = project_path.as_deref();
    let mut store = load_store(pp).await?;
    store.groups.retain(|g| g != &name);
    store.targets.retain(|t| t.group != name);
    save_store(&store, pp).await
}

#[tauri::command]
pub async fn target_clear_all(project_path: Option<String>) -> Result<(), String> {
    let store = TargetStore::default();
    save_store(&store, project_path.as_deref()).await
}
