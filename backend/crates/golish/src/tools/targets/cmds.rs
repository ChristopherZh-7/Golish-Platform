//! `#[tauri::command]` entry points for managing targets from the GUI.

use crate::error::GolishError;
use uuid::Uuid;

use crate::state::DbState;

use super::types::{
    detect_type, parse_iso8601, Scope, Target, TargetRow, TargetStatus, TargetStore, TargetType,
};

#[tauri::command]
pub async fn target_list(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<TargetStore, GolishError> {
    let pool = state.pool_ready().await?;

    let pp = project_path.as_deref().filter(|s| !s.is_empty());
    let rows = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports,
                  real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                  created_at, updated_at
           FROM targets WHERE ($1 IS NULL OR project_path = $1 OR project_path = '')
           ORDER BY created_at"#,
    )
    .bind(pp)
    .fetch_all(pool)
    .await?;

    let targets: Vec<Target> = rows.into_iter().map(Target::from).collect();

    Ok(TargetStore { targets })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn target_add(
    state: tauri::State<'_, DbState>,
    name: String,
    value: String,
    target_type: Option<TargetType>,
    scope: Option<Scope>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    grp: Option<String>,
    owner: Option<String>,
    time_window_start: Option<String>,
    time_window_end: Option<String>,
    organization_id: Option<String>,
    project_path: Option<String>,
    source: Option<String>,
    parent_id: Option<String>,
) -> Result<Target, GolishError> {
    let pool = state.pool_ready().await?;
    let tt = target_type.unwrap_or_else(|| detect_type(&value));
    let sc = scope.unwrap_or(Scope::InScope);
    let tags_json = serde_json::to_value(tags.unwrap_or_default()).unwrap_or_default();
    let n = if name.is_empty() { value.clone() } else { name };
    let nt = notes.unwrap_or_default();
    let g = grp
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let own = owner.unwrap_or_default();
    let tw_start = parse_iso8601(time_window_start.as_deref());
    let tw_end = parse_iso8601(time_window_end.as_deref());
    let src = source.unwrap_or_else(|| "manual".to_string());
    let pid: Option<Uuid> = parent_id.and_then(|s| s.parse().ok());
    let org_id: Option<Uuid> = organization_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse())
        .transpose()
        .map_err(|e: uuid::Error| e.to_string())?;

    let row = sqlx::query_as::<_, TargetRow>(
        r#"INSERT INTO targets (name, target_type, value, tags, notes, scope, grp, owner, time_window_start, time_window_end, organization_id, project_path, source, parent_id)
           VALUES ($1, $2::target_type, $3, $4, $5, $6::scope_type, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                     status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at"#,
    )
    .bind(&n)
    .bind(tt.as_str())
    .bind(&value)
    .bind(&tags_json)
    .bind(&nt)
    .bind(sc.as_str())
    .bind(&g)
    .bind(&own)
    .bind(tw_start)
    .bind(tw_end)
    .bind(org_id)
    .bind(project_path.as_deref())
    .bind(&src)
    .bind(pid)
    .fetch_one(pool)
    .await
?;

    Ok(Target::from(row))
}

#[tauri::command]
pub async fn target_batch_add(
    state: tauri::State<'_, DbState>,
    values: String,
    grp: Option<String>,
    project_path: Option<String>,
) -> Result<Vec<Target>, GolishError> {
    let pool = state.pool_ready().await?;

    let g = grp
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let existing: Vec<String> =
        sqlx::query_scalar("SELECT value FROM targets WHERE project_path = $1")
            .bind(project_path.as_deref())
            .fetch_all(pool)
            .await?;

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
               VALUES ($1, $2::target_type, $3, '[]', 'in'::scope_type, $4, $5)
               RETURNING id, name, target_type::text, value, tags, notes, scope::text,
                         status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at"#,
        )
        .bind(v)
        .bind(tt.as_str())
        .bind(v)
        .bind(&g)
        .bind(project_path.as_deref())
        .fetch_one(pool)
        .await?;
        added.push(Target::from(row));
    }
    Ok(added)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn target_update(
    state: tauri::State<'_, DbState>,
    id: String,
    name: Option<String>,
    scope: Option<Scope>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
    status: Option<TargetStatus>,
    ports: Option<Vec<serde_json::Value>>,
    grp: Option<String>,
    owner: Option<String>,
    time_window_start: Option<String>,
    time_window_end: Option<String>,
    organization_id: Option<String>,
    project_path: Option<String>,
) -> Result<Target, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    if let Some(n) = &name {
        sqlx::query("UPDATE targets SET name=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(s) = &scope {
        sqlx::query("UPDATE targets SET scope=$1::scope_type, updated_at=NOW() WHERE id=$2")
            .bind(s.as_str())
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(t) = &tags {
        let j = serde_json::to_value(t).unwrap_or_default();
        sqlx::query("UPDATE targets SET tags=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(n) = &notes {
        sqlx::query("UPDATE targets SET notes=$1, updated_at=NOW() WHERE id=$2")
            .bind(n)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(st) = &status {
        sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
            .bind(st.as_str())
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(p) = &ports {
        let j = serde_json::to_value(p).unwrap_or_default();
        sqlx::query("UPDATE targets SET ports=$1, updated_at=NOW() WHERE id=$2")
            .bind(&j)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(g) = &grp {
        let trimmed = g.trim();
        let val = if trimmed.is_empty() { "default" } else { trimmed };
        sqlx::query("UPDATE targets SET grp=$1, updated_at=NOW() WHERE id=$2")
            .bind(val)
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if let Some(o) = &owner {
        sqlx::query("UPDATE targets SET owner=$1, updated_at=NOW() WHERE id=$2")
            .bind(o.trim())
            .bind(uid)
            .execute(pool)
            .await?;
    }
    if time_window_start.is_some() || time_window_end.is_some() {
        let tw_start = parse_iso8601(time_window_start.as_deref());
        let tw_end = parse_iso8601(time_window_end.as_deref());
        sqlx::query(
            "UPDATE targets SET time_window_start=$1, time_window_end=$2, updated_at=NOW() WHERE id=$3",
        )
        .bind(tw_start)
        .bind(tw_end)
        .bind(uid)
        .execute(pool)
        .await?;
    }
    if let Some(org_id_str) = &organization_id {
        let org_id: Option<Uuid> = if org_id_str.trim().is_empty() {
            None
        } else {
            Some(
                org_id_str
                    .parse()
                    .map_err(|e: uuid::Error| e.to_string())?,
            )
        };
        sqlx::query("UPDATE targets SET organization_id=$1, updated_at=NOW() WHERE id=$2")
            .bind(org_id)
            .bind(uid)
            .execute(pool)
            .await?;
    }

    let row = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at
           FROM targets WHERE id=$1"#,
    )
    .bind(uid)
    .fetch_one(pool)
    .await?;

    Ok(Target::from(row))
}

#[tauri::command]
pub async fn target_delete(
    state: tauri::State<'_, DbState>,
    id: String,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;
    sqlx::query("DELETE FROM targets WHERE id=$1")
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn target_clear_all(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<(), GolishError> {
    let pool = state.pool_ready().await?;
    sqlx::query("DELETE FROM targets WHERE project_path = $1")
        .bind(project_path.as_deref())
        .execute(pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn target_update_status(
    state: tauri::State<'_, DbState>,
    id: String,
    status: TargetStatus,
    project_path: Option<String>,
) -> Result<Target, GolishError> {
    let pool = state.pool_ready().await?;
    let _ = project_path;
    let uid: Uuid = id.parse().map_err(|e: uuid::Error| e.to_string())?;

    sqlx::query("UPDATE targets SET status=$1::target_status, updated_at=NOW() WHERE id=$2")
        .bind(status.as_str())
        .bind(uid)
        .execute(pool)
        .await?;

    let row = sqlx::query_as::<_, TargetRow>(
        r#"SELECT id, name, target_type::text, value, tags, notes, scope::text,
                  status::text, grp, owner, time_window_start, time_window_end, organization_id, source, parent_id, ports,
                     real_ip, cdn_waf, http_title, http_status, webserver, os_info, content_type,
                     created_at, updated_at
           FROM targets WHERE id=$1"#,
    )
    .bind(uid)
    .fetch_one(pool)
    .await?;

    Ok(Target::from(row))
}
