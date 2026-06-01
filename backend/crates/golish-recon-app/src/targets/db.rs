//! Plain database helpers (no Tauri annotations) for targets/recon writes.
//!
//! The SQL now lives in `golish_db::repo::targets` (sunk there in S1-3 so the
//! cross-service `ReconTargetsPort` adapter can back pentest/agent without a
//! sibling-crate dependency); these are thin recon-side wrappers over it,
//! preserving the prior signatures/semantics. Currently caller-free inside the
//! workspace — the pentest AI tools that used them moved to the port — but kept
//! as recon's data-layer API.

use golish_app_core::GolishError;
use sqlx::PgPool;
use uuid::Uuid;

use super::recon::ReconUpdate;
use super::types::{detect_type, Target, TargetRow, TargetType};

#[allow(clippy::too_many_arguments)]
pub async fn db_target_add(
    pool: &PgPool,
    name: &str,
    value: &str,
    target_type: Option<&str>,
    grp: Option<&str>,
    owner: Option<&str>,
    time_window_start: Option<chrono::DateTime<chrono::Utc>>,
    time_window_end: Option<chrono::DateTime<chrono::Utc>>,
    organization_id: Option<Uuid>,
    project_path: Option<&str>,
    source: &str,
    parent_id: Option<Uuid>,
) -> Result<Target, GolishError> {
    let tt = target_type
        .map(TargetType::from_str)
        .unwrap_or_else(|| detect_type(value));
    let n = if name.is_empty() { value } else { name };
    let g = grp
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let own = owner.map(str::trim).unwrap_or("");

    if let Some(r) =
        golish_db::repo::targets::find_row_by_value_legacy::<TargetRow>(pool, value, project_path)
            .await?
    {
        return Ok(Target::from(r));
    }

    let row: TargetRow = golish_db::repo::targets::insert_full(
        pool,
        n,
        tt.as_str(),
        value,
        g,
        own,
        time_window_start,
        time_window_end,
        organization_id,
        project_path,
        source,
        parent_id,
    )
    .await?;
    Ok(Target::from(row))
}

pub async fn db_target_list(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<Target>, GolishError> {
    let rows: Vec<TargetRow> =
        golish_db::repo::targets::list_rows_by_project_exact(pool, project_path).await?;
    Ok(rows.into_iter().map(Target::from).collect())
}

pub async fn db_target_update_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
) -> Result<(), GolishError> {
    golish_db::repo::targets::update_status_by_id(pool, id, status).await?;
    Ok(())
}

pub async fn db_target_update_recon(
    pool: &PgPool,
    id: Uuid,
    ports: &serde_json::Value,
) -> Result<(), GolishError> {
    golish_db::repo::targets::update_ports_by_id(pool, id, ports).await?;
    Ok(())
}

/// Extended recon update accepting all httpx/nmap-derived fields.
/// Only non-empty values overwrite existing data.
pub async fn db_target_update_recon_extended(
    pool: &PgPool,
    id: Uuid,
    updates: &ReconUpdate,
) -> Result<(), GolishError> {
    golish_db::repo::targets::update_recon_extended_by_id(
        pool,
        id,
        &updates.real_ip,
        &updates.cdn_waf,
        &updates.http_title,
        updates.http_status,
        &updates.webserver,
        &updates.os_info,
        &updates.content_type,
        &updates.ports,
    )
    .await?;
    Ok(())
}
