use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::db::open_db;

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

impl TargetType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::Ip => "ip",
            Self::Cidr => "cidr",
            Self::Url => "url",
            Self::Wildcard => "wildcard",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "ip" => Self::Ip,
            "cidr" => Self::Cidr,
            "url" => Self::Url,
            "wildcard" => Self::Wildcard,
            _ => Self::Domain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[serde(rename = "in")]
    InScope,
    #[serde(rename = "out")]
    OutOfScope,
}

impl Scope {
    fn as_str(&self) -> &'static str {
        match self {
            Self::InScope => "in",
            Self::OutOfScope => "out",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "out" => Self::OutOfScope,
            _ => Self::InScope,
        }
    }
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

fn row_to_target(row: &rusqlite::Row) -> rusqlite::Result<Target> {
    let tags_json: String = row.get(4)?;
    let tt_str: String = row.get(2)?;
    let scope_str: String = row.get(6)?;
    Ok(Target {
        id: row.get(0)?,
        name: row.get(1)?,
        target_type: TargetType::from_str(&tt_str),
        value: row.get(3)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        notes: row.get(5)?,
        scope: Scope::from_str(&scope_str),
        group: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[tauri::command]
pub async fn target_list(project_path: Option<String>) -> Result<TargetStore, String> {
    let pp = project_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open_db(pp.as_deref())?;
        let mut stmt = conn
            .prepare("SELECT id, name, target_type, value, tags, notes, scope, grp, created_at, updated_at FROM targets ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let targets: Vec<Target> = stmt
            .query_map([], |row| row_to_target(row))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let groups: Vec<String> = {
            let mut grp_stmt = conn
                .prepare("SELECT name FROM target_groups ORDER BY name")
                .map_err(|e| e.to_string())?;
            let g: Vec<String> = grp_stmt
                .query_map([], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            g
        };

        Ok(TargetStore {
            targets,
            groups: if groups.is_empty() { vec!["default".to_string()] } else { groups },
        })
    })
    .await
    .map_err(|e| e.to_string())?
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
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
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
        let tags_json = serde_json::to_string(&target.tags).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO targets (id, name, target_type, value, tags, notes, scope, grp, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![target.id, target.name, target.target_type.as_str(), target.value, tags_json, target.notes, target.scope.as_str(), target.group, target.created_at, target.updated_at],
        ).map_err(|e| e.to_string())?;
        Ok(target)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_batch_add(
    values: String,
    group: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<Target>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();
        let grp = group.unwrap_or_else(|| "default".to_string());
        let mut added = Vec::new();

        let mut stmt = conn.prepare("SELECT value FROM targets").map_err(|e| e.to_string())?;
        let existing: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        for line in values.lines() {
            let v = line.trim();
            if v.is_empty() || v.starts_with('#') {
                continue;
            }
            if existing.iter().any(|e| e == v) {
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
            conn.execute(
                "INSERT INTO targets (id, name, target_type, value, tags, notes, scope, grp, created_at, updated_at) VALUES (?1,?2,?3,?4,'[]','','in',?5,?6,?7)",
                params![target.id, target.name, target.target_type.as_str(), target.value, target.group, ts, ts],
            ).map_err(|e| e.to_string())?;
            added.push(target);
        }
        Ok(added)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_update(
    id: String,
    name: Option<String>,
    scope: Option<Scope>,
    group: Option<String>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    project_path: Option<String>,
) -> Result<Target, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        let ts = now_ts();

        if let Some(n) = &name {
            conn.execute("UPDATE targets SET name=?1, updated_at=?2 WHERE id=?3", params![n, ts, id])
                .map_err(|e| e.to_string())?;
        }
        if let Some(s) = &scope {
            conn.execute("UPDATE targets SET scope=?1, updated_at=?2 WHERE id=?3", params![s.as_str(), ts, id])
                .map_err(|e| e.to_string())?;
        }
        if let Some(g) = &group {
            conn.execute("UPDATE targets SET grp=?1, updated_at=?2 WHERE id=?3", params![g, ts, id])
                .map_err(|e| e.to_string())?;
        }
        if let Some(t) = &tags {
            let j = serde_json::to_string(t).unwrap_or_else(|_| "[]".to_string());
            conn.execute("UPDATE targets SET tags=?1, updated_at=?2 WHERE id=?3", params![j, ts, id])
                .map_err(|e| e.to_string())?;
        }
        if let Some(n) = &notes {
            conn.execute("UPDATE targets SET notes=?1, updated_at=?2 WHERE id=?3", params![n, ts, id])
                .map_err(|e| e.to_string())?;
        }

        let mut stmt = conn
            .prepare("SELECT id, name, target_type, value, tags, notes, scope, grp, created_at, updated_at FROM targets WHERE id=?1")
            .map_err(|e| e.to_string())?;
        stmt.query_row(params![id], |row| row_to_target(row))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_delete(id: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM targets WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_add_group(name: String, project_path: Option<String>) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("INSERT OR IGNORE INTO target_groups (name) VALUES (?1)", params![name])
            .map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT name FROM target_groups ORDER BY name").map_err(|e| e.to_string())?;
        let groups: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        Ok(groups)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_delete_group(name: String, project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM target_groups WHERE name=?1", params![name])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM targets WHERE grp=?1", params![name])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn target_clear_all(project_path: Option<String>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let conn = open_db(project_path.as_deref())?;
        conn.execute("DELETE FROM targets", []).map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM target_groups", []).map_err(|e| e.to_string())?;
        conn.execute("INSERT OR IGNORE INTO target_groups (name) VALUES ('default')", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
