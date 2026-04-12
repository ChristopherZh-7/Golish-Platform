use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::AppState;

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
    pub status: TargetStatus,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub ports: Vec<serde_json::Value>,
    #[serde(default)]
    pub technologies: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TargetStatus {
    New,
    Recon,
    ReconDone,
    Scanning,
    Tested,
}

impl TargetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Recon => "recon",
            Self::ReconDone => "recon_done",
            Self::Scanning => "scanning",
            Self::Tested => "tested",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "recon" => Self::Recon,
            "recon_done" => Self::ReconDone,
            "scanning" => Self::Scanning,
            "tested" => Self::Tested,
            _ => Self::New,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetStore {
    pub targets: Vec<Target>,
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

fn ts_from_chrono(dt: chrono::DateTime<chrono::Utc>) -> u64 {
    dt.timestamp() as u64
}

#[derive(sqlx::FromRow)]
struct TargetRow {
    id: Uuid,
    name: String,
    target_type: String,
    value: String,
    tags: serde_json::Value,
    notes: String,
    scope: String,
    status: String,
    source: String,
    parent_id: Option<Uuid>,
    ports: serde_json::Value,
    technologies: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<TargetRow> for Target {
    fn from(r: TargetRow) -> Self {
        Target {
            id: r.id.to_string(),
            name: r.name,
            target_type: TargetType::from_str(&r.target_type),
            value: r.value,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            notes: r.notes,
            scope: Scope::from_str(&r.scope),
            status: TargetStatus::from_str(&r.status),
            source: r.source,
            parent_id: r.parent_id.map(|u| u.to_string()),
            ports: serde_json::from_value(r.ports).unwrap_or_default(),
            technologies: serde_json::from_value(r.technologies).unwrap_or_default(),
            created_at: ts_from_chrono(r.created_at),
            updated_at: ts_from_chrono(r.updated_at),
        }
    }
}

async fn pool_from_state(state: &AppState) -> &PgPool {
    &state.db_pool
}

#[tauri::command]
pub async fn target_list(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<TargetStore, String> {
    let pool = pool_from_state(&state).await;

    let rows = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports, technologies, created_at, updated_at
           FROM targets WHERE project_path IS NOT DISTINCT FROM $1
           ORDER BY created_at"#,
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let targets: Vec<Target> = rows.into_iter().map(Target::from).collect();

    Ok(TargetStore { targets })
}

#[tauri::command]
pub async fn target_add(
    state: tauri::State<'_, AppState>,
    name: String,
    value: String,
    target_type: Option<TargetType>,
    scope: Option<Scope>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    project_path: Option<String>,
    source: Option<String>,
    parent_id: Option<String>,
) -> Result<Target, String> {
    let pool = pool_from_state(&state).await;
    let tt = target_type.unwrap_or_else(|| detect_type(&value));
    let sc = scope.unwrap_or(Scope::InScope);
    let tags_json = serde_json::to_value(tags.unwrap_or_default()).unwrap_or_default();
    let n = if name.is_empty() { value.clone() } else { name };
    let nt = notes.unwrap_or_default();
    let src = source.unwrap_or_else(|| "manual".to_string());
    let pid: Option<Uuid> = parent_id.and_then(|s| s.parse().ok());

    let row = sqlx::query_as::<_, TargetRow>(
        r#"INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, project_path, source, parent_id)
           VALUES ($1, $2::target_type, $3, $4, $5, $6::scope_type, 'default', $7, $8, $9)
           RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                     status::text, source, parent_id, ports, technologies, created_at, updated_at"#,
    )
    .bind(&n)
    .bind(tt.as_str())
    .bind(&value)
    .bind(&tags_json)
    .bind(&nt)
    .bind(sc.as_str())
    .bind(project_path.as_deref())
    .bind(&src)
    .bind(pid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Target::from(row))
}

#[tauri::command]
pub async fn target_batch_add(
    state: tauri::State<'_, AppState>,
    values: String,
    project_path: Option<String>,
) -> Result<Vec<Target>, String> {
    let pool = pool_from_state(&state).await;

    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM targets WHERE project_path IS NOT DISTINCT FROM $1",
    )
    .bind(project_path.as_deref())
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut added = Vec::new();
    for line in values.lines() {
        let v = line.trim();
        if v.is_empty() || v.starts_with('#') {
            continue;
        }
        if existing.iter().any(|e| e == v) {
            continue;
        }
        let tt = detect_type(v);
        let row = sqlx::query_as::<_, TargetRow>(
            r#"INSERT INTO targets (name, target_type, value, tags, scope, grp, project_path)
               VALUES ($1, $2::target_type, $3, '[]', 'in'::scope_type, 'default', $4)
               RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                         status::text, source, parent_id, ports, technologies, created_at, updated_at"#,
        )
        .bind(v)
        .bind(tt.as_str())
        .bind(v)
        .bind(project_path.as_deref())
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
        added.push(Target::from(row));
    }
    Ok(added)
}

#[tauri::command]
pub async fn target_update(
    state: tauri::State<'_, AppState>,
    id: String,
    name: Option<String>,
    scope: Option<Scope>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    status: Option<TargetStatus>,
    ports: Option<Vec<serde_json::Value>>,
    technologies: Option<Vec<String>>,
    project_path: Option<String>,
) -> Result<Target, String> {
    let pool = pool_from_state(&state).await;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    if let Some(n) = &name {
        sqlx::query("UPDATE targets SET name=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(s) = &scope {
        sqlx::query("UPDATE targets SET scope=$1::scope_type, updated_at=NOW() WHERE id=$2")
            .bind(s.as_str())
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(t) = &tags {
        let j = serde_json::to_value(t).unwrap_or_default();
        sqlx::query("UPDATE targets SET tags=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(n) = &notes {
        sqlx::query("UPDATE targets SET notes=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(st) = &status {
        sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
            .bind(st.as_str())
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(p) = &ports {
        let j = serde_json::to_value(p).unwrap_or_default();
        sqlx::query("UPDATE targets SET ports=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(t) = &technologies {
        let j = serde_json::to_value(t).unwrap_or_default();
        sqlx::query("UPDATE targets SET technologies=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j)
            .bind(uid)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }

    let row = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports, technologies, created_at, updated_at
           FROM targets WHERE id=$1"#,
    )
    .bind(uid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Target::from(row))
}

#[tauri::command]
pub async fn target_delete(
    state: tauri::State<'_, AppState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = pool_from_state(&state).await;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM targets WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn target_clear_all(
    state: tauri::State<'_, AppState>,
    project_path: Option<String>,
) -> Result<(), String> {
    let pool = pool_from_state(&state).await;
    sqlx::query("DELETE FROM targets WHERE project_path IS NOT DISTINCT FROM $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn target_update_status(
    state: tauri::State<'_, AppState>,
    id: String,
    status: TargetStatus,
    project_path: Option<String>,
) -> Result<Target, String> {
    let pool = pool_from_state(&state).await;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
        .bind(status.as_str())
        .bind(uid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    let row = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports, technologies, created_at, updated_at
           FROM targets WHERE id=$1"#,
    )
    .bind(uid)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Target::from(row))
}

// ============================================================================
// Standalone DB functions for AI tool integration (no Tauri state needed)
// ============================================================================

pub async fn db_target_add(
    pool: &PgPool,
    name: &str,
    value: &str,
    target_type: Option<&str>,
    project_path: Option<&str>,
    source: &str,
    parent_id: Option<Uuid>,
) -> Result<Target, String> {
    let tt = target_type.map(TargetType::from_str).unwrap_or_else(|| detect_type(value));
    let n = if name.is_empty() { value } else { name };

    let row = sqlx::query_as::<_, TargetRow>(
        r#"INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, project_path, source, parent_id)
           VALUES ($1, $2::target_type, $3, '[]', '', 'in'::scope_type, 'default', $4, $5, $6)
           ON CONFLICT DO NOTHING
           RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                     status::text, source, parent_id, ports, technologies, created_at, updated_at"#,
    )
    .bind(n)
    .bind(tt.as_str())
    .bind(value)
    .bind(project_path)
    .bind(source)
    .bind(parent_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match row {
        Some(r) => Ok(Target::from(r)),
        None => {
            let existing = sqlx::query_as::<_, TargetRow>(
                r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                          status::text, source, parent_id, ports, technologies, created_at, updated_at
                   FROM targets WHERE value=$1 AND project_path IS NOT DISTINCT FROM $2 LIMIT 1"#,
            )
            .bind(value)
            .bind(project_path)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(Target::from(existing))
        }
    }
}

pub async fn db_target_list(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<Target>, String> {
    let rows = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, source, parent_id, ports, technologies, created_at, updated_at
           FROM targets WHERE project_path IS NOT DISTINCT FROM $1
           ORDER BY created_at"#,
    )
    .bind(project_path)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows.into_iter().map(Target::from).collect())
}

pub async fn db_target_update_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_target_update_recon(
    pool: &PgPool,
    id: Uuid,
    ports: &serde_json::Value,
    technologies: &serde_json::Value,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE targets SET ports=$1, technologies=$2, updated_at=NOW() WHERE id=$3",
    )
    .bind(ports)
    .bind(technologies)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
